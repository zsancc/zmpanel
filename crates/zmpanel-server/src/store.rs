use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use sysinfo::System;
use zmpanel_core::{
    AppError, MongoServiceConfig, NginxSecurityConfig, PostgresServiceConfig, RedisServiceConfig,
    StoreAppItem,
};

use crate::files;
use crate::runtime_env;

const BUILTIN_CATALOG_TOML: &str = r#"
[[apps]]
id = "docker"
name = "Docker 引擎 (含 Compose)"
category = "容器运行环境"
description = "轻量容器化运行与 Compose 编排引擎，自动校验宿主机是否支持嵌套（禁止在 OpenVZ / Docker 鸡套娃）。"
min_mem_mb = 256
supported_os = ["debian", "ubuntu", "alpine"]
install_modes = ["native"]
binary = "docker"
version_args = ["--version"]
has_manager = false

[[apps]]
id = "nginx"
name = "Nginx"
category = "Web 服务器"
description = "高性能反向代理与静态 Web 服务器，内置 zmpanel 轻量安全/限流配置面板（替代重型 WAF）。"
min_mem_mb = 32
supported_os = ["debian", "ubuntu", "alpine"]
install_modes = ["native", "docker"]
binary = "nginx"
version_args = ["-v"]
has_manager = true

[[apps]]
id = "python-uv"
name = "Python 管理器 (uv)"
category = "运行环境"
description = "基于 Rust 极速工具 uv 管理多版本 Python、虚拟环境与镜像源，极低内存占用。"
min_mem_mb = 64
supported_os = ["debian", "ubuntu", "alpine", "windows"]
install_modes = ["native"]
binary = "uv"
version_args = ["--version"]
has_manager = true

[[apps]]
id = "redis"
name = "Redis"
category = "缓存 / KV"
description = "轻量级内存键值存储与消息队列，支持内存上限与持久化可视化配置及 Docker 一键部署。"
min_mem_mb = 64
supported_os = ["debian", "ubuntu", "alpine"]
install_modes = ["native", "docker"]
binary = "redis-server"
version_args = ["--version"]
has_manager = true

[[apps]]
id = "postgresql"
name = "PostgreSQL"
category = "关系数据库"
description = "强一致现代关系型数据库，内置小内存服务器参数调优与 Docker 一键编排模板。"
min_mem_mb = 192
supported_os = ["debian", "ubuntu", "alpine"]
install_modes = ["native", "docker"]
binary = "postgres"
version_args = ["--version"]
has_manager = true

[[apps]]
id = "mongodb"
name = "MongoDB"
category = "文档数据库"
description = "NoSQL 文档型数据库。内置 WiredTiger 缓存限制，支持 4.4（兼容无 AVX NAT 鸡）与 7.x Docker 编排。"
min_mem_mb = 384
supported_os = ["debian", "ubuntu"]
install_modes = ["docker", "native"]
binary = "mongod"
version_args = ["--version"]
has_manager = true
"#;

#[derive(Debug, Deserialize)]
struct CatalogFile {
    apps: Vec<CatalogEntry>,
}

#[derive(Debug, Deserialize)]
struct CatalogEntry {
    id: String,
    name: String,
    category: String,
    description: String,
    min_mem_mb: u64,
    supported_os: Vec<String>,
    install_modes: Vec<String>,
    binary: String,
    version_args: Vec<String>,
    has_manager: bool,
}

pub fn detect_os_family() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Ok(txt) = fs::read_to_string("/etc/os-release") {
            let lower = txt.to_lowercase();
            if lower.contains("alpine") {
                return "alpine".to_string();
            }
            if lower.contains("ubuntu") {
                return "ubuntu".to_string();
            }
            if lower.contains("debian") {
                return "debian".to_string();
            }
        }
        "linux".to_string()
    }
    #[cfg(windows)]
    {
        "windows".to_string()
    }
    #[cfg(all(not(target_os = "linux"), not(windows)))]
    {
        std::env::consts::OS.to_string()
    }
}

