use chrono::Utc;
use std::collections::BTreeSet;
use std::time::Duration;
use zmpanel_core::{
    AppError, SecurityBaselineCheck, VulnPackageItem, VulnScanReport, VulnWatchConfig,
};

use crate::notify;
use crate::state::AppState;
use crate::store::detect_os_family;

/// Extract deduplicated CVE / DSA / USN identifiers from Debian/Ubuntu/Alpine changelog text.
/// When `stop_at_version` is non-empty, stops scanning once the changelog header for the already-installed
/// version is reached so only newly fixed CVEs in the pending upgrade are reported.
pub fn extract_cves_from_changelog(changelog: &str, stop_at_version: &str) -> Vec<String> {
    let mut relevant = String::new();
    let stop_marker = if stop_at_version.trim().is_empty() {
        None
    } else {
        Some(format!("({})", stop_at_version.trim()))
    };

    for line in changelog.lines() {
        // Debian changelog entry header starts at column 0: `pkg (ver) suite; urgency=...`
        if let Some(ref marker) = stop_marker
            && !line.starts_with(' ')
            && !line.starts_with('\t')
            && line.contains(marker.as_str())
        {
            break;
        }
        relevant.push_str(line);
        relevant.push('\n');
        // Bound scan size for safety on huge changelogs
        if relevant.len() > 24 * 1024 {
            break;
        }
    }

    extract_cve_ids(&relevant)
}

/// Pure zero-regex extractor for `CVE-YYYY-NNNN...`, `DSA-NNNN-N`, and `USN-NNNN-N`.
pub fn extract_cve_ids(text: &str) -> Vec<String> {
    let mut found = BTreeSet::new();
    let upper = text.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Match CVE-YYYY-NNNN+
        if i + 13 <= len && &bytes[i..i + 4] == b"CVE-" {
            let year_ok = bytes[i + 4..i + 8].iter().all(|b| b.is_ascii_digit());
            if year_ok && bytes[i + 8] == b'-' {
                let mut end = i + 9;
                while end < len && bytes[end].is_ascii_digit() {
                    end += 1;
                }
                if end - (i + 9) >= 4 && end - (i + 9) <= 7 {
                    found.insert(upper[i..end].to_string());
                    i = end;
                    continue;
                }
            }
        }
        // Match DSA-NNNN-N or USN-NNNN-N
        if i + 8 <= len && (&bytes[i..i + 4] == b"DSA-" || &bytes[i..i + 4] == b"USN-") {
            let mut end = i + 4;
            while end < len && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end - (i + 4) >= 4 {
                if end + 2 <= len && bytes[end] == b'-' && bytes[end + 1].is_ascii_digit() {
                    end += 1;
                    while end < len && bytes[end].is_ascii_digit() {
                        end += 1;
                    }
                }
                found.insert(upper[i..end].to_string());
                i = end;
                continue;
            }
        }
        i += 1;
    }

    // Put CVE-* first (newest first), followed by DSA/USN
    let mut list: Vec<String> = found.into_iter().collect();
    list.sort_by(|a, b| {
        let a_cve = a.starts_with("CVE-");
        let b_cve = b.starts_with("CVE-");
        b_cve.cmp(&a_cve).then_with(|| b.cmp(a))
    });
    list.truncate(12);
    list
}

/// Classify an upgradable package into security category, severity, and human-readable advisory.
pub fn classify_package(pkg: &str, is_security_repo: bool) -> (&'static str, &'static str, &'static str) {
    let p = pkg.to_lowercase();

    // Linux Kernel (local privilege escalation, network stack CVEs)
    if p.starts_with("linux-image")
        || p.starts_with("linux-libc")
        || p.starts_with("linux-headers")
        || p == "linux-lts"
        || p == "linux-virt"
        || p == "linux-edge"
        || p.starts_with("kernel")
    {
        return (
            "kernel",
            "critical",
            "Linux 内核安全更新（涉及内核提权/网络栈漏洞修复，升级后需重启服务器生效）",
        );
    }

    // Core privilege escalation & libc / crypto primitives
    if p == "sudo"
        || p.starts_with("polkit")
        || p.starts_with("policykit")
        || p == "libc6"
        || p.starts_with("glibc")
        || p == "musl"
        || p.starts_with("libpam")
        || p == "pam"
        || p.starts_with("systemd")
        || p == "dbus"
        || p == "xz-utils"
        || p.starts_with("liblzma")
        || p == "util-linux"
        || p == "passwd"
        || p == "shadow"
    {
        return (
            "privilege",
            if is_security_repo || p == "sudo" || p.starts_with("polkit") || p == "libc6" || p == "xz-utils" {
                "critical"
            } else {
                "high"
            },
            "系统核心提权/认证基础组件更新（sudo / polkit / glibc / PAM / systemd 等）",
        );
    }

    // OpenSSH
    if p.starts_with("openssh") {
        return (
            "ssh",
            "critical",
            "OpenSSH 远程登录组件安全更新（防止远程代码执行 / 认证绕过漏洞）",
        );
    }

    // Nginx, OpenSSL, curl & Web stack
    if p.starts_with("nginx")
        || p.starts_with("openssl")
        || p.starts_with("libssl")
        || p == "curl"
        || p.starts_with("libcurl")
        || p == "ca-certificates"
        || p.starts_with("caddy")
    {
        return (
            "web",
            if p.starts_with("nginx") || p.starts_with("openssl") || p.starts_with("libssl") {
                "high"
            } else if is_security_repo {
                "high"
            } else {
                "medium"
            },
            "Web 服务器与 TLS 加密库安全更新（Nginx / OpenSSL / curl）",
        );
    }

    // Node.js, Python, Go runtimes
    if p.starts_with("nodejs")
        || p == "npm"
        || p.starts_with("libnode")
        || p.starts_with("python3")
        || p.starts_with("golang")
        || p.starts_with("openjdk")
    {
        return (
            "runtime",
            if p.starts_with("nodejs") || p.starts_with("libnode") || is_security_repo {
                "high"
            } else {
                "medium"
            },
            "应用运行时安全更新（Node.js / Python / Go 等高危 CVE 修复）",
        );
    }

    // Docker / Container runtimes (runc / containerd escape CVEs)
    if p.starts_with("docker") || p.starts_with("containerd") || p == "runc" || p.starts_with("podman") {
        return (
            "container",
            "high",
            "容器引擎安全更新（修复容器逃逸 / runc / containerd 漏洞）",
        );
    }

    // Databases
    if p.starts_with("redis") || p.starts_with("postgresql") || p.starts_with("mongodb") || p.starts_with("sqlite") {
        return (
            "database",
            if is_security_repo { "high" } else { "medium" },
            "数据库服务安全补丁更新",
        );
    }

    if is_security_repo {
        (
            "system",
            "medium",
            "来自发行版官方安全通道 (-security) 的安全更新包",
        )
    } else {
        ("system", "low", "发行版常规软件包更新")
    }
}

