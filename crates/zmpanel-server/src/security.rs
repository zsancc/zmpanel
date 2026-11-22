use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::net::IpAddr;
use std::path::Path;
#[cfg(target_os = "linux")]
use std::process::Command;
use std::time::Duration;
use zmpanel_core::{AppError, SecurityBanItem, SecurityConfig, SshHardeningStatus};

use crate::firewall;
use crate::state::AppState;

#[derive(Debug, Clone)]
pub struct DetectionFinding {
    pub category: String,
    pub ip: String,
    pub hits: u32,
    pub reason: String,
    pub sample: String,
}

pub fn load_security_config(state: &AppState) -> SecurityConfig {
    state
        .db
        .get_setting("security_engine_config")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn spawn_security_engine(state: AppState) {
    tokio::spawn(async move {
        // Lightweight background loop: checks logs & expired bans every 20s (~0% CPU, <1MB RAM)
        loop {
            tokio::time::sleep(Duration::from_secs(20)).await;
            let st = state.clone();
            let _ = tokio::task::spawn_blocking(move || run_security_cycle(&st)).await;
        }
    });
}

pub fn run_security_cycle(state: &AppState) -> usize {
    let now = Utc::now().timestamp();
    let mut need_fw_sync = false;

    // 1. Unban expired IPs
    if let Ok(expired) = state.db.purge_expired_bans(now)
        && !expired.is_empty()
    {
        for ip in &expired {
            let _ = state.db.insert_security_event(
                "system",
                ip,
                "封禁期满自动解封",
                "unbanned",
                "",
            );
        }
        need_fw_sync = true;
    }

    let cfg = load_security_config(state);
    if !cfg.enabled {
        if need_fw_sync {
            resync_firewall(state);
        }
        return 0;
    }

    let mut findings = Vec::new();

    // 2. SSH Brute-Force Detection (Debian journald / Ubuntu auth.log / Alpine messages)
    if cfg.ssh_guard_enabled {
        let ssh_lines = read_ssh_log_tail(600);
        findings.extend(analyze_ssh_lines(&ssh_lines, cfg.ssh_max_failures));
    }

    // 3. Port Scan Detection (kernel firewall drop logs)
    if cfg.port_scan_enabled {
        let kern_lines = read_kernel_log_tail(600);
        findings.extend(analyze_port_scan_lines(&kern_lines, cfg.port_scan_threshold));
    }

    // 4. Web Scanner / Abnormal HTTP Detection (Nginx access log)
    if cfg.web_guard_enabled {
        let web_lines = read_tail_lines(Path::new(&cfg.nginx_log_path), 800);
        findings.extend(analyze_web_lines(&web_lines, cfg.web_scan_threshold));
    }

    let banned_count = apply_findings(state, &cfg, findings);
    if banned_count > 0 || need_fw_sync {
        resync_firewall(state);
    }
    banned_count
}

pub fn apply_findings(
    state: &AppState,
    cfg: &SecurityConfig,
    findings: Vec<DetectionFinding>,
) -> usize {
    let now = Utc::now().timestamp();
    let existing_ips: HashSet<String> = state
        .db
        .list_security_bans()
        .unwrap_or_default()
        .into_iter()
        .map(|b| b.ip)
        .collect();

    let mut newly_banned = 0;
    for f in findings {
        if is_local_or_private_safe(&f.ip) {
            continue;
        }
        if state.db.is_ip_whitelisted(&f.ip).unwrap_or(false) {
            continue;
        }
        if existing_ips.contains(&f.ip) {
            continue;
        }
        let expires_at = if cfg.ban_duration_secs > 0 {
            now + cfg.ban_duration_secs
        } else {
            0
        };
        let ban = SecurityBanItem {
            ip: f.ip.clone(),
            reason: f.reason.clone(),
            category: f.category.clone(),
            hits: f.hits,
            banned_at: now,
            expires_at,
        };
        if state.db.upsert_security_ban(&ban).is_ok() {
            let _ = state.db.insert_security_event(
                &f.category,
                &f.ip,
                &f.reason,
                "auto_banned",
                &f.sample,
            );
            newly_banned += 1;
        }
    }
    newly_banned
}

pub fn resync_firewall(state: &AppState) -> String {
    let rules = state.db.list_firewall_rules().unwrap_or_default();
    let bans: Vec<String> = state
        .db
        .list_security_bans()
        .unwrap_or_default()
        .into_iter()
        .map(|b| b.ip)
        .collect();
    firewall::sync_system_firewall(&rules, &bans)
}

/// Pure Rust SSH brute-force detector (zero regex crate overhead).
pub fn analyze_ssh_lines(lines: &[String], threshold: u32) -> Vec<DetectionFinding> {
    let mut counts: HashMap<String, (u32, String)> = HashMap::new();
    for line in lines {
        let is_fail = line.contains("Failed password")
            || line.contains("Invalid user")
            || line.contains("Failed publickey")
            || line.contains("Connection closed by authenticating user")
            || line.contains("maximum authentication attempts exceeded");
        if !is_fail {
            continue;
        }
        if let Some(ip) = extract_ip_after_keyword(line, "from ")
            .or_else(|| extract_ip_after_keyword(line, "user "))
            .or_else(|| extract_first_ip(line))
        {
            let entry = counts.entry(ip).or_insert((0, line.clone()));
            entry.0 += 1;
            entry.1 = line.clone();
        }
    }

    let min_hits = threshold.max(1);
    counts
        .into_iter()
        .filter(|(_, (cnt, _))| *cnt >= min_hits)
        .map(|(ip, (hits, sample))| DetectionFinding {
            category: "ssh".to_string(),
            ip,
            hits,
            reason: format!("SSH 爆破告警：检测到 {} 次认证失败尝试", hits),
            sample: sample.chars().take(240).collect(),
        })
        .collect()
}

/// Pure Rust port-scan detector parsing kernel netfilter `SRC=<ip> ... DPT=<port>` drop logs.
pub fn analyze_port_scan_lines(lines: &[String], threshold: u32) -> Vec<DetectionFinding> {
    let mut ports_by_ip: HashMap<String, (HashSet<u16>, String)> = HashMap::new();
    for line in lines {
        if !line.contains("SRC=") || !line.contains("DPT=") {
            continue;
        }
        let Some(src) = extract_kv_token(line, "SRC=") else {
            continue;
        };
        let Some(dpt_str) = extract_kv_token(line, "DPT=") else {
            continue;
        };
        let Ok(ip) = src.parse::<IpAddr>() else {
            continue;
        };
        let Ok(port) = dpt_str.parse::<u16>() else {
            continue;
        };
        let entry = ports_by_ip
            .entry(ip.to_string())
            .or_insert_with(|| (HashSet::new(), line.clone()));
        entry.0.insert(port);
        entry.1 = line.clone();
    }

    let min_ports = threshold.max(2) as usize;
    ports_by_ip
        .into_iter()
        .filter(|(_, (ports, _))| ports.len() >= min_ports)
        .map(|(ip, (ports, sample))| DetectionFinding {
            category: "port_scan".to_string(),
            ip,
            hits: ports.len() as u32,
            reason: format!("端口扫描告警：短时间探测 {} 个不同端口", ports.len()),
            sample: sample.chars().take(240).collect(),
        })
        .collect()
}

/// Pure Rust Nginx abnormal Web scanner / CC detector.
pub fn analyze_web_lines(lines: &[String], threshold: u32) -> Vec<DetectionFinding> {
    const BAD_PATTERNS: &[&str] = &[
        "/.env",
        "/.git",
        "/wp-login.php",
        "/xmlrpc.php",
        "/phpmyadmin",
        "/eval-stdin.php",
        "/boaform/admin",
        "/cgi-bin/",
        "/vendor/phpunit",
        "../",
    ];
    let mut counts: HashMap<String, (u32, String)> = HashMap::new();
    for line in lines {
        let lower = line.to_lowercase();
        let is_bad_path = BAD_PATTERNS.iter().any(|p| lower.contains(p));
        let is_err_status = line.contains("\" 404 ")
            || line.contains("\" 403 ")
            || line.contains("\" 444 ")
            || line.contains("\" 429 ");
        if !is_bad_path && !is_err_status {
            continue;
        }
        let Some(first_tok) = line.split_whitespace().next() else {
            continue;
        };
        if first_tok.parse::<IpAddr>().is_err() {
            continue;
        }
        let weight = if is_bad_path { 3 } else { 1 };
        let entry = counts.entry(first_tok.to_string()).or_insert((0, line.clone()));
        entry.0 += weight;
        entry.1 = line.clone();
    }

    let min_score = threshold.max(3);
    counts
        .into_iter()
        .filter(|(_, (score, _))| *score >= min_score)
        .map(|(ip, (hits, sample))| DetectionFinding {
            category: "web".to_string(),
            ip,
            hits,
            reason: format!("异常 Web 扫描/高频探测（风险分 {}）", hits),
            sample: sample.chars().take(240).collect(),
        })
        .collect()
}

fn read_ssh_log_tail(max_lines: usize) -> Vec<String> {
    for candidate in ["/var/log/auth.log", "/var/log/messages", "/var/log/secure"] {
        let p = Path::new(candidate);
        if p.is_file() {
            let lines = read_tail_lines(p, max_lines);
            if !lines.is_empty() {
                return lines;
            }
        }
    }
    // Fallback for Debian 12+ where rsyslog is omitted and sshd logs to systemd-journald
    #[cfg(target_os = "linux")]
    {
        if let Ok(out) = Command::new("journalctl")
            .args(["-u", "ssh", "-u", "sshd", "-n", &max_lines.to_string(), "--no-pager"])
            .output()
            && out.status.success()
        {
            return String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(|s| s.to_string())
                .collect();
        }
    }
    vec![]
}

fn read_kernel_log_tail(max_lines: usize) -> Vec<String> {
    for candidate in ["/var/log/kern.log", "/var/log/syslog", "/var/log/messages"] {
        let p = Path::new(candidate);
        if p.is_file() {
            let lines = read_tail_lines(p, max_lines);
            if !lines.is_empty() {
                return lines;
            }
        }
    }
    vec![]
}

fn read_tail_lines(path: &Path, max_lines: usize) -> Vec<String> {
    let Ok(content) = fs::read_to_string(path) else {
        return vec![];
    };
    let all: Vec<&str> = content.lines().collect();
    let start = all.len().saturating_sub(max_lines);
    all[start..].iter().map(|s| (*s).to_string()).collect()
}

fn extract_ip_after_keyword(line: &str, kw: &str) -> Option<String> {
    let idx = line.find(kw)?;
    let rest = &line[idx + kw.len()..];
    for tok in rest.split_whitespace().take(3) {
        let clean = tok.trim_matches(|c: char| !c.is_ascii_hexdigit() && c != '.' && c != ':');
        if let Ok(ip) = clean.parse::<IpAddr>() {
            return Some(ip.to_string());
        }
    }
    None
}

fn extract_first_ip(line: &str) -> Option<String> {
    for tok in line.split_whitespace() {
        let clean = tok.trim_matches(|c: char| !c.is_ascii_hexdigit() && c != '.' && c != ':');
        if let Ok(ip) = clean.parse::<IpAddr>() {
            return Some(ip.to_string());
        }
    }
    None
}

fn extract_kv_token<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let idx = line.find(key)?;
    let rest = &line[idx + key.len()..];
    rest.split_whitespace().next()
}