pub fn list_store_apps(store_dir: &Path) -> Vec<StoreAppItem> {
    let mut entries: Vec<CatalogEntry> = toml::from_str::<CatalogFile>(BUILTIN_CATALOG_TOML)
        .map(|c| c.apps)
        .unwrap_or_default();

    // Load user custom TOML manifests from <data_dir>/store/custom/*.toml
    let custom_dir = store_dir.join("custom");
    if let Ok(rd) = fs::read_dir(&custom_dir) {
        for item in rd.flatten() {
            let path = item.path();
            if path.extension().and_then(|s| s.to_str()) == Some("toml")
                && let Ok(raw) = fs::read_to_string(&path)
                && let Ok(parsed) = toml::from_str::<CatalogFile>(&raw)
            {
                entries.extend(parsed.apps);
            }
        }
    }

    let os_family = detect_os_family();
    let host_env = runtime_env::detect_runtime_env(false);
    let mut sys = System::new();
    sys.refresh_memory();
    let total_mem_mb = sys.total_memory() / (1024 * 1024);

    entries
        .into_iter()
        .map(|e| {
            let (installed, version) = probe_binary(&e.binary, &e.version_args);
            let mut warnings = Vec::new();
            if total_mem_mb > 0 && total_mem_mb < e.min_mem_mb {
                warnings.push(format!(
                    "当前总内存 {} MB 低于建议值 {} MB",
                    total_mem_mb, e.min_mem_mb
                ));
            }
            if !e.supported_os.iter().any(|o| o == &os_family) {
                if host_env.docker_nesting_supported {
                    warnings.push(format!(
                        "当前系统 ({}) 不支持原生包安装，建议改用 Docker 方式或 Linux 节点",
                        os_family
                    ));
                } else {
                    warnings.push(format!(
                        "当前系统 ({} · {}) 不支持原生包安装，且容器环境禁止嵌套 Docker",
                        os_family, host_env.virt_label
                    ));
                }
            } else if !host_env.docker_nesting_supported
                && e.install_modes.iter().any(|m| m == "docker")
                && !e.install_modes.iter().any(|m| m == "native")
            {
                warnings.push(format!(
                    "当前为 {}，不支持嵌套运行 Docker，无法使用容器方式部署",
                    host_env.virt_label
                ));
            }
            StoreAppItem {
                id: e.id,
                name: e.name,
                category: e.category,
                description: e.description,
                min_mem_mb: e.min_mem_mb,
                supported_os: e.supported_os,
                install_modes: e.install_modes,
                installed,
                running: installed,
                version,
                has_manager: e.has_manager,
                compat_warning: if warnings.is_empty() {
                    None
                } else {
                    Some(warnings.join("；"))
                },
            }
        })
        .collect()
}

fn resolve_binary_candidates(bin: &str) -> Vec<String> {
    #[allow(unused_mut)]
    let mut list = vec![bin.to_string()];
    #[cfg(target_os = "linux")]
    {
        if bin == "uv" {
            list.push("/usr/local/bin/uv".to_string());
            list.push("/root/.local/bin/uv".to_string());
            if let Ok(home) = std::env::var("HOME") {
                list.push(format!("{}/.local/bin/uv", home.trim_end_matches('/')));
            }
        }
    }
    list
}

fn probe_binary(bin: &str, args: &[String]) -> (bool, String) {
    for candidate in resolve_binary_candidates(bin) {
        if let Ok(out) = Command::new(&candidate).args(args).output() {
            let text = if !out.stdout.is_empty() {
                String::from_utf8_lossy(&out.stdout).to_string()
            } else {
                String::from_utf8_lossy(&out.stderr).to_string()
            };
            let first_line = text.lines().next().unwrap_or("").trim().to_string();
            return (true, first_line);
        }
    }
    (false, String::new())
}