/// Parse `apt list --upgradable` output lines into structured upgradable package items.
/// Example line:
/// `nginx/stable-security 1.26.3-1+deb13u1 amd64 [upgradable from: 1.26.2-1]`
pub fn parse_apt_upgradable(output: &str) -> Vec<VulnPackageItem> {
    let mut items = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("Listing") || !trimmed.contains("[upgradable from:") {
            continue;
        }
        let Some((pkg_repo, rest)) = trimmed.split_once(' ') else {
            continue;
        };
        let Some((pkg, repo)) = pkg_repo.split_once('/') else {
            continue;
        };
        let candidate = rest.split_whitespace().next().unwrap_or("").to_string();
        let current = trimmed
            .split("[upgradable from:")
            .nth(1)
            .and_then(|s| s.split(']').next())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        let is_sec = repo.to_lowercase().contains("security");
        let (category, severity, advisory) = classify_package(pkg, is_sec);

        items.push(VulnPackageItem {
            package: pkg.to_string(),
            category: category.to_string(),
            severity: severity.to_string(),
            current_version: current,
            candidate_version: candidate,
            is_security_repo: is_sec,
            advisory: advisory.to_string(),
            cves: Vec::new(),
        });
    }
    sort_packages(&mut items);
    items
}

/// Parse Alpine `apk version -l '<'` output.
/// Example line:
/// `openssl-3.3.1-r0                          < 3.3.2-r0`
pub fn parse_apk_upgradable(output: &str) -> Vec<VulnPackageItem> {
    let mut items = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("Installed:") || !trimmed.contains(" < ") {
            continue;
        }
        let Some((left, right)) = trimmed.split_once(" < ") else {
            continue;
        };
        let pkg_with_ver = left.trim();
        let candidate = right.trim().to_string();

        // Split `<name>-<ver>-r<rel>` at the dash preceding the version digit
        let (pkg, current) = split_apk_name_version(pkg_with_ver);
        let (category, severity, advisory) = classify_package(&pkg, true);

        items.push(VulnPackageItem {
            package: pkg,
            category: category.to_string(),
            severity: severity.to_string(),
            current_version: current,
            candidate_version: candidate,
            is_security_repo: true,
            advisory: advisory.to_string(),
            cves: Vec::new(),
        });
    }
    sort_packages(&mut items);
    items
}

fn split_apk_name_version(s: &str) -> (String, String) {
    let bytes = s.as_bytes();
    for i in 0..bytes.len().saturating_sub(1) {
        if bytes[i] == b'-' && bytes[i + 1].is_ascii_digit() {
            return (s[..i].to_string(), s[i + 1..].to_string());
        }
    }
    (s.to_string(), String::new())
}

fn severity_rank(s: &str) -> u8 {
    match s {
        "critical" => 0,
        "high" => 1,
        "medium" => 2,
        _ => 3,
    }
}

fn sort_packages(items: &mut [VulnPackageItem]) {
    items.sort_by(|a, b| {
        severity_rank(&a.severity)
            .cmp(&severity_rank(&b.severity))
            .then_with(|| b.is_security_repo.cmp(&a.is_security_repo))
            .then_with(|| a.package.cmp(&b.package))
    });
}

/// Evaluate standalone Node.js binary version against EOL / non-LTS baselines.
pub fn check_nodejs_version(ver_raw: &str) -> SecurityBaselineCheck {
    let clean = ver_raw.trim().trim_start_matches('v');
    let major = clean
        .split('.')
        .next()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);

    if major == 0 {
        return SecurityBaselineCheck {
            id: "runtime-nodejs".to_string(),
            category: "runtime".to_string(),
            title: "Node.js 运行时安全版本检查".to_string(),
            severity: "info".to_string(),
            status: "pass".to_string(),
            detail: format!("检测到 Node.js 输出: {}（无法解析主版本）", ver_raw.trim()),
            remediation: "建议定期检查 Node.js 官方安全通告并保持在活跃 LTS 分支".to_string(),
            cves: Vec::new(),
        };
    }

    if major < 20 {
        SecurityBaselineCheck {
            id: "runtime-nodejs".to_string(),
            category: "runtime".to_string(),
            title: "Node.js 处于已停止维护 (EOL) 高危版本".to_string(),
            severity: "high".to_string(),
            status: "risk".to_string(),
            detail: format!(
                "当前系统 Node.js 版本为 v{}，已停止官方安全补丁维护，存在多个未修补的 HTTP/TLS/权限绕过 CVE",
                clean
            ),
            remediation: "请尽快升级至 Node.js 22 LTS 或 24 LTS 版本".to_string(),
            cves: Vec::new(),
        }
    } else if major % 2 == 1 {
        SecurityBaselineCheck {
            id: "runtime-nodejs".to_string(),
            category: "runtime".to_string(),
            title: "Node.js 使用非 LTS 过渡版本".to_string(),
            severity: "medium".to_string(),
            status: "warn".to_string(),
            detail: format!("当前 Node.js 版本为 v{}（奇数非 LTS 分支，安全维护周期极短）", clean),
            remediation: "生产服务器建议切换为偶数 LTS 分支（如 v22 LTS / v24 LTS）".to_string(),
            cves: Vec::new(),
        }
    } else {
        SecurityBaselineCheck {
            id: "runtime-nodejs".to_string(),
            category: "runtime".to_string(),
            title: "Node.js 运行时版本在活跃 LTS 分支".to_string(),
            severity: "pass".to_string(),
            status: "pass".to_string(),
            detail: format!("当前 Node.js 版本为 v{}（请保持小版本同步更新安全补丁）", clean),
            remediation: "无需处理，关注官方小版本安全发布即可".to_string(),
            cves: Vec::new(),
        }
    }
}

