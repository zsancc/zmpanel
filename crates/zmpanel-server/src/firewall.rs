#[cfg(target_os = "linux")]
use std::fs;
use std::net::IpAddr;
#[cfg(target_os = "linux")]
use std::process::Command;
use zmpanel_core::{AppError, FirewallRule, FirewallStatus};

pub fn detect_ssh_port() -> u16 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = fs::read_to_string("/etc/ssh/sshd_config") {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                let mut parts = trimmed.split_whitespace();
                if let (Some(k), Some(v)) = (parts.next(), parts.next())
                    && k.eq_ignore_ascii_case("Port")
                    && let Ok(p) = v.parse::<u16>()
                    && p > 0
                {
                    return p;
                }
            }
        }
    }
    22
}

pub fn detect_backend() -> (String, bool, String) {
    #[cfg(not(target_os = "linux"))]
    {
        (
            "local-rules".to_string(),
            false,
            "当前为非 Linux 环境，防火墙规则保存在本地数据库，部署到 Debian / Ubuntu / Alpine 后将自动下发至 nftables / ufw / iptables".to_string(),
        )
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(out) = Command::new("nft").arg("--version").output()
            && out.status.success()
        {
            let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
            return ("nftables".to_string(), true, format!("已检测到原生 {}", ver));
        }
        if let Ok(out) = Command::new("ufw").arg("status").output()
            && out.status.success()
        {
            let txt = String::from_utf8_lossy(&out.stdout);
            let active = txt.to_lowercase().contains("status: active");
            return (
                "ufw".to_string(),
                active,
                if active {
                    "检测到 UFW 防火墙并处于运行状态".to_string()
                } else {
                    "检测到 UFW（当前未激活）".to_string()
                },
            );
        }
        if let Ok(out) = Command::new("iptables").arg("--version").output()
            && out.status.success()
        {
            let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
            return ("iptables".to_string(), true, format!("已检测到 {}", ver));
        }
        (
            "none".to_string(),
            false,
            "未检测到 nftables / ufw / iptables，建议安装 nftables".to_string(),
        )
    }
}

pub fn get_status(panel_port: u16) -> FirewallStatus {
    let ssh_port = detect_ssh_port();
    let (backend, active, message) = detect_backend();
    let mut protected = vec![ssh_port];
    if panel_port != ssh_port {
        protected.push(panel_port);
    }
    FirewallStatus {
        backend,
        active,
        protected_ports: protected,
        ssh_port,
        panel_port,
        message,
    }
}

pub fn validate_ip_or_cidr(input: &str) -> Result<String, AppError> {
    let clean = input.trim();
    if clean.is_empty() || clean.eq_ignore_ascii_case("any") {
        return Ok("any".to_string());
    }
    if let Some((ip_part, mask_part)) = clean.split_once('/') {
        let ip: IpAddr = ip_part
            .parse()
            .map_err(|_| AppError::BadRequest(format!("无效的 IP 地址: {}", ip_part)))?;
        let prefix: u8 = mask_part
            .parse()
            .map_err(|_| AppError::BadRequest(format!("无效的 CIDR 掩码: {}", mask_part)))?;
        let max_prefix = if ip.is_ipv4() { 32 } else { 128 };
        if prefix > max_prefix {
            return Err(AppError::BadRequest("CIDR 掩码超出范围".to_string()));
        }
        Ok(format!("{}/{}", ip, prefix))
    } else {
        let ip: IpAddr = clean
            .parse()
            .map_err(|_| AppError::BadRequest(format!("无效的 IP 地址: {}", clean)))?;
        Ok(ip.to_string())
    }
}

pub fn validate_port_spec(spec: &str) -> Result<(u16, u16, String), AppError> {
    let clean = spec.trim();
    if clean.is_empty() {
        return Err(AppError::BadRequest("端口不能为空".to_string()));
    }
    if let Some((a, b)) = clean.split_once('-') {
        let start: u16 = a
            .trim()
            .parse()
            .map_err(|_| AppError::BadRequest("无效起始端口".to_string()))?;
        let end: u16 = b
            .trim()
            .parse()
            .map_err(|_| AppError::BadRequest("无效结束端口".to_string()))?;
        if start == 0 || end == 0 || start > end {
            return Err(AppError::BadRequest("端口范围无效（需满足 1 <= 起始 <= 结束 <= 65535）".to_string()));
        }
        Ok((start, end, format!("{}-{}", start, end)))
    } else {
        let port: u16 = clean
            .parse()
            .map_err(|_| AppError::BadRequest("端口必须是 1~65535 的数字或范围（如 8000-8010）".to_string()))?;
        if port == 0 {
            return Err(AppError::BadRequest("端口不能为 0".to_string()));
        }
        Ok((port, port, port.to_string()))
    }
}

/// Anti-lockout guard: prevent denying SSH port or Panel port.
pub fn check_anti_lockout(
    port_start: u16,
    port_end: u16,
    action: &str,
    panel_port: u16,
) -> Result<(), AppError> {
    if action != "deny" {
        return Ok(());
    }
    let ssh_port = detect_ssh_port();
    if ssh_port >= port_start && ssh_port <= port_end {
        return Err(AppError::Forbidden(format!(
            "防自锁保护已拦截：禁止封禁当前 SSH 端口 ({})！如需修改请先在安全中心变更 SSH 端口。",
            ssh_port
        )));
    }
    if panel_port >= port_start && panel_port <= port_end {
        return Err(AppError::Forbidden(format!(
            "防自锁保护已拦截：禁止封禁当前面板监听端口 ({})！",
            panel_port
        )));
    }
    Ok(())
}