pub fn install_native_app(app_id: &str) -> Result<String, AppError> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app_id;
        return Err(AppError::BadRequest(
            "软件商店原生包安装仅支持 Debian / Ubuntu / Alpine Linux 服务器".to_string(),
        ));
    }

    #[cfg(target_os = "linux")]
    {
        if app_id == "docker" {
            return crate::docker::install_docker_engine();
        }
        if app_id == "python-uv" {
            return install_uv_linux();
        }
        let os = detect_os_family();
        let pkg = match (os.as_str(), app_id) {
            ("alpine", "nginx") => "nginx",
            ("alpine", "redis") => "redis",
            ("alpine", "postgresql") => "postgresql",
            ("alpine", "mongodb") => {
                return Err(AppError::BadRequest(
                    "Alpine Linux 官方仓库已移除 MongoDB，请前往 Docker 工作区部署 MongoDB 容器"
                        .to_string(),
                ));
            }
            (_, "nginx") => "nginx",
            (_, "redis") => "redis-server",
            (_, "postgresql") => "postgresql",
            (_, "mongodb") => {
                return Err(AppError::BadRequest(
                    "MongoDB 需按 CPU AVX 支持情况选择版本，建议通过 Docker 工作区部署以避免依赖冲突"
                        .to_string(),
                ));
            }
            _ => return Err(AppError::BadRequest(format!("未知软件标识: {}", app_id))),
        };

        let out = if os == "alpine" {
            Command::new("apk")
                .args(["add", "--no-cache", pkg])
                .output()
                .map_err(|e| AppError::Internal(format!("执行 apk 失败: {}", e)))?
        } else {
            let _ = Command::new("apt-get").args(["update"]).output();
            Command::new("apt-get")
                .args(["install", "-y", pkg])
                .env("DEBIAN_FRONTEND", "noninteractive")
                .output()
                .map_err(|e| AppError::Internal(format!("执行 apt-get 失败: {}", e)))?
        };

        let combined = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
        .trim()
        .to_string();
        if out.status.success() {
            Ok(combined)
        } else {
            Err(AppError::BadRequest(format!("安装失败:\n{}", combined)))
        }
    }
}

#[cfg(target_os = "linux")]
fn install_uv_linux() -> Result<String, AppError> {
    use flate2::read::GzDecoder;
    use std::os::unix::fs::PermissionsExt;

    let arch_triple = match std::env::consts::ARCH {
        "x86_64" => "x86_64-unknown-linux-musl",
        "aarch64" => "aarch64-unknown-linux-musl",
        other => {
            return Err(AppError::BadRequest(format!(
                "暂不支持该 CPU 架构一键安装 uv: {}",
                other
            )));
        }
    };
    let url = format!(
        "https://github.com/astral-sh/uv/releases/latest/download/uv-{}.tar.gz",
        arch_triple
    );
    let tmp_tar = std::env::temp_dir().join("zmpanel-uv-install.tar.gz");
    let _ = fs::remove_file(&tmp_tar);

    let dl = Command::new("curl")
        .args([
            "-fsSL",
            "--connect-timeout",
            "15",
            "--max-time",
            "180",
            "-o",
            &tmp_tar.to_string_lossy(),
            &url,
        ])
        .output()
        .map_err(|e| AppError::Internal(format!("调用 curl 下载 uv 失败: {}", e)))?;

    if !dl.status.success() {
        let _ = fs::remove_file(&tmp_tar);
        return Err(AppError::BadRequest(format!(
            "下载 uv 静态包失败 ({}): {}",
            url,
            String::from_utf8_lossy(&dl.stderr).trim()
        )));
    }

    let file = fs::File::open(&tmp_tar)
        .map_err(|e| AppError::Internal(format!("打开下载包失败: {}", e)))?;
    let mut archive = tar::Archive::new(GzDecoder::new(file));
    let mut installed_bins = Vec::new();
    for entry_res in archive
        .entries()
        .map_err(|e| AppError::Internal(format!("解析 uv 压缩包失败: {}", e)))?
    {
        let mut entry = entry_res.map_err(|e| AppError::Internal(e.to_string()))?;
        let path = entry
            .path()
            .map_err(|e| AppError::Internal(e.to_string()))?
            .into_owned();
        let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if fname == "uv" || fname == "uvx" {
            let dest = PathBuf::from("/usr/local/bin").join(fname);
            let mut out = fs::File::create(&dest).map_err(|e| {
                AppError::Forbidden(format!("写入 {} 失败（需 root 权限）: {}", dest.display(), e))
            })?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| AppError::Internal(format!("解压 {} 失败: {}", fname, e)))?;
            fs::set_permissions(&dest, fs::Permissions::from_mode(0o755))
                .map_err(|e| AppError::Internal(e.to_string()))?;
            installed_bins.push(dest.to_string_lossy().to_string());
        }
    }
    let _ = fs::remove_file(&tmp_tar);

    if installed_bins.is_empty() {
        return Err(AppError::Internal("未在压缩包中找到 uv 可执行文件".to_string()));
    }
    let (_, ver) = probe_binary("uv", &["--version".to_string()]);
    Ok(format!(
        "已通过官方静态包安装 uv ({}) 至 {}",
        ver,
        installed_bins.join(", ")
    ))
}