/// Check OpenSSH version string against known high-profile CVEs (e.g., CVE-2024-6387 regreSSHion 8.5p1..9.7p1).
pub fn check_openssh_version(ver_raw: &str) -> SecurityBaselineCheck {
    // Example: "OpenSSH_9.2p1 Debian-2+deb12u3, OpenSSL 3.0.15"
    let first = ver_raw.lines().next().unwrap_or(ver_raw).trim();
    let parsed = first
        .split("OpenSSH_")
        .nth(1)
        .and_then(|s| s.split(|c: char| !c.is_ascii_digit() && c != '.').next())
        .and_then(|v| {
            let mut parts = v.split('.');
            let maj = parts.next()?.parse::<u32>().ok()?;
            let min = parts.next()?.parse::<u32>().ok()?;
            Some((maj, min))
        });

    let is_backported_distro = first.contains("Debian-") || first.contains("Ubuntu-");

    match parsed {
        Some((maj, min)) if (maj < 8) || (maj == 8 && min < 5) => SecurityBaselineCheck {
            id: "ssh-version".to_string(),
            category: "ssh".to_string(),
            title: "OpenSSH 版本过旧".to_string(),
            severity: "high".to_string(),
            status: "risk".to_string(),
            detail: format!("检测到较旧的 OpenSSH: {}", first),
            remediation: "请通过下方安全补丁升级 openssh-server，或升级系统发行版".to_string(),
            cves: Vec::new(),
        },
        Some((maj, min)) if ((maj == 8 && min >= 5) || (maj == 9 && min <= 7)) && !is_backported_distro => {
            SecurityBaselineCheck {
                id: "ssh-version".to_string(),
                category: "ssh".to_string(),
                title: "OpenSSH 位于 CVE-2024-6387 (regreSSHion) 影响版本区间".to_string(),
                severity: "critical".to_string(),
                status: "risk".to_string(),
                detail: format!(
                    "检测到 {}（OpenSSH 8.5p1 ~ 9.7p1 存在远程未认证提权漏洞 CVE-2024-6387）",
                    first
                ),
                remediation: "立即升级 openssh-server 至 >= 9.8p1 或应用发行版安全补丁".to_string(),
                cves: vec!["CVE-2024-6387".to_string()],
            }
        }
        _ => SecurityBaselineCheck {
            id: "ssh-version".to_string(),
            category: "ssh".to_string(),
            title: "OpenSSH 版本基线正常".to_string(),
            severity: "pass".to_string(),
            status: "pass".to_string(),
            detail: if first.is_empty() {
                "未检测到 OpenSSH 版本信息".to_string()
            } else {
                format!("当前版本: {}", first)
            },
            remediation: "如有 openssh-server 可升级安全补丁，请及时点击上方修复".to_string(),
            cves: Vec::new(),
        },
    }
}

/// Check `/etc/passwd` for unexpected extra UID 0 accounts and `/etc/shadow` for empty passwords.
pub fn check_passwd_shadow_baseline(
    passwd_text: &str,
    shadow_text: Option<&str>,
) -> Vec<SecurityBaselineCheck> {
    let mut out = Vec::new();
    let mut extra_uid0 = Vec::new();
    for line in passwd_text.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() >= 3 && parts[2] == "0" && parts[0] != "root" {
            extra_uid0.push(parts[0].to_string());
        }
    }
    if extra_uid0.is_empty() {
        out.push(SecurityBaselineCheck {
            id: "priv-uid0".to_string(),
            category: "privilege".to_string(),
            title: "特权账户 (UID=0) 检查通过".to_string(),
            severity: "pass".to_string(),
            status: "pass".to_string(),
            detail: "仅 root 拥有 UID 0 超级管理员权限，未发现异常影子 root 账户".to_string(),
            remediation: "保持现状".to_string(),
            cves: Vec::new(),
        });
    } else {
        out.push(SecurityBaselineCheck {
            id: "priv-uid0".to_string(),
            category: "privilege".to_string(),
            title: "发现非 root 的 UID=0 超级特权账户".to_string(),
            severity: "critical".to_string(),
            status: "risk".to_string(),
            detail: format!("异常 UID=0 账户: {}（高度疑似后门或危险提权配置）", extra_uid0.join(", ")),
            remediation: "立即核查 /etc/passwd 并使用 userdel 删除不明账户".to_string(),
            cves: Vec::new(),
        });
    }

    if let Some(shadow) = shadow_text {
        let mut empty_pw = Vec::new();
        for line in shadow.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 2 && parts[1].is_empty() {
                empty_pw.push(parts[0].to_string());
            }
        }
        if empty_pw.is_empty() {
            out.push(SecurityBaselineCheck {
                id: "priv-empty-pass".to_string(),
                category: "privilege".to_string(),
                title: "空密码账户检查通过".to_string(),
                severity: "pass".to_string(),
                status: "pass".to_string(),
                detail: "未在 /etc/shadow 中发现无密码可登录账户".to_string(),
                remediation: "保持现状".to_string(),
                cves: Vec::new(),
            });
        } else {
            out.push(SecurityBaselineCheck {
                id: "priv-empty-pass".to_string(),
                category: "privilege".to_string(),
                title: "存在空密码系统账户".to_string(),
                severity: "critical".to_string(),
                status: "risk".to_string(),
                detail: format!("空密码账户: {}", empty_pw.join(", ")),
                remediation: "立即执行 passwd -l <用户名> 锁定或设置强密码".to_string(),
                cves: Vec::new(),
            });
        }
    }
    out
}

