use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use zmpanel_core::{
    AppError, DockerContainerInfo, DockerImageInfo, DockerProject, DockerStatusResponse,
};

use crate::files;
use crate::runtime_env;

pub fn check_docker_status() -> DockerStatusResponse {
    let ver = Command::new("docker")
        .args(["version", "--format", "{{.Server.Version}}"])
        .output();

    match ver {
        Ok(out) if out.status.success() => {
            let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let compose_ok = Command::new("docker")
                .args(["compose", "version"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            let env = runtime_env::detect_runtime_env(true);
            let message = match env.docker_mode.as_str() {
                "dood" => "Docker 引擎可用（DooD 模式：通过宿主机 docker.sock 运行）".to_string(),
                "dind" => "Docker 引擎可用（容器内嵌套 DinD 模式）".to_string(),
                _ => "Docker 引擎运行正常".to_string(),
            };
            DockerStatusResponse {
                available: true,
                version: if version.is_empty() {
                    "已运行".to_string()
                } else {
                    version
                },
                compose_available: compose_ok,
                message,
                env,
            }
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let env = runtime_env::detect_runtime_env(false);
            let message = if !env.docker_nesting_supported {
                env.docker_nesting_reason.clone()
            } else if stderr.is_empty() {
                "Docker 守护进程未启动或无访问权限".to_string()
            } else {
                stderr
            };
            DockerStatusResponse {
                available: false,
                version: String::new(),
                compose_available: false,
                message,
                env,
            }
        }
        Err(_) => {
            let env = runtime_env::detect_runtime_env(false);
            let message = if !env.docker_nesting_supported {
                env.docker_nesting_reason.clone()
            } else {
                "当前服务器未安装 Docker（低内存 NAT 鸡可按需保持未安装）".to_string()
            };
            DockerStatusResponse {
                available: false,
                version: String::new(),
                compose_available: false,
                message,
                env,
            }
        }
    }
}

/// Install Docker Engine & Compose natively via distribution package manager (Debian/Ubuntu/Alpine).
/// Blocks installation on unprivileged Docker/OpenVZ/non-nesting LXC containers.
pub fn install_docker_engine() -> Result<String, AppError> {
    #[cfg(not(target_os = "linux"))]
    {
        return Err(AppError::BadRequest(
            "一键安装 Docker 引擎仅支持 Debian / Ubuntu / Alpine Linux 服务器".to_string(),
        ));
    }

    #[cfg(target_os = "linux")]
    {
        let env = runtime_env::detect_runtime_env(false);
        if !env.docker_nesting_supported {
            return Err(AppError::Forbidden(format!(
                "已拦截安装：{}（{}）",
                env.virt_label, env.docker_nesting_reason
            )));
        }

        let os = crate::store::detect_os_family();
        let mut logs = Vec::new();

        if os == "alpine" {
            let out = Command::new("apk")
                .args(["add", "--no-cache", "docker", "docker-cli-compose"])
                .output()
                .map_err(|e| AppError::Internal(format!("执行 apk 安装 Docker 失败: {}", e)))?;
            logs.push(format!(
                "{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ));
            if !out.status.success() {
                return Err(AppError::BadRequest(format!(
                    "Alpine 安装 Docker 失败:\n{}",
                    logs.join("\n").trim()
                )));
            }
            let _ = Command::new("rc-update").args(["add", "docker", "default"]).output();
            let _ = Command::new("rc-service").args(["docker", "start"]).output();
        } else {
            let _ = Command::new("apt-get")
                .args(["update", "-qq"])
                .env("DEBIAN_FRONTEND", "noninteractive")
                .output();
            let out = Command::new("apt-get")
                .args(["install", "-y", "docker.io"])
                .env("DEBIAN_FRONTEND", "noninteractive")
                .output()
                .map_err(|e| AppError::Internal(format!("执行 apt-get 安装 docker.io 失败: {}", e)))?;
            logs.push(format!(
                "{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ));
            if !out.status.success() {
                return Err(AppError::BadRequest(format!(
                    "安装 docker.io 失败:\n{}",
                    logs.join("\n").trim()
                )));
            }

            // Best-effort install Compose plugin (Debian 12/13 & Ubuntu package names vary across releases)
            for compose_pkg in ["docker-compose-plugin", "docker-compose-v2", "docker-compose"] {
                let c_out = Command::new("apt-get")
                    .args(["install", "-y", compose_pkg])
                    .env("DEBIAN_FRONTEND", "noninteractive")
                    .output();
                if let Ok(o) = c_out
                    && o.status.success()
                {
                    logs.push(format!("[zmpanel] 已附带安装 {}", compose_pkg));
                    break;
                }
            }

            let _ = Command::new("systemctl")
                .args(["enable", "--now", "docker"])
                .output();
        }

        let st = check_docker_status();
        logs.push(format!(
            "[zmpanel] 安装完成检测: available={}, version={}, compose={}",
            st.available, st.version, st.compose_available
        ));
        Ok(logs.join("\n").trim().to_string())
    }
}

pub fn list_containers(include_all: bool) -> Result<Vec<DockerContainerInfo>, AppError> {
    let mut args = vec!["ps", "--format", "{{json .}}"];
    if include_all {
        args.insert(1, "-a");
    }
    let out = run_docker(&args, None)?;

    // Fetch live resource stats if available (best effort for running containers)
    let mut stats_map: HashMap<String, (String, String)> = HashMap::new();
    if let Ok(stats_out) = Command::new("docker")
        .args(["stats", "--no-stream", "--format", "{{json .}}"])
        .output()
        && stats_out.status.success()
    {
        let txt = String::from_utf8_lossy(&stats_out.stdout);
        for line in txt.lines() {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                let id = v["ID"].as_str().unwrap_or("").to_string();
                let name = v["Name"].as_str().unwrap_or("").to_string();
                let cpu = v["CPUPerc"].as_str().unwrap_or("0.00%").to_string();
                let mem = v["MemUsage"].as_str().unwrap_or("-").to_string();
                if !id.is_empty() {
                    stats_map.insert(id, (cpu.clone(), mem.clone()));
                }
                if !name.is_empty() {
                    stats_map.insert(name, (cpu, mem));
                }
            }
        }
    }

    let mut list = Vec::new();
    for line in out.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            let id = v["ID"].as_str().unwrap_or("").to_string();
            let name = v["Names"].as_str().unwrap_or("").to_string();
            let (cpu_pct, mem_usage) = stats_map
                .get(&id)
                .or_else(|| stats_map.get(&name))
                .cloned()
                .unwrap_or_else(|| ("-".to_string(), "-".to_string()));
            list.push(DockerContainerInfo {
                id,
                name,
                image: v["Image"].as_str().unwrap_or("").to_string(),
                state: v["State"].as_str().unwrap_or("").to_string(),
                status: v["Status"].as_str().unwrap_or("").to_string(),
                ports: v["Ports"].as_str().unwrap_or("").to_string(),
                created: v["CreatedAt"].as_str().unwrap_or("").to_string(),
                cpu_pct,
                mem_usage,
            });
        }
    }
    Ok(list)
}

pub fn container_action(container_id: &str, action: &str) -> Result<String, AppError> {
    validate_docker_id(container_id)?;
    let args: Vec<&str> = match action {
        "start" => vec!["start", container_id],
        "stop" => vec!["stop", container_id],
        "restart" => vec!["restart", container_id],
        "remove" => vec!["rm", "-f", container_id],
        _ => return Err(AppError::BadRequest(format!("不支持的容器动作: {}", action))),
    };
    run_docker(&args, None)
}

pub fn container_logs(container_id: &str, tail: usize) -> Result<String, AppError> {
    validate_docker_id(container_id)?;
    let tail_str = tail.clamp(10, 2000).to_string();
    let out = Command::new("docker")
        .args(["logs", "--tail", &tail_str, "--timestamps", container_id])
        .output()
        .map_err(|e| AppError::Internal(format!("执行 docker logs 失败: {}", e)))?;
    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(&out.stdout));
    if !out.stderr.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&String::from_utf8_lossy(&out.stderr));
    }
    Ok(combined)
}