fn find_uv_bin() -> String {
    for candidate in resolve_binary_candidates("uv") {
        if Command::new(&candidate)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return candidate;
        }
    }
    "uv".to_string()
}

// ---------- Nginx Security & Config Engine ----------

pub fn render_nginx_security_conf(cfg: &NginxSecurityConfig) -> String {
    let mut s = String::from("# Generated by zmpanel Nginx Security Module (Lightweight WAF alternative)\n\n");
    if cfg.hide_version {
        s.push_str("server_tokens off;\n");
    }
    s.push_str(&format!("client_max_body_size {}m;\n\n", cfg.client_max_body_mb.clamp(1, 2048)));

    if cfg.gzip_enabled {
        s.push_str(
            "gzip on;\ngzip_vary on;\ngzip_min_length 1024;\ngzip_comp_level 5;\ngzip_types text/plain text/css application/json application/javascript text/xml application/xml image/svg+xml;\n\n",
        );
    }

    if cfg.rate_limit_enabled {
        s.push_str(&format!(
            "limit_req_zone $binary_remote_addr zone=zmpanel_cc:10m rate={}r/s;\nlimit_req zone=zmpanel_cc burst={} nodelay;\nlimit_req_status 429;\n\n",
            cfg.rate_limit_rps.clamp(1, 5000),
            cfg.rate_limit_burst.clamp(1, 10000)
        ));
    }

    if !cfg.blocked_ips.is_empty() {
        s.push_str("# IP Blacklist\n");
        for ip in &cfg.blocked_ips {
            let clean = ip.trim();
            if !clean.is_empty()
                && clean
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() || matches!(c, '.' | ':' | '/'))
            {
                s.push_str(&format!("deny {};\n", clean));
            }
        }
        s.push('\n');
    }

    if cfg.block_sensitive_paths {
        s.push_str(
            "# Block common vulnerability & secret scanners\nlocation ~* (\\.env|\\.git|\\.svn|wp-login\\.php|xmlrpc\\.php|phpmyadmin|\\.sql|\\.bak)$ {\n    return 444;\n}\n\n",
        );
    }

    if cfg.block_bad_ua {
        s.push_str(
            "# Block empty or known scanner User-Agents\nif ($http_user_agent ~* (sqlmap|nikto|nmap|masscan|acunetix|netsparker|dirbuster)) {\n    return 444;\n}\n\n",
        );
    }

    if !cfg.custom_rules.trim().is_empty() {
        s.push_str("# Custom rules\n");
        s.push_str(cfg.custom_rules.trim());
        s.push('\n');
    }
    s
}

pub fn write_nginx_security_conf(nginx_dir: &Path, cfg: &NginxSecurityConfig) -> Result<PathBuf, AppError> {
    fs::create_dir_all(nginx_dir)
        .map_err(|e| AppError::Internal(format!("创建 Nginx 配置目录失败: {}", e)))?;
    let path = nginx_dir.join("security.conf");
    fs::write(&path, render_nginx_security_conf(cfg))
        .map_err(|e| AppError::Forbidden(format!("写入 security.conf 失败: {}", e)))?;
    Ok(path)
}

pub fn nginx_control(action: &str) -> Result<String, AppError> {
    let args: Vec<&str> = match action {
        "test" => vec!["-t"],
        "reload" => vec!["-s", "reload"],
        "stop" => vec!["-s", "quit"],
        "start" => vec![],
        _ => return Err(AppError::BadRequest(format!("不支持的 Nginx 操作: {}", action))),
    };
    let out = Command::new("nginx")
        .args(&args)
        .output()
        .map_err(|e| AppError::BadRequest(format!("无法执行 nginx（请确认已安装 Nginx）: {}", e)))?;
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
    .trim()
    .to_string();
    if out.status.success() {
        Ok(if combined.is_empty() {
            format!("nginx {} 执行成功", action)
        } else {
            combined
        })
    } else {
        Err(AppError::BadRequest(format!("Nginx 执行失败:\n{}", combined)))
    }
}