/// Check `ss -tlnH` listening sockets for sensitive services exposed directly to 0.0.0.0 / [::].
pub fn check_exposed_ports_baseline(ss_output: &str) -> Vec<SecurityBaselineCheck> {
    let watched = [
        (2375u16, "Docker 未加密 API (2375)", "critical", "公网暴露 2375 等于直接交出宿主机 root 权限，请立即关闭 Docker TCP 监听或仅绑定 127.0.0.1"),
        (6379u16, "Redis (6379)", "high", "Redis 直接暴露公网极易被未授权写入 SSH 公钥或定时任务提权，请改 bind 127.0.0.1 并设置强密码"),
        (5432u16, "PostgreSQL (5432)", "medium", "建议仅监听 127.0.0.1，或在防火墙限制仅允许特定源 IP 访问"),
        (27017u16, "MongoDB (27017)", "high", "MongoDB 暴露公网极易遭遇勒索删库，请绑定 127.0.0.1 并启用认证"),
    ];

    let mut exposed = Vec::new();
    for line in ss_output.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 4 {
            continue;
        }
        // `ss -tlnH` col[3] is Local Address:Port
        let local_addr = cols[3];
        let is_wildcard = local_addr.starts_with("0.0.0.0:")
            || local_addr.starts_with("*:")
            || local_addr.starts_with("[::]:")
            || local_addr.starts_with(":::");
        if !is_wildcard {
            continue;
        }
        let port = local_addr
            .rsplit(':')
            .next()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(0);
        for &(wp, label, sev, rem) in &watched {
            if port == wp && !exposed.iter().any(|(p, _, _, _)| *p == wp) {
                exposed.push((wp, label, sev, rem));
            }
        }
    }

    if exposed.is_empty() {
        vec![SecurityBaselineCheck {
            id: "net-exposed-ports".to_string(),
            category: "network".to_string(),
            title: "高危敏感端口公网暴露检查通过".to_string(),
            severity: "pass".to_string(),
            status: "pass".to_string(),
            detail: "未发现 Docker(2375) / Redis(6379) / PostgreSQL(5432) / MongoDB(27017) 直接监听在 0.0.0.0".to_string(),
            remediation: "保持敏感数据库与管理端口仅监听 127.0.0.1".to_string(),
            cves: Vec::new(),
        }]
    } else {
        exposed
            .into_iter()
            .map(|(port, label, sev, rem)| SecurityBaselineCheck {
                id: format!("net-exposed-{}", port),
                category: "network".to_string(),
                title: format!("{} 直接监听在全网地址 (0.0.0.0)", label),
                severity: sev.to_string(),
                status: "risk".to_string(),
                detail: format!("检测到端口 {} 绑定在全网接口，若防火墙未拦截将暴露于公网", port),
                remediation: rem.to_string(),
                cves: Vec::new(),
            })
            .collect()
    }
}