pub fn list_images() -> Result<Vec<DockerImageInfo>, AppError> {
    let out = run_docker(&["images", "--format", "{{json .}}"], None)?;
    let mut list = Vec::new();
    for line in out.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            list.push(DockerImageInfo {
                id: v["ID"].as_str().unwrap_or("").to_string(),
                repository: v["Repository"].as_str().unwrap_or("").to_string(),
                tag: v["Tag"].as_str().unwrap_or("").to_string(),
                size: v["Size"].as_str().unwrap_or("").to_string(),
                created: v["CreatedSince"].as_str().unwrap_or("").to_string(),
            });
        }
    }
    Ok(list)
}

pub fn remove_image(image_id: &str) -> Result<String, AppError> {
    validate_docker_id(image_id)?;
    run_docker(&["rmi", "-f", image_id], None)
}

pub fn prune_system() -> Result<String, AppError> {
    run_docker(&["system", "prune", "-f"], None)
}

pub fn init_project_workspace(
    default_base: &Path,
    name: &str,
    custom_dir: Option<&str>,
    compose_content: Option<&str>,
    env_content: Option<&str>,
) -> Result<PathBuf, AppError> {
    let dir = match custom_dir {
        Some(d) if !d.trim().is_empty() => files::resolve_safe_path(d)?,
        _ => {
            let safe_name: String = name
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                        c
                    } else {
                        '-'
                    }
                })
                .collect();
            let slug = safe_name.trim_matches('-');
            let folder = if slug.is_empty() { "project" } else { slug };
            default_base.join(folder)
        }
    };

    fs::create_dir_all(&dir)
        .map_err(|e| AppError::Forbidden(format!("创建 Docker 项目目录失败: {}", e)))?;

    let compose_path = dir.join("docker-compose.yml");
    if let Some(content) = compose_content {
        fs::write(&compose_path, content)
            .map_err(|e| AppError::Forbidden(format!("写入 docker-compose.yml 失败: {}", e)))?;
    } else if !compose_path.exists() {
        let template = format!(
            "services:\n  {name}:\n    image: alpine:latest\n    container_name: {name}\n    restart: unless-stopped\n    command: [\"sleep\", \"infinity\"]\n",
            name = name.to_lowercase().replace(' ', "-")
        );
        fs::write(&compose_path, template)
            .map_err(|e| AppError::Forbidden(format!("写入默认 compose 模板失败: {}", e)))?;
    }

    if let Some(env_txt) = env_content {
        fs::write(dir.join(".env"), env_txt)
            .map_err(|e| AppError::Forbidden(format!("写入 .env 失败: {}", e)))?;
    }

    Ok(dir)
}