fn is_local_or_private_safe(ip_str: &str) -> bool {
    let Ok(ip) = ip_str.parse::<IpAddr>() else {
        return true;
    };
    ip.is_loopback()
}

// ---------- SSH Hardening ----------

pub fn read_ssh_hardening() -> SshHardeningStatus {
    let path = "/etc/ssh/sshd_config";
    let Ok(content) = fs::read_to_string(path) else {
        return SshHardeningStatus {
            config_path: path.to_string(),
            readable: false,
            port: 22,
            password_auth: true,
            permit_root_login: "yes".to_string(),
            pubkey_auth: true,
        };
    };

    let mut port = 22u16;
    let mut password_auth = true;
    let mut permit_root_login = "prohibit-password".to_string();
    let mut pubkey_auth = true;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let (Some(k), Some(v)) = (parts.next(), parts.next()) else {
            continue;
        };
        if k.eq_ignore_ascii_case("Port") {
            if let Ok(p) = v.parse::<u16>() {
                port = p;
            }
        } else if k.eq_ignore_ascii_case("PasswordAuthentication") {
            password_auth = !v.eq_ignore_ascii_case("no");
        } else if k.eq_ignore_ascii_case("PermitRootLogin") {
            permit_root_login = v.to_lowercase();
        } else if k.eq_ignore_ascii_case("PubkeyAuthentication") {
            pubkey_auth = !v.eq_ignore_ascii_case("no");
        }
    }

    SshHardeningStatus {
        config_path: path.to_string(),
        readable: true,
        port,
        password_auth,
        permit_root_login,
        pubkey_auth,
    }
}