/// Run a full vulnerability & security baseline scan on the local host.
pub fn run_vuln_scan(refresh_index: bool) -> VulnScanReport {
    let os_family = detect_os_family();
    let kernel_running = sysinfo::System::kernel_version().unwrap_or_else(|| "-".to_string());
    #[allow(unused_mut)]
    let mut packages: Vec<VulnPackageItem> = Vec::new();
    let mut baselines: Vec<SecurityBaselineCheck> = Vec::new();
    #[allow(unused_mut)]
    let mut reboot_required = false;

    #[cfg(target_os = "linux")]
    {
        use std::fs;
        use std::path::Path;
        use std::process::Command;

        // 1. Package manager security updates scan
        if os_family == "alpine" {
            if refresh_index {
                let _ = Command::new("apk").args(["update"]).output();
            }
            if let Ok(out) = Command::new("apk").args(["version", "-l", "<"]).output() {
                let txt = String::from_utf8_lossy(&out.stdout);
                packages = parse_apk_upgradable(&txt);
            }
        } else if Command::new("apt").arg("--version").output().is_ok() {
            if refresh_index {
                let _ = Command::new("apt-get")
                    .args(["update", "-qq"])
                    .env("DEBIAN_FRONTEND", "noninteractive")
                    .output();
            }
            if let Ok(out) = Command::new("apt")
                .args(["list", "--upgradable"])
                .env("LANG", "C")
                .output()
            {
                let txt = String::from_utf8_lossy(&out.stdout);
                packages = parse_apt_upgradable(&txt);
            }

            // Enrich security / high-priority upgradable packages with real CVE / DSA / USN IDs from apt changelog
            let mut enriched = 0usize;
            for item in &mut packages {
                if !(item.is_security_repo || item.severity == "critical" || item.severity == "high") {
                    continue;
                }
                if enriched >= 16 {
                    break;
                }
                enriched += 1;
                if let Ok(ch_out) = Command::new("apt-get")
                    .args(["changelog", "-qq", &item.package])
                    .env("LANG", "C")
                    .output()
                    && ch_out.status.success()
                {
                    let ch_txt = String::from_utf8_lossy(&ch_out.stdout);
                    let cves = extract_cves_from_changelog(&ch_txt, &item.current_version);
                    if !cves.is_empty() {
                        if item.severity == "low" || item.severity == "medium" {
                            item.severity = "high".to_string();
                        }
                        item.cves = cves;
                    }
                }
            }
            sort_packages(&mut packages);
        }

        // 2. Kernel pending reboot check
        reboot_required = Path::new("/var/run/reboot-required").exists();
        if reboot_required {
            baselines.push(SecurityBaselineCheck {
                id: "kernel-reboot".to_string(),
                category: "kernel".to_string(),
                title: "已安装新内核/关键库补丁，等待重启生效".to_string(),
                severity: "high".to_string(),
                status: "warn".to_string(),
                detail: format!(
                    "系统标记 /var/run/reboot-required（当前运行内核: {}），重启前旧内核补丁不会生效",
                    kernel_running
                ),
                remediation: "请在业务低峰期重启服务器以启用新内核与底层库安全补丁".to_string(),
                cves: Vec::new(),
            });
        } else {
            baselines.push(SecurityBaselineCheck {
                id: "kernel-reboot".to_string(),
                category: "kernel".to_string(),
                title: "内核运行状态正常（无待重启补丁）".to_string(),
                severity: "pass".to_string(),
                status: "pass".to_string(),
                detail: format!("当前运行内核版本: {}", kernel_running),
                remediation: "如上方发现 linux-image 安全更新，请点击修复并择机重启".to_string(),
                cves: Vec::new(),
            });
        }

        // 3. OpenSSH version baseline
        if let Ok(out) = Command::new("ssh").arg("-V").output() {
            let ver = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            baselines.push(check_openssh_version(&ver));
        }

        // 4. SSH config baseline
        let ssh_cfg = crate::security::read_ssh_hardening();
        if ssh_cfg.readable {
            if ssh_cfg.password_auth && ssh_cfg.permit_root_login == "yes" {
                baselines.push(SecurityBaselineCheck {
                    id: "ssh-root-password".to_string(),
                    category: "ssh".to_string(),
                    title: "SSH 同时开启了 Root 密码登录".to_string(),
                    severity: "high".to_string(),
                    status: "risk".to_string(),
                    detail: format!(
                        "端口 {} 允许 PermitRootLogin=yes 且 PasswordAuthentication=yes，极易成为 SSH 爆破目标",
                        ssh_cfg.port
                    ),
                    remediation: "前往「SSH 安全加固」配置密钥登录并关闭 Root 密码登录".to_string(),
                    cves: Vec::new(),
                });
            } else {
                baselines.push(SecurityBaselineCheck {
                    id: "ssh-root-password".to_string(),
                    category: "ssh".to_string(),
                    title: "SSH 认证加固基线良好".to_string(),
                    severity: "pass".to_string(),
                    status: "pass".to_string(),
                    detail: format!(
                        "端口 {} | PermitRootLogin={} | PasswordAuth={}",
                        ssh_cfg.port, ssh_cfg.permit_root_login, ssh_cfg.password_auth
                    ),
                    remediation: "保持密钥优先登录并配合内置 SSH 防爆破引擎".to_string(),
                    cves: Vec::new(),
                });
            }
        }

        // 5. Standalone Node.js version check (catches nvm / manual tar installs)
        if let Ok(out) = Command::new("node").arg("-v").output()
            && out.status.success()
        {
            let ver = String::from_utf8_lossy(&out.stdout);
            baselines.push(check_nodejs_version(&ver));
        }

        // 6. Nginx version check
        if let Ok(out) = Command::new("nginx").arg("-v").output() {
            let ver = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if !ver.is_empty() {
                baselines.push(SecurityBaselineCheck {
                    id: "web-nginx-ver".to_string(),
                    category: "web".to_string(),
                    title: "Nginx 运行版本信息".to_string(),
                    severity: "pass".to_string(),
                    status: "pass".to_string(),
                    detail: format!("已安装 {}", ver),
                    remediation: "可在「软件商店 → Nginx 安全防护」开启隐藏版本号与敏感路径拦截".to_string(),
                    cves: Vec::new(),
                });
            }
        }

        // 7. Privilege escalation baseline (/etc/passwd, /etc/shadow, /etc/sudoers)
        if let Ok(passwd) = fs::read_to_string("/etc/passwd") {
            let shadow = fs::read_to_string("/etc/shadow").ok();
            baselines.extend(check_passwd_shadow_baseline(&passwd, shadow.as_deref()));
        }
        if let Ok(sudoers) = fs::read_to_string("/etc/sudoers")
            && sudoers
                .lines()
                .any(|l| !l.trim().starts_with('#') && l.contains("NOPASSWD") && l.contains("ALL"))
        {
            baselines.push(SecurityBaselineCheck {
                id: "priv-sudoers-nopasswd".to_string(),
                category: "privilege".to_string(),
                title: "/etc/sudoers 存在无密码全局提权 (NOPASSWD: ALL)".to_string(),
                severity: "medium".to_string(),
                status: "warn".to_string(),
                detail: "检测到非注释行配置了 NOPASSWD: ALL，普通用户被入侵后可直接无密码提权至 root".to_string(),
                remediation: "按需缩小 sudo 授权范围，非必要不配置 NOPASSWD: ALL".to_string(),
                cves: Vec::new(),
            });
        }

        // 8. Exposed sensitive ports check (`ss -tlnH`)
        if let Ok(out) = Command::new("ss").args(["-tlnH"]).output()
            && out.status.success()
        {
            let txt = String::from_utf8_lossy(&out.stdout);
            baselines.extend(check_exposed_ports_baseline(&txt));
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = refresh_index;
        baselines.push(SecurityBaselineCheck {
            id: "non-linux-hint".to_string(),
            category: "kernel".to_string(),
            title: "当前为非 Linux 开发预览环境".to_string(),
            severity: "info".to_string(),
            status: "pass".to_string(),
            detail: "部署到 Debian / Ubuntu / Alpine 服务器后将自动扫描 apt/apk 安全补丁、内核/提权基线与敏感端口暴露".to_string(),
            remediation: "在 Linux 服务器使用以获取真实系统漏洞与一键修复能力".to_string(),
            cves: Vec::new(),
        });
    }

    let critical_count = packages.iter().filter(|p| p.severity == "critical").count()
        + baselines.iter().filter(|b| b.status == "risk" && b.severity == "critical").count();
    let high_count = packages.iter().filter(|p| p.severity == "high").count()
        + baselines
            .iter()
            .filter(|b| b.status != "pass" && b.severity == "high")
            .count();
    let med_count = packages.iter().filter(|p| p.severity == "medium").count()
        + baselines
            .iter()
            .filter(|b| b.status != "pass" && b.severity == "medium")
            .count();

    let penalty = (critical_count * 20 + high_count * 10 + med_count * 4).min(90) as u8;
    let security_score = 100u8.saturating_sub(penalty);
    let total_upgradable = packages.len();

    let summary = if critical_count == 0 && high_count == 0 && total_upgradable == 0 {
        "未发现待修复的系统安全补丁与高危基线风险，服务器安全状况良好。".to_string()
    } else {
        format!(
            "扫描完成：发现 {} 项严重、{} 项高危风险，共 {} 个可升级软件包（安全评分 {} / 100）。",
            critical_count, high_count, total_upgradable, security_score
        )
    };

    VulnScanReport {
        scanned_at: Utc::now().timestamp(),
        os_family,
        kernel_running,
        reboot_required,
        security_score,
        critical_count,
        high_count,
        total_upgradable,
        packages,
        baselines,
        summary,
    }
}

pub fn validate_package_name(pkg: &str) -> Result<String, AppError> {
    let trimmed = pkg.trim();
    if trimmed.is_empty() || trimmed.len() > 128 {
        return Err(AppError::BadRequest(format!("非法软件包名: {}", pkg)));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '_' | '-' | ':'))
    {
        return Err(AppError::Forbidden(format!(
            "拒绝非法软件包名（含不允许字符）: {}",
            trimmed
        )));
    }
    Ok(trimmed.to_string())
}