pub fn run_compose_action(project: &DockerProject, action: &str) -> Result<String, AppError> {
    let work_dir = files::resolve_safe_path(&project.work_dir)?;
    if !work_dir.is_dir() {
        return Err(AppError::NotFound(format!(
            "项目目录不存在: {}",
            project.work_dir
        )));
    }
    let compose_file = if project.compose_file.trim().is_empty() {
        "docker-compose.yml"
    } else {
        project.compose_file.trim()
    };

    let args: Vec<&str> = match action {
        "up" => vec!["compose", "-f", compose_file, "up", "-d"],
        "up_build" => vec!["compose", "-f", compose_file, "up", "-d", "--build"],
        "pull" => vec!["compose", "-f", compose_file, "pull"],
        "down" => vec!["compose", "-f", compose_file, "down"],
        "restart" => vec!["compose", "-f", compose_file, "restart"],
        "ps" => vec!["compose", "-f", compose_file, "ps"],
        "logs" => vec!["compose", "-f", compose_file, "logs", "--tail", "200"],
        _ => {
            return Err(AppError::BadRequest(format!(
                "不支持的 Compose 动作: {}",
                action
            )));
        }
    };
    run_docker(&args, Some(&work_dir))
}

/// Safely execute a saved `docker ...` deployment command without invoking a shell.
pub fn run_project_deploy_cmd(project: &DockerProject) -> Result<String, AppError> {
    let work_dir = files::resolve_safe_path(&project.work_dir)?;
    if !work_dir.is_dir() {
        return Err(AppError::NotFound(format!(
            "项目目录不存在: {}",
            project.work_dir
        )));
    }
    let tokens = split_command_args(&project.deploy_cmd)?;
    if tokens.is_empty() {
        return Err(AppError::BadRequest("部署命令不能为空".to_string()));
    }
    let bin = tokens[0].to_lowercase();
    if bin != "docker" && bin != "docker-compose" {
        return Err(AppError::Forbidden(
            "一键部署仅允许以 docker 或 docker-compose 开头的命令（复杂脚本请直接点击「终端」执行）"
                .to_string(),
        ));
    }
    let args: Vec<&str> = tokens[1..].iter().map(|s| s.as_str()).collect();
    let out = Command::new(&tokens[0])
        .args(&args)
        .current_dir(&work_dir)
        .output()
        .map_err(|e| AppError::Internal(format!("执行命令失败: {}", e)))?;

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{}{}", stdout, if stderr.is_empty() { String::new() } else { format!("\n{}", stderr) })
        .trim()
        .to_string();

    if out.status.success() {
        Ok(if combined.is_empty() {
            "命令执行成功".to_string()
        } else {
            combined
        })
    } else {
        Err(AppError::BadRequest(format!(
            "命令返回错误 (exit {}):\n{}",
            out.status.code().unwrap_or(-1),
            combined
        )))
    }
}

fn run_docker(args: &[&str], cwd: Option<&Path>) -> Result<String, AppError> {
    let mut cmd = Command::new("docker");
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let out = cmd
        .output()
        .map_err(|e| AppError::BadRequest(format!("无法调用 docker 命令（请确认已安装 Docker）: {}", e)))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    if out.status.success() {
        Ok(if stdout.trim().is_empty() && !stderr.trim().is_empty() {
            stderr
        } else {
            stdout
        })
    } else {
        Err(AppError::BadRequest(format!(
            "Docker 执行失败: {}",
            if stderr.trim().is_empty() {
                stdout.trim()
            } else {
                stderr.trim()
            }
        )))
    }
}

fn validate_docker_id(id: &str) -> Result<(), AppError> {
    let trimmed = id.trim();
    if trimmed.is_empty()
        || trimmed.len() > 128
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | '/'))
    {
        return Err(AppError::BadRequest("无效的容器或镜像标识符".to_string()));
    }
    Ok(())
}

pub fn split_command_args(input: &str) -> Result<Vec<String>, AppError> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '\\' if in_double => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            c if c.is_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if in_single || in_double {
        return Err(AppError::BadRequest("命令行引号未闭合".to_string()));
    }
    if !current.is_empty() {
        args.push(current);
    }
    Ok(args)
}