pub fn apply_ssh_hardening(
    port: u16,
    password_auth: bool,
    permit_root_login: &str,
    pubkey_auth: bool,
) -> Result<String, AppError> {
    if port == 0 {
        return Err(AppError::BadRequest("SSH 端口必须在 1~65535 之间".to_string()));
    }
    let root_val = match permit_root_login {
        "yes" | "no" | "prohibit-password" => permit_root_login,
        _ => return Err(AppError::BadRequest("PermitRootLogin 仅支持 yes / prohibit-password / no".to_string())),
    };
    if !password_auth && !pubkey_auth {
        return Err(AppError::Forbidden(
            "防自锁拦截：禁止同时关闭密码登录与公钥登录！".to_string(),
        ));
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = root_val;
        return Err(AppError::BadRequest(
            "SSH 服务加固仅支持 Linux 服务器环境".to_string(),
        ));
    }

    #[cfg(target_os = "linux")]
    {
        let path = "/etc/ssh/sshd_config";
        let orig = fs::read_to_string(path)
            .map_err(|e| AppError::Forbidden(format!("读取 {} 失败（需 root 权限）: {}", path, e)))?;
        let backup_path = "/etc/ssh/sshd_config.zmpanel.bak";
        let _ = fs::write(backup_path, &orig);

        let updated = update_sshd_directives(
            &orig,
            &[
                ("Port", port.to_string()),
                (
                    "PasswordAuthentication",
                    if password_auth { "yes" } else { "no" }.to_string(),
                ),
                ("PermitRootLogin", root_val.to_string()),
                (
                    "PubkeyAuthentication",
                    if pubkey_auth { "yes" } else { "no" }.to_string(),
                ),
            ],
        );

        fs::write(path, &updated)
            .map_err(|e| AppError::Forbidden(format!("写入 {} 失败: {}", path, e)))?;

        // Validate sshd config syntax before reloading; rollback immediately on failure!
        if let Ok(check) = Command::new("sshd").arg("-t").output()
            && !check.status.success()
        {
            let _ = fs::write(path, &orig);
            return Err(AppError::BadRequest(format!(
                "sshd -t 语法校验失败，已自动回滚原配置: {}",
                String::from_utf8_lossy(&check.stderr)
            )));
        }

        // Reload sshd across systemd / OpenRC
        let _ = Command::new("systemctl").args(["reload", "ssh"]).status();
        let _ = Command::new("systemctl").args(["reload", "sshd"]).status();
        let _ = Command::new("rc-service").args(["sshd", "reload"]).status();

        Ok(format!(
            "SSH 加固配置已校验并生效（端口 {}，密码登录 {}，Root 登录 {}）",
            port,
            if password_auth { "允许" } else { "禁用" },
            root_val
        ))
    }
}

#[cfg(target_os = "linux")]
fn update_sshd_directives(orig: &str, updates: &[(&str, String)]) -> String {
    let mut remaining: HashMap<String, String> = updates
        .iter()
        .map(|(k, v)| (k.to_lowercase(), format!("{} {}", k, v)))
        .collect();
    let mut lines_out = Vec::new();
    for line in orig.lines() {
        let trimmed = line.trim();
        let key_opt = if !trimmed.starts_with('#') {
            trimmed.split_whitespace().next().map(|s| s.to_lowercase())
        } else {
            None
        };
        if let Some(k) = key_opt
            && let Some(replacement) = remaining.remove(&k)
        {
            lines_out.push(replacement);
            continue;
        }
        lines_out.push(line.to_string());
    }
    if !remaining.is_empty() {
        lines_out.push("\n# Managed by zmpanel SSH Hardening".to_string());
        for (_, directive) in remaining {
            lines_out.push(directive);
        }
    }
    lines_out.join("\n") + "\n"
}