/// Upgrade specified packages (or all critical/high/security packages when `fix_all_security` is true).
pub fn fix_packages(packages: &[String], fix_all_security: bool) -> Result<String, AppError> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (packages, fix_all_security);
        return Err(AppError::BadRequest(
            "一键漏洞补丁修复仅支持 Debian / Ubuntu / Alpine Linux 服务器".to_string(),
        ));
    }

    #[cfg(target_os = "linux")]
    {
        use std::process::Command;
        let target_pkgs: Vec<String> = if !packages.is_empty() {
            let mut valid = Vec::new();
            for p in packages {
                valid.push(validate_package_name(p)?);
            }
            valid
        } else if fix_all_security {
            let report = run_vuln_scan(true);
            report
                .packages
                .into_iter()
                .filter(|p| p.severity == "critical" || p.severity == "high" || p.is_security_repo)
                .map(|p| p.package)
                .collect()
        } else {
            return Err(AppError::BadRequest("请选择至少一个要修复的软件包".to_string()));
        };

        if target_pkgs.is_empty() {
            return Ok("未发现需要升级的高危或安全频道软件包，系统已是最新安全状态。".to_string());
        }

        let os = detect_os_family();
        let out = if os == "alpine" {
            let mut args = vec!["upgrade".to_string()];
            args.extend(target_pkgs.iter().cloned());
            Command::new("apk")
                .args(&args)
                .output()
                .map_err(|e| AppError::Internal(format!("执行 apk upgrade 失败: {}", e)))?
        } else {
            let _ = Command::new("apt-get")
                .args(["update", "-qq"])
                .env("DEBIAN_FRONTEND", "noninteractive")
                .output();
            let mut args = vec![
                "install".to_string(),
                "-y".to_string(),
                "--only-upgrade".to_string(),
                "-o".to_string(),
                "Dpkg::Options::=--force-confdef".to_string(),
                "-o".to_string(),
                "Dpkg::Options::=--force-confold".to_string(),
            ];
            args.extend(target_pkgs.iter().cloned());
            Command::new("apt-get")
                .args(&args)
                .env("DEBIAN_FRONTEND", "noninteractive")
                .output()
                .map_err(|e| AppError::Internal(format!("执行 apt-get 升级失败: {}", e)))?
        };

        let mut combined = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
        .trim()
        .to_string();

        if !out.status.success() {
            return Err(AppError::BadRequest(format!(
                "漏洞修复执行失败:\n{}",
                combined
            )));
        }

        // Post-fix smart actions
        if target_pkgs.iter().any(|p| p.starts_with("nginx")) {
            let test_ok = Command::new("nginx")
                .arg("-t")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if test_ok {
                let _ = Command::new("nginx").args(["-s", "reload"]).output();
                combined.push_str("\n[zmpanel] 已自动校验并平滑重载 Nginx (nginx -t && nginx -s reload)");
            }
        }
        if target_pkgs
            .iter()
            .any(|p| p.starts_with("linux-image") || p == "linux-lts" || p == "linux-virt")
        {
            combined.push_str("\n[zmpanel] 重要提示：Linux 内核安全补丁已安装，请择机重启服务器以使新内核正式生效！");
        }

        Ok(combined)
    }
}

// ---------- Scheduled CVE Watcher & High-Severity Alerting ----------

pub fn load_watch_config(state: &AppState) -> VulnWatchConfig {
    state
        .db
        .get_setting("vuln_watch_config")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_watch_config(state: &AppState, cfg: &VulnWatchConfig) -> Result<(), AppError> {
    let raw = serde_json::to_string(cfg)
        .map_err(|e| AppError::Internal(format!("序列化漏洞巡检配置失败: {}", e)))?;
    state
        .db
        .set_setting("vuln_watch_config", &raw)
        .map_err(|e| AppError::Internal(e.to_string()))
}

/// Returns true iff `severity` meets or exceeds the configured alert threshold.
/// Default threshold `"high"` alerts on `critical` and `high`, suppressing `medium`/`low`/`info`/`pass`.
pub fn meets_alert_threshold(severity: &str, min_severity: &str) -> bool {
    match min_severity {
        "critical" => severity == "critical",
        _ => severity == "critical" || severity == "high",
    }
}

pub fn filter_alertable_vulns<'a>(
    report: &'a VulnScanReport,
    min_severity: &str,
) -> (Vec<&'a VulnPackageItem>, Vec<&'a SecurityBaselineCheck>) {
    let pkgs = report
        .packages
        .iter()
        .filter(|p| meets_alert_threshold(&p.severity, min_severity))
        .collect();
    let bases = report
        .baselines
        .iter()
        .filter(|b| b.status != "pass" && meets_alert_threshold(&b.severity, min_severity))
        .collect();
    (pkgs, bases)
}

