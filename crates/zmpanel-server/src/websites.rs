use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use zmpanel_core::{AppError, SslCertificate, WebsiteKind, WebsiteRecord};

use crate::docker::split_command_args;
use crate::files;

struct ManagedProc {
    child: Child,
    pid: u32,
    restart_count: u32,
}

/// Built-in cross-platform lightweight process supervisor for Rust / Go / Python web apps.
/// Works seamlessly on Debian (systemd), Alpine (OpenRC), LXC/OpenVZ NAT VPS without systemd, and Windows.
#[derive(Default)]
pub struct ProcessSupervisor {
    procs: Mutex<HashMap<String, ManagedProc>>,
}

#[derive(Debug, Clone)]
pub struct RuntimeStatus {
    pub status: String,
    pub pid: Option<u32>,
    pub restart_count: u32,
}

impl ProcessSupervisor {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn get_status(&self, site_id: &str, kind: &WebsiteKind) -> RuntimeStatus {
        if matches!(kind, WebsiteKind::Proxy | WebsiteKind::Static) {
            return RuntimeStatus {
                status: "nginx".to_string(),
                pid: None,
                restart_count: 0,
            };
        }
        let mut guard = self.procs.lock().expect("supervisor poisoned");
        if let Some(proc_info) = guard.get_mut(site_id) {
            match proc_info.child.try_wait() {
                Ok(None) => {
                    return RuntimeStatus {
                        status: "running".to_string(),
                        pid: Some(proc_info.pid),
                        restart_count: proc_info.restart_count,
                    };
                }
                Ok(Some(_)) | Err(_) => {
                    let rc = proc_info.restart_count;
                    guard.remove(site_id);
                    return RuntimeStatus {
                        status: "exited".to_string(),
                        pid: None,
                        restart_count: rc,
                    };
                }
            }
        }
        RuntimeStatus {
            status: "stopped".to_string(),
            pid: None,
            restart_count: 0,
        }
    }

    pub fn start_site(&self, site: &WebsiteRecord, logs_dir: &Path) -> Result<u32, AppError> {
        if matches!(site.kind, WebsiteKind::Proxy | WebsiteKind::Static) {
            return Err(AppError::BadRequest(
                "静态站点和纯反向代理站点由 Nginx 托管，无需启动独立后端进程".to_string(),
            ));
        }

        let _ = self.stop_site(&site.id);

        let tokens = split_command_args(&site.start_cmd)?;
        if tokens.is_empty() {
            return Err(AppError::BadRequest("请先填写启动命令".to_string()));
        }

        let work_dir = files::resolve_safe_path(&site.root_dir)?;
        if !work_dir.is_dir() {
            return Err(AppError::NotFound(format!(
                "项目根目录不存在: {}",
                site.root_dir
            )));
        }

        let site_logs_dir = logs_dir.join("sites");
        fs::create_dir_all(&site_logs_dir)
            .map_err(|e| AppError::Internal(format!("创建日志目录失败: {}", e)))?;
        let log_path = site_logs_dir.join(format!("{}.log", site.id));
        let stdout_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|e| AppError::Internal(format!("打开项目日志文件失败: {}", e)))?;
        let stderr_file = stdout_file
            .try_clone()
            .map_err(|e| AppError::Internal(format!("克隆日志句柄失败: {}", e)))?;

        let mut program = tokens[0].clone();
        let args = &tokens[1..];

        // Resolve Python virtual environment executable / PATH when kind == Python
        let mut extra_path_prefix: Option<PathBuf> = None;
        if site.kind == WebsiteKind::Python
            && let Some(ref venv) = site.python_venv
            && !venv.trim().is_empty()
        {
            let venv_path = PathBuf::from(venv.trim());
            #[cfg(windows)]
            let bin_dir = venv_path.join("Scripts");
            #[cfg(not(windows))]
            let bin_dir = venv_path.join("bin");

            if bin_dir.is_dir() {
                let candidate = bin_dir.join(&program);
                if candidate.is_file() {
                    program = candidate.to_string_lossy().to_string();
                } else {
                    #[cfg(windows)]
                    {
                        let exe_cand = bin_dir.join(format!("{}.exe", program));
                        if exe_cand.is_file() {
                            program = exe_cand.to_string_lossy().to_string();
                        }
                    }
                }
                extra_path_prefix = Some(bin_dir);
            }
        }