// ---------- Python uv Manager ----------

pub fn uv_status() -> (bool, String, String) {
    let (ok, ver) = probe_binary("uv", &["--version".to_string()]);
    if !ok {
        return (false, String::new(), String::new());
    }
    let uv_bin = find_uv_bin();
    let py_list = Command::new(&uv_bin)
        .args(["python", "list", "--only-installed"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    (true, ver, py_list)
}

pub fn uv_install_python(version: &str) -> Result<String, AppError> {
    let clean = version.trim();
    if clean.is_empty() || !clean.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-')) {
        return Err(AppError::BadRequest("无效的 Python 版本号（例如 3.12 或 3.13）".to_string()));
    }
    let uv_bin = find_uv_bin();
    let out = Command::new(&uv_bin)
        .args(["python", "install", clean])
        .output()
        .map_err(|e| AppError::BadRequest(format!("无法调用 uv（请确认已安装 uv）: {}", e)))?;
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
    .trim()
    .to_string();
    if out.status.success() {
        Ok(combined)
    } else {
        Err(AppError::BadRequest(format!("安装 Python {} 失败:\n{}", clean, combined)))
    }
}

pub fn uv_create_venv(path_raw: &str, python_version: &str) -> Result<String, AppError> {
    let target = files::resolve_safe_path(path_raw)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| AppError::Forbidden(format!("创建虚拟环境父目录失败: {}", e)))?;
    }
    let mut args = vec!["venv".to_string(), target.to_string_lossy().to_string()];
    if !python_version.trim().is_empty() {
        args.push("--python".to_string());
        args.push(python_version.trim().to_string());
    }
    let uv_bin = find_uv_bin();
    let out = Command::new(&uv_bin)
        .args(&args)
        .output()
        .map_err(|e| AppError::BadRequest(format!("无法调用 uv 创建虚拟环境: {}", e)))?;
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
    .trim()
    .to_string();
    if out.status.success() {
        Ok(files::normalize_display_path(&target))
    } else {
        Err(AppError::BadRequest(format!("创建虚拟环境失败:\n{}", combined)))
    }
}

pub fn uv_pip_install(venv_path: &str, packages: &[String], index_url: Option<&str>) -> Result<String, AppError> {
    if packages.is_empty() {
        return Err(AppError::BadRequest("请输入至少一个包名".to_string()));
    }
    let venv_dir = files::resolve_safe_path(venv_path)?;
    #[cfg(windows)]
    let py_bin = venv_dir.join("Scripts").join("python.exe");
    #[cfg(not(windows))]
    let py_bin = venv_dir.join("bin").join("python");

    let mut args = vec![
        "pip".to_string(),
        "install".to_string(),
        "--python".to_string(),
        py_bin.to_string_lossy().to_string(),
    ];
    if let Some(url) = index_url
        && !url.trim().is_empty()
    {
        args.push("--index-url".to_string());
        args.push(url.trim().to_string());
    }
    for p in packages {
        let clean = p.trim();
        if !clean.is_empty() {
            args.push(clean.to_string());
        }
    }

    let uv_bin = find_uv_bin();
    let out = Command::new(&uv_bin)
        .args(&args)
        .output()
        .map_err(|e| AppError::BadRequest(format!("无法调用 uv pip install: {}", e)))?;
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
    .trim()
    .to_string();
    if out.status.success() {
        Ok(combined)
    } else {
        Err(AppError::BadRequest(format!("安装依赖包失败:\n{}", combined)))
    }
}

// ---------- Stage 3: Redis / PostgreSQL / MongoDB Config & Docker Compose Generators ----------

pub fn render_redis_conf(cfg: &RedisServiceConfig) -> String {
    let mut s = format!(
        "# Generated by zmpanel Store (Redis)\n\
         bind {bind}\n\
         port {port}\n\
         protected-mode yes\n\
         maxmemory {mem}mb\n\
         maxmemory-policy {policy}\n\
         appendonly {aof}\n",
        bind = cfg.bind.trim(),
        port = cfg.port,
        mem = cfg.maxmemory_mb.clamp(16, 32768),
        policy = cfg.maxmemory_policy.trim(),
        aof = if cfg.appendonly { "yes" } else { "no" },
    );
    if !cfg.password.trim().is_empty() {
        s.push_str(&format!("requirepass {}\n", cfg.password.trim()));
    }
    s
}