pub fn run_watch_cycle(state: &AppState) {
    let mut cfg = load_watch_config(state);
    if !cfg.enabled {
        return;
    }
    let now = Utc::now().timestamp();
    let interval_secs = (cfg.interval_hours.max(1) as i64) * 3600;
    if cfg.last_scan_at > 0 && (now - cfg.last_scan_at) < interval_secs {
        return;
    }

    let report = run_vuln_scan(true);
    cfg.last_scan_at = now;
    cfg.last_summary = report.summary.clone();

    let (alert_pkgs, alert_bases) = filter_alertable_vulns(&report, &cfg.alert_min_severity);

    // Load previously alerted fingerprints so we only alert once per unique vulnerability/version
    let mut seen: BTreeSet<String> = state
        .db
        .get_setting("vuln_notified_keys")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .map(|v| v.into_iter().collect())
        .unwrap_or_default();

    let mut new_lines = Vec::new();
    let mut new_keys = Vec::new();
    let mut auto_fix_targets = Vec::new();

    for p in &alert_pkgs {
        let key = format!("pkg:{}@{}", p.package, p.candidate_version);
        if !seen.contains(&key) {
            let sev_cn = if p.severity == "critical" { "严重" } else { "高危" };
            let cve_str = if p.cves.is_empty() {
                String::new()
            } else {
                format!(" [{}]", p.cves.join(", "))
            };
            new_lines.push(format!(
                "• [{}] {} ({} → {}){}",
                sev_cn, p.package, p.current_version, p.candidate_version, cve_str
            ));
            new_keys.push(key);
        }
        if cfg.auto_fix_non_kernel && p.category != "kernel" {
            auto_fix_targets.push(p.package.clone());
        }
    }

    for b in &alert_bases {
        let key = format!("base:{}:{}", b.id, b.severity);
        if !seen.contains(&key) {
            let sev_cn = if b.severity == "critical" { "严重" } else { "高危" };
            let cve_str = if b.cves.is_empty() {
                String::new()
            } else {
                format!(" [{}]", b.cves.join(", "))
            };
            new_lines.push(format!("• [{}基线] {}{}", sev_cn, b.title, cve_str));
            new_keys.push(key);
        }
    }

    let mut auto_fix_note = String::new();
    if cfg.auto_fix_non_kernel && !auto_fix_targets.is_empty() {
        match fix_packages(&auto_fix_targets, false) {
            Ok(_) => {
                auto_fix_note = format!(
                    "\n\n已自动静默修复 {} 个非内核高危安全包：{}",
                    auto_fix_targets.len(),
                    auto_fix_targets.join(", ")
                );
            }
            Err(e) => {
                auto_fix_note = format!("\n\n自动修复执行失败（请手动检查）：{}", e);
            }
        }
    }

    if !new_lines.is_empty() {
        let host = sysinfo::System::host_name().unwrap_or_else(|| "server".to_string());
        let title = format!("高危安全漏洞告警 ({})", host);
        let body = format!(
            "发现 {} 项新严重/高危安全漏洞（中危/低危已按策略静音）：\n{}{}",
            new_lines.len(),
            new_lines.join("\n"),
            auto_fix_note
        );
        let ncfg = notify::load_notify_config(state);
        if ncfg.enabled {
            if let Ok(channel) = notify::send_notification(&ncfg, &title, &body) {
                cfg.last_alert_at = now;
                let _ = state.db.insert_security_event(
                    "system",
                    "127.0.0.1",
                    &format!("高危 CVE 自动告警 ({} 项)", new_lines.len()),
                    &channel,
                    &new_lines.join(" | "),
                );
            }
        }
        for k in new_keys {
            seen.insert(k);
        }
        // Trim stale keys if set grows large
        let keys_vec: Vec<String> = seen.into_iter().take(500).collect();
        if let Ok(raw_keys) = serde_json::to_string(&keys_vec) {
            let _ = state.db.set_setting("vuln_notified_keys", &raw_keys);
        }
    }

    let _ = save_watch_config(state, &cfg);
}

