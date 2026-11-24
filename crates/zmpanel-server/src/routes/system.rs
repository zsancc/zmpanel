use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::Utc;
use serde::Deserialize;
use zmpanel_core::{
    ApiResponse, AppError, AuditLogItem, MetricPoint, NodeSummary, ProcessItem, SystemSnapshot,
};

use crate::auth::AuthUser;
use crate::error::ServerResult;
use crate::monitor;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct MetricsQuery {
    pub hours: Option<i64>,
    pub max_points: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct ProcessQuery {
    pub sort: Option<String>,
    pub q: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct SwapRequest {
    pub size_mb: u64,
}

#[derive(Debug, Deserialize)]
pub struct PowerActionRequest {
    /// "panel" | "server"
    pub target: String,
}

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    pub limit: Option<usize>,
}

pub async fn get_snapshot(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<SystemSnapshot>>> {
    let snap = state
        .latest_snapshot
        .read()
        .expect("snapshot lock poisoned")
        .clone()
        .ok_or_else(|| AppError::Internal("监控数据正在初始化，请稍后重试".to_string()))?;
    Ok(Json(ApiResponse::success(snap)))
}

pub async fn get_metrics(
    State(state): State<AppState>,
    _auth: AuthUser,
    Query(query): Query<MetricsQuery>,
) -> ServerResult<Json<ApiResponse<Vec<MetricPoint>>>> {
    let hours = query.hours.unwrap_or(6).clamp(1, 168);
    let max_points = query.max_points.unwrap_or(180).clamp(20, 600);
    let since = Utc::now().timestamp() - hours * 3600;
    let points = state.db.query_metrics(since, max_points)?;
    Ok(Json(ApiResponse::success(points)))
}

pub async fn get_processes(
    _auth: AuthUser,
    Query(query): Query<ProcessQuery>,
) -> ServerResult<Json<ApiResponse<Vec<ProcessItem>>>> {
    let sort = query.sort.unwrap_or_else(|| "mem".to_string());
    let limit = query.limit.unwrap_or(80);
    let q = query.q;
    let list = tokio::task::spawn_blocking(move || {
        monitor::list_processes(&sort, q.as_deref(), limit)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(ApiResponse::success(list)))
}

pub async fn kill_process(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(pid): Path<u32>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    tokio::task::spawn_blocking(move || monitor::kill_process(pid))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "system.kill_process",
        &format!("pid:{}", pid),
        "手动终止进程",
    );
    Ok(Json(ApiResponse::success(true)))
}

pub async fn configure_swap(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<SwapRequest>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let size_mb = payload.size_mb;
    let msg = tokio::task::spawn_blocking(move || monitor::configure_swap_file(size_mb))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "system.swap",
        "swapfile",
        &msg,
    );
    Ok(Json(ApiResponse::success(msg)))
}

pub async fn power_action(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<PowerActionRequest>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let target = payload.target.trim().to_lowercase();
    match target.as_str() {
        "panel" => {
            let _ = state.db.write_audit(
                &auth.user.username,
                &auth.client_ip,
                "system.restart_panel",
                "zmpanel",
                "触发重启面板服务",
            );
            tokio::spawn(async {
                tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                #[cfg(target_os = "linux")]
                {
                    use std::process::Command;
                    let is_systemd = Command::new("systemctl")
                        .args(["is-active", "--quiet", "zmpanel"])
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false);
                    if is_systemd {
                        let _ = Command::new("systemctl")
                            .args(["restart", "zmpanel"])
                            .status();
                        return;
                    }
                    let is_openrc = Command::new("rc-service")
                        .args(["zmpanel", "status"])
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false);
                    if is_openrc {
                        let _ = Command::new("rc-service")
                            .args(["zmpanel", "restart"])
                            .status();
                        return;
                    }
                    // Fallback: exit cleanly so supervisor (systemd Restart=always / container) restarts it
                    std::process::exit(0);
                }
                #[cfg(not(target_os = "linux"))]
                {
                    std::process::exit(0);
                }
            });
            Ok(Json(ApiResponse::success(
                "已下发重启面板指令，约 2~3 秒后自动恢复连接".to_string(),
            )))
        }
        "server" => {
            #[cfg(not(target_os = "linux"))]
            {
                return Err(AppError::BadRequest(
                    "重启服务器仅支持 Linux 生产环境（Debian / Ubuntu / Alpine）".to_string(),
                )
                .into());
            }
            #[cfg(target_os = "linux")]
            {
                let _ = state.db.write_audit(
                    &auth.user.username,
                    &auth.client_ip,
                    "system.reboot_server",
                    "host",
                    "触发重启服务器系统",
                );
                tokio::spawn(async {
                    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
                    use std::process::Command;
                    let ok = Command::new("systemctl")
                        .arg("reboot")
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false);
                    if !ok {
                        let _ = Command::new("reboot").status();
                    }
                });
                Ok(Json(ApiResponse::success(
                    "已下发服务器系统重启指令，预计 30~60 秒后恢复访问".to_string(),
                )))
            }
        }
        _ => Err(AppError::BadRequest("target 仅支持 panel 或 server".to_string()).into()),
    }
}

pub async fn get_audit_logs(
    State(state): State<AppState>,
    _auth: AuthUser,
    Query(query): Query<AuditQuery>,
) -> ServerResult<Json<ApiResponse<Vec<AuditLogItem>>>> {
    let limit = query.limit.unwrap_or(100).clamp(10, 500);
    let logs = state.db.list_audit_logs(limit)?;
    Ok(Json(ApiResponse::success(logs)))
}

pub async fn list_nodes(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<Vec<NodeSummary>>>> {
    let cfg = state.config_snapshot();
    let snap = state
        .latest_snapshot
        .read()
        .expect("snapshot lock poisoned")
        .clone();

    let local = match snap {
        Some(s) => {
            let mem_pct = if s.mem_total > 0 {
                (s.mem_used as f64 / s.mem_total as f64 * 100.0) as f32
            } else {
                0.0
            };
            NodeSummary {
                id: "local".to_string(),
                name: format!("本机 ({})", s.hostname),
                mode: format!("{:?}", cfg.mode).to_lowercase(),
                status: "online".to_string(),
                os_name: format!("{} {}", s.os_name, s.os_version),
                arch: s.arch,
                cpu_usage: s.cpu_usage,
                mem_used_pct: mem_pct,
                last_seen: s.timestamp,
                is_local: true,
            }
        }
        None => NodeSummary {
            id: "local".to_string(),
            name: "本机节点".to_string(),
            mode: format!("{:?}", cfg.mode).to_lowercase(),
            status: "online".to_string(),
            os_name: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            cpu_usage: 0.0,
            mem_used_pct: 0.0,
            last_seen: Utc::now().timestamp(),
            is_local: true,
        },
    };
    let mut all_nodes = vec![local];
    if let Ok(remote_nodes) = state.db.list_cluster_nodes() {
        for n in remote_nodes {
            let status = if state.cluster.is_online(&n.id) {
                "online".to_string()
            } else {
                "offline".to_string()
            };
            all_nodes.push(NodeSummary {
                id: n.id,
                name: format!("[{}] {}", n.group_name, n.name),
                mode: "agent".to_string(),
                status,
                os_name: if n.os_name.is_empty() {
                    "等待接入".to_string()
                } else {
                    n.os_name
                },
                arch: if n.arch.is_empty() { "-".to_string() } else { n.arch },
                cpu_usage: n.cpu_usage,
                mem_used_pct: n.mem_used_pct,
                last_seen: n.last_seen,
                is_local: false,
            });
        }
    }
    Ok(Json(ApiResponse::success(all_nodes)))
}