pub fn render_redis_compose(cfg: &RedisServiceConfig) -> String {
    let mut cmd = format!(
        "redis-server --appendonly {} --maxmemory {}mb --maxmemory-policy {}",
        if cfg.appendonly { "yes" } else { "no" },
        cfg.maxmemory_mb.clamp(16, 32768),
        cfg.maxmemory_policy.trim()
    );
    if !cfg.password.trim().is_empty() {
        cmd.push_str(&format!(" --requirepass {}", cfg.password.trim()));
    }
    format!(
        "services:\n\
         \x20 redis:\n\
         \x20   image: redis:7-alpine\n\
         \x20   container_name: zmpanel-redis\n\
         \x20   restart: unless-stopped\n\
         \x20   ports:\n\
         \x20     - \"{bind}:{port}:6379\"\n\
         \x20   volumes:\n\
         \x20     - ./data:/data\n\
         \x20   command: {cmd}\n",
        bind = cfg.bind.trim(),
        port = cfg.port,
        cmd = cmd
    )
}

pub fn render_postgres_compose(cfg: &PostgresServiceConfig) -> String {
    let pass = if cfg.password.trim().is_empty() {
        "change_me_strong_pass"
    } else {
        cfg.password.trim()
    };
    format!(
        "services:\n\
         \x20 postgres:\n\
         \x20   image: postgres:16-alpine\n\
         \x20   container_name: zmpanel-postgres\n\
         \x20   restart: unless-stopped\n\
         \x20   environment:\n\
         \x20     POSTGRES_USER: {user}\n\
         \x20     POSTGRES_PASSWORD: {pass}\n\
         \x20     POSTGRES_DB: {db}\n\
         \x20   ports:\n\
         \x20     - \"{bind}:{port}:5432\"\n\
         \x20   volumes:\n\
         \x20     - ./pgdata:/var/lib/postgresql/data\n\
         \x20   command: [\"postgres\", \"-c\", \"shared_buffers={buf}MB\", \"-c\", \"max_connections={conn}\"]\n",
        user = cfg.superuser.trim(),
        pass = pass,
        db = cfg.default_db.trim(),
        bind = cfg.bind.trim(),
        port = cfg.port,
        buf = cfg.shared_buffers_mb.clamp(16, 8192),
        conn = cfg.max_connections.clamp(10, 500),
    )
}

pub fn render_mongo_compose(cfg: &MongoServiceConfig) -> String {
    let pass = if cfg.root_password.trim().is_empty() {
        "change_me_strong_pass"
    } else {
        cfg.root_password.trim()
    };
    let tag = if cfg.image_tag.trim().is_empty() {
        "4.4"
    } else {
        cfg.image_tag.trim()
    };
    format!(
        "services:\n\
         \x20 mongodb:\n\
         \x20   image: mongo:{tag}\n\
         \x20   container_name: zmpanel-mongodb\n\
         \x20   restart: unless-stopped\n\
         \x20   environment:\n\
         \x20     MONGO_INITDB_ROOT_USERNAME: {user}\n\
         \x20     MONGO_INITDB_ROOT_PASSWORD: {pass}\n\
         \x20   ports:\n\
         \x20     - \"{bind}:{port}:27017\"\n\
         \x20   volumes:\n\
         \x20     - ./mongodata:/data/db\n\
         \x20   command: [\"--wiredTigerCacheSizeGB\", \"{cache:.2}\"]\n",
        tag = tag,
        user = cfg.root_user.trim(),
        pass = pass,
        bind = cfg.bind.trim(),
        port = cfg.port,
        cache = cfg.cache_size_gb.clamp(0.1, 16.0),
    )
}

pub fn write_service_file(store_dir: &Path, sub: &str, filename: &str, content: &str) -> Result<PathBuf, AppError> {
    let dir = store_dir.join(sub);
    fs::create_dir_all(&dir)
        .map_err(|e| AppError::Internal(format!("创建服务配置目录失败: {}", e)))?;
    let p = dir.join(filename);
    fs::write(&p, content)
        .map_err(|e| AppError::Forbidden(format!("写入服务配置失败: {}", e)))?;
    Ok(p)
}