pub fn spawn_vuln_watcher(state: AppState) {
    tokio::spawn(async move {
        // Wait 60s after boot before first check to keep startup lean
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            let st = state.clone();
            let _ = tokio::task::spawn_blocking(move || run_watch_cycle(&st)).await;
            tokio::time::sleep(Duration::from_secs(300)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_only_new_cves_from_changelog_stopping_at_installed_version() {
        let ch = "\
nginx (1.26.3-1+deb13u2) stable-security; urgency=high

  * Fix HTTP/2 request smuggling (CVE-2025-23419, CVE-2025-53859)
  * Reference DSA-5910-1

 -- Debian Security <team@security.debian.org>  Mon, 10 Nov 2026 10:00:00 +0000

nginx (1.26.2-1) stable; urgency=medium

  * Old already-installed fix CVE-2023-44487
";
        let cves = extract_cves_from_changelog(ch, "1.26.2-1");
        assert!(cves.contains(&"CVE-2025-53859".to_string()));
        assert!(cves.contains(&"CVE-2025-23419".to_string()));
        assert!(cves.contains(&"DSA-5910-1".to_string()));
        assert!(!cves.contains(&"CVE-2023-44487".to_string()));
    }

    #[test]
    fn alerts_only_on_critical_and_high_suppressing_medium_and_low() {
        let report = VulnScanReport {
            scanned_at: 0,
            os_family: "debian".to_string(),
            kernel_running: "6.12".to_string(),
            reboot_required: false,
            security_score: 60,
            critical_count: 1,
            high_count: 1,
            total_upgradable: 4,
            packages: vec![
                VulnPackageItem {
                    package: "openssh-server".to_string(),
                    category: "ssh".to_string(),
                    severity: "critical".to_string(),
                    current_version: "1:9.8p1".to_string(),
                    candidate_version: "1:9.9p2".to_string(),
                    is_security_repo: true,
                    advisory: "".to_string(),
                    cves: vec!["CVE-2025-26465".to_string()],
                },
                VulnPackageItem {
                    package: "nginx".to_string(),
                    category: "web".to_string(),
                    severity: "high".to_string(),
                    current_version: "1.26.1".to_string(),
                    candidate_version: "1.26.3".to_string(),
                    is_security_repo: true,
                    advisory: "".to_string(),
                    cves: vec!["CVE-2025-23419".to_string()],
                },
                VulnPackageItem {
                    package: "curl".to_string(),
                    category: "web".to_string(),
                    severity: "medium".to_string(),
                    current_version: "8.10".to_string(),
                    candidate_version: "8.11".to_string(),
                    is_security_repo: false,
                    advisory: "".to_string(),
                    cves: vec![],
                },
                VulnPackageItem {
                    package: "vim-tiny".to_string(),
                    category: "system".to_string(),
                    severity: "low".to_string(),
                    current_version: "9.0".to_string(),
                    candidate_version: "9.1".to_string(),
                    is_security_repo: false,
                    advisory: "".to_string(),
                    cves: vec![],
                },
            ],
            baselines: vec![
                SecurityBaselineCheck {
                    id: "b1".to_string(),
                    category: "privilege".to_string(),
                    title: "sudoers".to_string(),
                    severity: "medium".to_string(),
                    status: "warn".to_string(),
                    detail: "".to_string(),
                    remediation: "".to_string(),
                    cves: vec![],
                },
                SecurityBaselineCheck {
                    id: "b2".to_string(),
                    category: "network".to_string(),
                    title: "redis exposed".to_string(),
                    severity: "high".to_string(),
                    status: "risk".to_string(),
                    detail: "".to_string(),
                    remediation: "".to_string(),
                    cves: vec![],
                },
            ],
            summary: String::new(),
        };

        let (pkgs, bases) = filter_alertable_vulns(&report, "high");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(bases.len(), 1);
        assert!(pkgs.iter().all(|p| p.severity == "critical" || p.severity == "high"));
        assert_eq!(bases[0].id, "b2");
    }

    #[test]
    fn parses_apt_upgradable_and_ranks_critical_vulns_first() {
        let sample = "\
Listing... Done
vim-tiny/stable 2:9.1.1-1 amd64 [upgradable from: 2:9.1.0-1]
nginx/stable-security 1.26.3-1+deb13u1 amd64 [upgradable from: 1.26.2-1]
openssh-server/stable-security 1:9.9p1-3+deb13u1 amd64 [upgradable from: 1:9.9p1-2]
linux-image-6.12.12-amd64/stable-security 6.12.12-1 amd64 [upgradable from: 6.12.10-1]
sudo/stable-security 1.9.16p2-1 amd64 [upgradable from: 1.9.15p5-1]
nodejs/stable-security 20.18.2+dfsg-1 amd64 [upgradable from: 20.18.0+dfsg-1]
";
        let items = parse_apt_upgradable(sample);
        assert_eq!(items.len(), 6);
        assert_eq!(items[0].severity, "critical");
        assert!(items.iter().any(|i| i.package == "sudo" && i.category == "privilege" && i.severity == "critical"));
        assert!(items.iter().any(|i| i.package == "nginx" && i.category == "web" && i.severity == "high"));
        assert!(items.iter().any(|i| i.package == "nodejs" && i.category == "runtime" && i.severity == "high"));
        assert!(items.iter().any(|i| i.package == "vim-tiny" && i.severity == "low"));
    }

    #[test]
    fn parses_alpine_apk_upgradable() {
        let sample = "\
Installed:                                Available:
openssh-server-9.7_p1-r1                  < 9.9_p1-r0
nginx-1.26.1-r0                           < 1.26.2-r1
";
        let items = parse_apk_upgradable(sample);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].package, "openssh-server");
        assert_eq!(items[0].category, "ssh");
        assert_eq!(items[0].severity, "critical");
        assert_eq!(items[1].package, "nginx");
        assert_eq!(items[1].candidate_version, "1.26.2-r1");
    }

    #[test]
    fn checks_nodejs_openssh_privilege_and_exposed_ports_baselines() {
        let node_old = check_nodejs_version("v18.19.0\n");
        assert_eq!(node_old.status, "risk");
        assert_eq!(node_old.severity, "high");

        let node_lts = check_nodejs_version("v22.14.0\n");
        assert_eq!(node_lts.status, "pass");

        let ssh_vuln = check_openssh_version("OpenSSH_9.6p1, OpenSSL 3.2.1");
        assert_eq!(ssh_vuln.status, "risk");
        assert_eq!(ssh_vuln.severity, "critical");

        let priv_checks = check_passwd_shadow_baseline(
            "root:x:0:0:root:/root:/bin/bash\nbackdoor:x:0:0::/tmp:/bin/sh\n",
            Some("root:$6$abc:19000:0:99999:7:::\nguest::19000:0:99999:7:::\n"),
        );
        assert_eq!(priv_checks.len(), 2);
        assert_eq!(priv_checks[0].status, "risk");
        assert_eq!(priv_checks[1].status, "risk");

        let ports = check_exposed_ports_baseline(
            "LISTEN 0 511 0.0.0.0:6379 0.0.0.0:*\nLISTEN 0 128 127.0.0.1:5432 0.0.0.0:*\n",
        );
        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0].id, "net-exposed-6379");
        assert_eq!(ports[0].status, "risk");
    }

    #[test]
    fn rejects_shell_injection_in_package_names() {
        assert!(validate_package_name("nginx").is_ok());
        assert!(validate_package_name("linux-image-6.12-amd64").is_ok());
        assert!(validate_package_name("nginx; rm -rf /").is_err());
        assert!(validate_package_name("$(whoami)").is_err());
    }
}