        let mut cmd = Command::new(&program);
        cmd.args(args)
            .current_dir(&work_dir)
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file));

        if let Some(bin_dir) = extra_path_prefix {
            let cur_path = std::env::var_os("PATH").unwrap_or_default();
            let mut paths = vec![bin_dir];
            paths.extend(std::env::split_paths(&cur_path));
            if let Ok(joined) = std::env::join_paths(paths) {
                cmd.env("PATH", joined);
            }
            if let Some(ref venv) = site.python_venv {
                cmd.env("VIRTUAL_ENV", venv);
            }
        }

        for line in site.env_vars.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = trimmed.split_once('=') {
                let key = k.trim();
                if !key.is_empty() {
                    cmd.env(key, v.trim());
                }
            }
        }
        cmd.env("PORT", site.listen_port.to_string());

        let child = cmd
            .spawn()
            .map_err(|e| AppError::BadRequest(format!("启动项目进程失败 ({}): {}", program, e)))?;
        let pid = child.id();

        let mut guard = self.procs.lock().expect("supervisor poisoned");
        guard.insert(
            site.id.clone(),
            ManagedProc {
                child,
                pid,
                restart_count: 0,
            },
        );
        Ok(pid)
    }

    pub fn stop_site(&self, site_id: &str) -> Result<(), AppError> {
        let mut guard = self.procs.lock().expect("supervisor poisoned");
        if let Some(mut proc_info) = guard.remove(site_id) {
            let _ = proc_info.child.kill();
            let _ = proc_info.child.wait();
        }
        Ok(())
    }
}

pub fn read_site_logs(logs_dir: &Path, site_id: &str, max_lines: usize) -> Result<String, AppError> {
    let log_path = logs_dir.join("sites").join(format!("{}.log", site_id));
    if !log_path.exists() {
        return Ok("暂无运行日志输出".to_string());
    }
    let content = fs::read_to_string(&log_path)
        .map_err(|e| AppError::Internal(format!("读取站点日志失败: {}", e)))?;
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(max_lines.clamp(20, 2000));
    Ok(lines[start..].join("\n"))
}

pub fn generate_nginx_config(site: &WebsiteRecord, ssl_cert: Option<&SslCertificate>) -> String {
    if let Some(ref custom) = site.custom_nginx
        && !custom.trim().is_empty()
    {
        return custom.clone();
    }

    let server_names = if site.domains.is_empty() {
        "_".to_string()
    } else {
        site.domains.join(" ")
    };

    let location_block = match site.kind {
        WebsiteKind::Static => {
            let try_line = if site.spa_fallback {
                "        try_files $uri $uri/ /index.html;"
            } else {
                "        try_files $uri $uri/ =404;"
            };
            format!(
                "    root {};\n    index index.html index.htm;\n\n    location / {{\n{}\n    }}",
                site.root_dir, try_line
            )
        }
        WebsiteKind::Proxy | WebsiteKind::Rust | WebsiteKind::Go | WebsiteKind::Python => {
            let upstream = match site.kind {
                WebsiteKind::Proxy => site
                    .proxy_target
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| format!("http://127.0.0.1:{}", site.listen_port)),
                _ => format!("http://127.0.0.1:{}", site.listen_port),
            };
            let ws_lines = if site.websocket_enabled {
                "\n        proxy_http_version 1.1;\n        proxy_set_header Upgrade $http_upgrade;\n        proxy_set_header Connection \"upgrade\";"
            } else {
                ""
            };
            format!(
                "    location / {{\n        proxy_pass {upstream};{ws}\n        proxy_set_header Host $host;\n        proxy_set_header X-Real-IP $remote_addr;\n        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n        proxy_set_header X-Forwarded-Proto $scheme;\n        proxy_read_timeout 120s;\n    }}",
                upstream = upstream,
                ws = ws_lines
            )
        }
    };

    if let Some(cert) = ssl_cert {
        let mut out = String::new();
        if site.force_https {
            out.push_str(&format!(
                "server {{\n    listen 80;\n    listen [::]:80;\n    server_name {server_names};\n    return 301 https://$host$request_uri;\n}}\n\n"
            ));
        }
        out.push_str(&format!(
            "# Managed by zmpanel ({kind} site: {name})\nserver {{\n    listen 443 ssl http2;\n    listen [::]:443 ssl http2;\n    server_name {server_names};\n\n    ssl_certificate {cert_path};\n    ssl_certificate_key {key_path};\n    ssl_protocols TLSv1.2 TLSv1.3;\n    ssl_ciphers HIGH:!aNULL:!MD5;\n\n{location_block}\n}}\n",
            kind = site.kind.as_str(),
            name = site.name,
            cert_path = cert.cert_path,
            key_path = cert.key_path,
        ));
        out
    } else {
        format!(
            "# Managed by zmpanel ({kind} site: {name})\nserver {{\n    listen 80;\n    listen [::]:80;\n    server_name {server_names};\n\n{location_block}\n}}\n",
            kind = site.kind.as_str(),
            name = site.name,
        )
    }
}

pub fn write_site_nginx_conf(
    nginx_sites_dir: &Path,
    site: &WebsiteRecord,
    ssl_cert: Option<&SslCertificate>,
) -> Result<PathBuf, AppError> {
    fs::create_dir_all(nginx_sites_dir)
        .map_err(|e| AppError::Internal(format!("创建 Nginx 站点目录失败: {}", e)))?;
    let path = nginx_sites_dir.join(format!("{}.conf", site.id));
    let content = generate_nginx_config(site, ssl_cert);
    fs::write(&path, content)
        .map_err(|e| AppError::Forbidden(format!("写入 Nginx 站点配置失败: {}", e)))?;
    Ok(path)
}