/// Best-effort sync of enabled firewall rules and banned IPs to Linux nftables/iptables.
pub fn sync_system_firewall(rules: &[FirewallRule], banned_ips: &[String]) -> String {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (rules, banned_ips);
        "规则已保存到本地数据库（Linux 服务器将自动同步至 nftables/iptables）".to_string()
    }
    #[cfg(target_os = "linux")]
    {
        let (backend, _, _) = detect_backend();
        match backend.as_str() {
            "nftables" => sync_nftables(rules, banned_ips),
            "iptables" => sync_iptables_bans(banned_ips),
            "ufw" => sync_ufw_rules(rules),
            _ => "规则已保存到本地数据库（未检测到系统防火墙命令）".to_string(),
        }
    }
}

#[cfg(target_os = "linux")]
fn sync_nftables(rules: &[FirewallRule], banned_ips: &[String]) -> String {
    // Build an atomic nftables script for our dedicated `inet zmpanel` table (never touches user's other tables)
    let mut script = String::from(
        "add table inet zmpanel\n\
         flush table inet zmpanel\n\
         add chain inet zmpanel input { type filter hook input priority 0; policy accept; }\n\
         add set inet zmpanel banned_v4 { type ipv4_addr; }\n\
         add set inet zmpanel banned_v6 { type ipv6_addr; }\n\
         add rule inet zmpanel input ip saddr @banned_v4 drop\n\
         add rule inet zmpanel input ip6 saddr @banned_v6 drop\n",
    );

    for ip_str in banned_ips {
        if let Ok(ip) = ip_str.parse::<IpAddr>() {
            match ip {
                IpAddr::V4(v4) => {
                    script.push_str(&format!("add element inet zmpanel banned_v4 {{ {} }}\n", v4));
                }
                IpAddr::V6(v6) => {
                    script.push_str(&format!("add element inet zmpanel banned_v6 {{ {} }}\n", v6));
                }
            }
        }
    }

    for r in rules.iter().filter(|r| r.enabled) {
        let verdict = if r.action == "deny" { "drop" } else { "accept" };
        let protos: Vec<&str> = match r.protocol.as_str() {
            "udp" => vec!["udp"],
            "both" => vec!["tcp", "udp"],
            _ => vec!["tcp"],
        };
        for proto in protos {
            let src_clause = if r.source == "any" || r.source.is_empty() {
                String::new()
            } else if r.source.contains(':') {
                format!("ip6 saddr {} ", r.source)
            } else {
                format!("ip saddr {} ", r.source)
            };
            script.push_str(&format!(
                "add rule inet zmpanel input {}{} dport {} {}\n",
                src_clause, proto, r.port, verdict
            ));
        }
    }

    let mut child = match Command::new("nft")
        .arg("-f")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return format!("调用 nft 失败（规则已保存至数据库）: {}", e),
    };

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = stdin.write_all(script.as_bytes());
    }
    match child.wait_with_output() {
        Ok(out) if out.status.success() => "已原子同步至 nftables (table inet zmpanel)".to_string(),
        Ok(out) => format!(
            "规则已保存至数据库，同步 nftables 需要 root/CAP_NET_ADMIN 权限: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) => format!("等待 nft 执行失败: {}", e),
    }
}

#[cfg(target_os = "linux")]
fn sync_iptables_bans(banned_ips: &[String]) -> String {
    // Dedicated chain ZMPANEL_BAN
    let _ = Command::new("iptables").args(["-N", "ZMPANEL_BAN"]).status();
    let _ = Command::new("iptables").args(["-F", "ZMPANEL_BAN"]).status();
    let check = Command::new("iptables")
        .args(["-C", "INPUT", "-j", "ZMPANEL_BAN"])
        .status();
    if !matches!(check, Ok(s) if s.success()) {
        let _ = Command::new("iptables")
            .args(["-I", "INPUT", "1", "-j", "ZMPANEL_BAN"])
            .status();
    }
    for ip in banned_ips {
        if let Ok(IpAddr::V4(v4)) = ip.parse::<IpAddr>() {
            let _ = Command::new("iptables")
                .args(["-A", "ZMPANEL_BAN", "-s", &v4.to_string(), "-j", "DROP"])
                .status();
        }
    }
    "已同步至 iptables (chain ZMPANEL_BAN)".to_string()
}

#[cfg(target_os = "linux")]
fn sync_ufw_rules(rules: &[FirewallRule]) -> String {
    for r in rules.iter().filter(|r| r.enabled) {
        let verb = if r.action == "deny" { "deny" } else { "allow" };
        let spec = if r.protocol == "both" {
            r.port.replace('-', ":")
        } else {
            format!("{}/{}", r.port.replace('-', ":"), r.protocol)
        };
        let _ = Command::new("ufw").args([verb, &spec]).status();
    }
    "已同步至 UFW".to_string()
}
