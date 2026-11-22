use axum::extract::ws::{Message, WebSocket};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessesToUpdate, System};
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};
use uuid::Uuid;
use zmpanel_core::{
    AgentToMasterMsg, AppError, BackupItem, ClusterJobResult, MasterBackupRecord,
    MasterToAgentMsg, PanelConfig,
};

use crate::backup::{self, CreateBackupRequest};
use crate::db::Database;
use crate::files;
use crate::state::AppState;
use crate::updater;

#[derive(Debug, Clone)]
pub struct PendingResult {
    pub success: bool,
    pub output: String,
    pub archive_b64: Option<String>,
    pub archive_sha256: Option<String>,
    pub archive_name: Option<String>,
    pub archive_summary: Option<String>,
}

/// Master-side connection hub for outbound-connected NAT Agent nodes.
#[derive(Default)]
pub struct ClusterHub {
    agents: Mutex<HashMap<String, mpsc::Sender<MasterToAgentMsg>>>,
    pending: Mutex<HashMap<String, oneshot::Sender<PendingResult>>>,
}

impl ClusterHub {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn register(&self, node_id: &str, tx: mpsc::Sender<MasterToAgentMsg>) {
        self.agents
            .lock()
            .expect("hub lock poisoned")
            .insert(node_id.to_string(), tx);
    }

    pub fn unregister(&self, node_id: &str) {
        self.agents
            .lock()
            .expect("hub lock poisoned")
            .remove(node_id);
    }

    pub fn is_online(&self, node_id: &str) -> bool {
        self.agents
            .lock()
            .expect("hub lock poisoned")
            .contains_key(node_id)
    }

    pub fn complete_pending(&self, req_id: &str, res: PendingResult) {
        if let Some(tx) = self
            .pending
            .lock()
            .expect("pending lock poisoned")
            .remove(req_id)
        {
            let _ = tx.send(res);
        }
    }

    pub async fn dispatch_rpc(
        &self,
        node_id: &str,
        request_id: String,
        msg: MasterToAgentMsg,
        timeout_secs: u64,
    ) -> Result<PendingResult, AppError> {
        let tx = self
            .agents
            .lock()
            .expect("hub lock poisoned")
            .get(node_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("节点 {} 当前离线", node_id)))?;

        let (resp_tx, resp_rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("pending lock poisoned")
            .insert(request_id.clone(), resp_tx);

        if tx.send(msg).await.is_err() {
            self.pending
                .lock()
                .expect("pending lock poisoned")
                .remove(&request_id);
            return Err(AppError::Internal(format!("向节点 {} 发送指令失败", node_id)));
        }

        match tokio::time::timeout(Duration::from_secs(timeout_secs), resp_rx).await {
            Ok(Ok(res)) => Ok(res),
            Ok(Err(_)) => Err(AppError::Internal("节点响应通道已关闭".to_string())),
            Err(_) => {
                self.pending
                    .lock()
                    .expect("pending lock poisoned")
                    .remove(&request_id);
                Err(AppError::Internal(format!(
                    "等待节点 {} 响应超时 ({}s)",
                    node_id, timeout_secs
                )))
            }
        }
    }
}

/// Master-side handler for an authenticated Agent WebSocket connection.
pub async fn handle_agent_socket(state: AppState, node_id: String, socket: WebSocket) {
    let (mut ws_tx, mut ws_rx) = socket.split();
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<MasterToAgentMsg>(64);

    state.cluster.register(&node_id, cmd_tx);
    info!("Cluster agent connected: {}", node_id);

    let send_task = tokio::spawn(async move {
        while let Some(msg) = cmd_rx.recv().await {
            if let Ok(txt) = serde_json::to_string(&msg)
                && ws_tx.send(Message::Text(txt.into())).await.is_err()
            {
                break;
            }
        }
    });

    while let Some(Ok(frame)) = ws_rx.next().await {
        match frame {
            Message::Text(txt) => {
                if let Ok(msg) = serde_json::from_str::<AgentToMasterMsg>(&txt) {
                    match msg {
                        AgentToMasterMsg::Hello {
                            hostname,
                            os_name,
                            arch,
                            version,
                            mem_total_bytes,
                            ..
                        } => {
                            let _ = state.db.update_cluster_node_hello(
                                &node_id,
                                &hostname,
                                &os_name,
                                &arch,
                                &version,
                                mem_total_bytes,
                            );
                        }
                        AgentToMasterMsg::Heartbeat {
                            cpu_usage,
                            mem_used_pct,
                            mem_total_bytes,
                            agent_rss_bytes,
                            net_rx_month,
                            net_tx_month,
                            ..
                        } => {
                            let _ = state.db.update_cluster_node_heartbeat(
                                &node_id,
                                cpu_usage,
                                mem_used_pct,
                                mem_total_bytes,
                                agent_rss_bytes,
                                net_rx_month,
                                net_tx_month,
                            );
                        }
                        AgentToMasterMsg::TaskResult {
                            request_id,
                            success,
                            output,
                            archive_b64,
                            archive_sha256,
                            archive_name,
                            archive_summary,
                        } => {
                            state.cluster.complete_pending(
                                &request_id,
                                PendingResult {
                                    success,
                                    output,
                                    archive_b64,
                                    archive_sha256,
                                    archive_name,
                                    archive_summary,
                                },
                            );
                        }
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    state.cluster.unregister(&node_id);
    let _ = state.db.set_cluster_node_offline(&node_id);
    send_task.abort();
    info!("Cluster agent disconnected: {}", node_id);
}

/// Dispatch batch command / software update / self-update concurrently to multiple nodes.
pub async fn dispatch_batch(
    state: &AppState,
    node_ids: Vec<String>,
    build_msg: impl Fn(String) -> MasterToAgentMsg + Send + Sync + 'static,
    timeout_secs: u64,
) -> Vec<ClusterJobResult> {
    let builder = Arc::new(build_msg);
    let mut handles = Vec::new();

    for nid in node_ids {
        let st = state.clone();
        let b = builder.clone();
        handles.push(tokio::spawn(async move {
            let t0 = Instant::now();
            let node_name = st
                .db
                .get_cluster_node(&nid)
                .ok()
                .flatten()
                .map(|n| n.name)
                .unwrap_or_else(|| nid.clone());
            let req_id = Uuid::new_v4().to_string();
            let msg = b(req_id.clone());
            match st.cluster.dispatch_rpc(&nid, req_id, msg, timeout_secs).await {
                Ok(res) => ClusterJobResult {
                    node_id: nid,
                    node_name,
                    success: res.success,
                    duration_ms: t0.elapsed().as_millis() as i64,
                    output: res.output,
                },
                Err(e) => ClusterJobResult {
                    node_id: nid,
                    node_name,
                    success: false,
                    duration_ms: t0.elapsed().as_millis() as i64,
                    output: e.to_string(),
                },
            }
        }));
    }

    let mut results = Vec::new();
    for h in handles {
        if let Ok(r) = h.await {
            results.push(r);
        }
    }
    results
}

/// Pull a backup archive directly from a NAT Agent over reverse WS and store in Master central repository.
pub async fn pull_backup_from_agent_rpc(
    state: &AppState,
    node_id: &str,
    name: Option<String>,
    include_websites: bool,
    include_docker: bool,
    include_ssl: bool,
    include_panel_db: bool,
    custom_dirs: Vec<String>,
) -> Result<MasterBackupRecord, AppError> {
    let node = state
        .db
        .get_cluster_node(node_id)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("节点不存在".to_string()))?;

    let req_id = Uuid::new_v4().to_string();
    let res = state
        .cluster
        .dispatch_rpc(
            node_id,
            req_id.clone(),
            MasterToAgentMsg::PullBackup {
                request_id: req_id,
                name,
                include_websites,
                include_docker,
                include_ssl,
                include_panel_db,
                custom_dirs,
            },
            180,
        )
        .await?;

    if !res.success {
        return Err(AppError::BadRequest(format!(
            "节点打包备份失败: {}",
            res.output
        )));
    }
    let b64 = res
        .archive_b64
        .ok_or_else(|| AppError::Internal("节点未回传备份归档数据".to_string()))?;
    let bytes = B64
        .decode(b64.as_bytes())
        .map_err(|e| AppError::Internal(format!("解码节点备份数据失败: {}", e)))?;

    let repo_dir = state
        .config_snapshot()
        .master_backups_dir()
        .join(node_id);
    fs::create_dir_all(&repo_dir)
        .map_err(|e| AppError::Internal(format!("创建主控备份仓库目录失败: {}", e)))?;

    let bk_id = Uuid::new_v4().to_string();
    let bk_name = res
        .archive_name
        .unwrap_or_else(|| format!("{}-{}", node.name, Utc::now().format("%Y%m%d-%H%M%S")));
    let file_path = repo_dir.join(format!("{}-{}.tar.gz", &bk_id[..8], Utc::now().format("%Y%m%d%H%M%S")));
    fs::write(&file_path, &bytes)
        .map_err(|e| AppError::Forbidden(format!("写入主控备份仓库失败: {}", e)))?;

    let actual_sha = updater::sha256_file(&file_path)?;
    if let Some(expected_sha) = res.archive_sha256
        && !expected_sha.is_empty()
        && actual_sha != expected_sha
    {
        let _ = fs::remove_file(&file_path);
        return Err(AppError::Forbidden(
            "传输校验失败：主控接收到的备份包 SHA256 与节点计算不一致".to_string(),
        ));
    }

    let record = MasterBackupRecord {
        id: bk_id,
        node_id: node.id,
        node_name: node.name,
        name: bk_name,
        file_path: files::normalize_display_path(&file_path),
        size_bytes: bytes.len() as u64,
        sha256: actual_sha,
        summary: res.archive_summary.unwrap_or_else(|| res.output),
        created_at: Utc::now().timestamp(),
    };
    state
        .db
        .insert_master_backup(&record)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(record)
}

/// Push a stored central backup from Master to any target Agent node (cross-machine migration!).
pub async fn push_restore_to_agent(
    state: &AppState,
    backup_id: &str,
    target_node_id: &str,
    restore_dir: Option<String>,
) -> Result<String, AppError> {
    let bk = state
        .db
        .get_master_backup(backup_id)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("主控仓库中未找到该备份".to_string()))?;

    let bytes = fs::read(&bk.file_path)
        .map_err(|e| AppError::NotFound(format!("读取主控备份文件失败: {}", e)))?;
    let archive_b64 = B64.encode(&bytes);

    let req_id = Uuid::new_v4().to_string();
    let res = state
        .cluster
        .dispatch_rpc(
            target_node_id,
            req_id.clone(),
            MasterToAgentMsg::PushRestore {
                request_id: req_id,
                archive_name: bk.name,
                archive_sha256: bk.sha256,
                archive_b64,
                restore_dir,
            },
            180,
        )
        .await?;

    if res.success {
        Ok(res.output)
    } else {
        Err(AppError::BadRequest(format!(
            "目标节点恢复失败: {}",
            res.output
        )))
    }
}

// =========================================================================
// Agent-side Outbound Reverse WebSocket Client (Zero inbound ports, NAT safe)
// =========================================================================

pub async fn run_agent_loop(config: PanelConfig, db: Arc<Database>) {
    let Some(master_url) = config.master_url.clone() else {
        warn!("Agent 模式缺少 --master-url 参数");
        return;
    };
    let Some(node_token) = config.node_token.clone() else {
        warn!("Agent 模式缺少 --node-token 参数");
        return;
    };

    let node_id = std::env::var("ZMPANEL_NODE_ID")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| db.get_setting("agent_node_id").ok().flatten())
        .unwrap_or_else(|| {
            let id = System::host_name().unwrap_or_else(|| Uuid::new_v4().to_string());
            let _ = db.set_setting("agent_node_id", &id);
            id
        });

    let mut backoff_secs = 2u64;
    loop {
        let sep = if master_url.contains('?') { "&" } else { "?" };
        let full_url = format!(
            "{}{}node_id={}&token={}",
            master_url.trim_end_matches('/'),
            sep,
            node_id,
            node_token
        );

        match tokio_tungstenite::connect_async(&full_url).await {
            Ok((ws_stream, _)) => {
                info!("Agent ({}) connected to Master successfully", node_id);
                backoff_secs = 2;
                run_agent_session(&config, &db, &node_id, ws_stream).await;
                warn!("Agent disconnected from Master, will reconnect...");
            }
            Err(e) => {
                warn!(
                    "Agent failed to connect to Master ({}), retry in {}s",
                    e, backoff_secs
                );
            }
        }
        tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(30);
    }
}

pub async fn run_agent_once(
    config: PanelConfig,
    db: Arc<Database>,
    node_id: String,
    full_ws_url: String,
) -> anyhow::Result<()> {
    let (ws_stream, _) = tokio_tungstenite::connect_async(&full_ws_url).await?;
    run_agent_session(&config, &db, &node_id, ws_stream).await;
    Ok(())
}

async fn run_agent_session<S>(
    config: &PanelConfig,
    db: &Arc<Database>,
    node_id: &str,
    ws_stream: tokio_tungstenite::WebSocketStream<S>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut ws_tx, mut ws_rx) = ws_stream.split();
    let (out_tx, mut out_rx) = mpsc::channel::<AgentToMasterMsg>(32);

    // Send Hello immediately
    let mut sys = System::new();
    sys.refresh_memory();
    let hello = AgentToMasterMsg::Hello {
        node_id: node_id.to_string(),
        hostname: System::host_name().unwrap_or_else(|| "agent".to_string()),
        os_name: format!(
            "{} {}",
            System::name().unwrap_or_else(|| std::env::consts::OS.to_string()),
            System::os_version().unwrap_or_default()
        )
        .trim()
        .to_string(),
        arch: std::env::consts::ARCH.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        mem_total_bytes: sys.total_memory(),
    };
    let _ = out_tx.send(hello).await;

    // Lightweight heartbeat task (every 10s, uses System::new() to stay under ~10MB RAM)
    let hb_tx = out_tx.clone();
    let hb_node = node_id.to_string();
    let hb_db = db.clone();
    let hb_task = tokio::spawn(async move {
        let mut s = System::new();
        let self_pid = Pid::from_u32(std::process::id());
        loop {
            s.refresh_cpu_usage();
            s.refresh_memory();
            s.refresh_processes(ProcessesToUpdate::Some(&[self_pid]), true);
            let total = s.total_memory();
            let used = s.used_memory();
            let mem_pct = if total > 0 {
                (used as f64 / total as f64 * 100.0) as f32
            } else {
                0.0
            };
            let rss = s.process(self_pid).map(|p| p.memory()).unwrap_or(0);
            let month_key = Utc::now().format("%Y-%m").to_string();
            let (rx_m, tx_m) = hb_db.get_monthly_traffic(&month_key).unwrap_or((0, 0));

            let hb = AgentToMasterMsg::Heartbeat {
                node_id: hb_node.clone(),
                cpu_usage: s.global_cpu_usage(),
                mem_used_pct: mem_pct,
                mem_total_bytes: total,
                agent_rss_bytes: rss,
                net_rx_month: rx_m,
                net_tx_month: tx_m,
            };
            if hb_tx.send(hb).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });

    let write_task = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if let Ok(txt) = serde_json::to_string(&msg)
                && ws_tx
                    .send(tokio_tungstenite::tungstenite::Message::Text(txt.into()))
                    .await
                    .is_err()
            {
                break;
            }
        }
    });

    while let Some(Ok(frame)) = ws_rx.next().await {
        if let tokio_tungstenite::tungstenite::Message::Text(txt) = frame
            && let Ok(cmd) = serde_json::from_str::<MasterToAgentMsg>(&txt)
        {
            let cfg_c = config.clone();
            let db_c = db.clone();
            let tx_c = out_tx.clone();
            tokio::spawn(async move {
                if let Some(reply) = handle_master_command(&cfg_c, &db_c, cmd).await {
                    let _ = tx_c.send(reply).await;
                }
            });
        }
    }

    hb_task.abort();
    write_task.abort();
}

async fn handle_master_command(
    config: &PanelConfig,
    db: &Arc<Database>,
    cmd: MasterToAgentMsg,
) -> Option<AgentToMasterMsg> {
    match cmd {
        MasterToAgentMsg::Ping => None,
        MasterToAgentMsg::ExecCommand {
            request_id,
            command,
        } => {
            let res = tokio::task::spawn_blocking(move || exec_agent_command(&command)).await;
            let (success, output) = match res {
                Ok(Ok(o)) => (true, o),
                Ok(Err(e)) => (false, e),
                Err(e) => (false, e.to_string()),
            };
            Some(AgentToMasterMsg::TaskResult {
                request_id,
                success,
                output,
                archive_b64: None,
                archive_sha256: None,
                archive_name: None,
                archive_summary: None,
            })
        }
        MasterToAgentMsg::UpdateSoftware {
            request_id,
            package,
        } => {
            let res = tokio::task::spawn_blocking(move || agent_update_software(&package)).await;
            let (success, output) = match res {
                Ok(Ok(o)) => (true, o),
                Ok(Err(e)) => (false, e),
                Err(e) => (false, e.to_string()),
            };
            Some(AgentToMasterMsg::TaskResult {
                request_id,
                success,
                output,
                archive_b64: None,
                archive_sha256: None,
                archive_name: None,
                archive_summary: None,
            })
        }
        MasterToAgentMsg::PullBackup {
            request_id,
            name,
            include_websites,
            include_docker,
            include_ssl,
            include_panel_db,
            custom_dirs,
        } => {
            let data_dir = config.data_dir.clone();
            let db_c = db.clone();
            let req_id_inner = request_id.clone();
            let res = tokio::task::spawn_blocking(move || -> Result<AgentToMasterMsg, AppError> {
                let website_ids = if include_websites {
                    db_c.list_websites()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|w| w.id)
                        .collect()
                } else {
                    vec![]
                };
                let docker_project_ids = if include_docker {
                    db_c.list_docker_projects()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|d| d.id)
                        .collect()
                } else {
                    vec![]
                };
                let ssl_cert_ids = if include_ssl {
                    db_c.list_ssl_certs()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|c| c.id)
                        .collect()
                } else {
                    vec![]
                };
                let req = CreateBackupRequest {
                    name,
                    website_ids,
                    docker_project_ids,
                    ssl_cert_ids,
                    custom_dirs,
                    include_panel_db,
                    target_repo: Some("master".to_string()),
                };
                let item = backup::create_backup_archive(&data_dir, &db_c, &req)?;
                let bytes = fs::read(&item.file_path)
                    .map_err(|e| AppError::Internal(format!("读取打包备份失败: {}", e)))?;
                Ok(AgentToMasterMsg::TaskResult {
                    request_id: req_id_inner,
                    success: true,
                    output: format!("已打包 {} ({} bytes)", item.name, item.size_bytes),
                    archive_b64: Some(B64.encode(&bytes)),
                    archive_sha256: Some(item.sha256),
                    archive_name: Some(item.name),
                    archive_summary: Some(item.summary),
                })
            })
            .await;

            match res {
                Ok(Ok(msg)) => Some(msg),
                Ok(Err(e)) => Some(AgentToMasterMsg::TaskResult {
                    request_id,
                    success: false,
                    output: e.to_string(),
                    archive_b64: None,
                    archive_sha256: None,
                    archive_name: None,
                    archive_summary: None,
                }),
                Err(e) => Some(AgentToMasterMsg::TaskResult {
                    request_id,
                    success: false,
                    output: e.to_string(),
                    archive_b64: None,
                    archive_sha256: None,
                    archive_name: None,
                    archive_summary: None,
                }),
            }
        }
        MasterToAgentMsg::PushRestore {
            request_id,
            archive_name,
            archive_sha256,
            archive_b64,
            restore_dir,
        } => {
            let data_dir = config.data_dir.clone();
            let db_c = db.clone();
            let res = tokio::task::spawn_blocking(move || -> Result<String, AppError> {
                let bytes = B64
                    .decode(archive_b64.as_bytes())
                    .map_err(|e| AppError::BadRequest(format!("解码推送备份失败: {}", e)))?;
                let recv_dir = data_dir.join("backups");
                fs::create_dir_all(&recv_dir)
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                let id = Uuid::new_v4().to_string();
                let archive_path = recv_dir.join(format!("pushed-{}.tar.gz", &id[..8]));
                fs::write(&archive_path, &bytes)
                    .map_err(|e| AppError::Internal(e.to_string()))?;

                let bk = BackupItem {
                    id,
                    name: archive_name,
                    file_path: files::normalize_display_path(&archive_path),
                    size_bytes: bytes.len() as u64,
                    sha256: archive_sha256,
                    target_repo: "master-push".to_string(),
                    scopes: vec!["主控下发恢复".to_string()],
                    summary: "主控跨机恢复".to_string(),
                    created_at: Utc::now().timestamp(),
                };
                backup::restore_backup_archive(&bk, &data_dir, &db_c, restore_dir.as_deref())
            })
            .await;

            let (success, output) = match res {
                Ok(Ok(o)) => (true, o),
                Ok(Err(e)) => (false, e.to_string()),
                Err(e) => (false, e.to_string()),
            };
            Some(AgentToMasterMsg::TaskResult {
                request_id,
                success,
                output,
                archive_b64: None,
                archive_sha256: None,
                archive_name: None,
                archive_summary: None,
            })
        }
        MasterToAgentMsg::SelfUpdate {
            request_id,
            download_url,
            sha256,
            target_version,
        } => {
            let updates_dir = config.updates_dir();
            let res = tokio::task::spawn_blocking(move || {
                updater::perform_self_update(&updates_dir, &download_url, &sha256, &target_version)
            })
            .await;
            let (success, output) = match res {
                Ok(Ok(o)) => (true, o),
                Ok(Err(e)) => (false, e.to_string()),
                Err(e) => (false, e.to_string()),
            };
            Some(AgentToMasterMsg::TaskResult {
                request_id,
                success,
                output,
                archive_b64: None,
                archive_sha256: None,
                archive_name: None,
                archive_summary: None,
            })
        }
    }
}

fn exec_agent_command(cmd_str: &str) -> Result<String, String> {
    #[cfg(windows)]
    let out = Command::new("pwsh")
        .args(["-NoLogo", "-NoProfile", "-Command", cmd_str])
        .output()
        .or_else(|_| {
            Command::new("powershell")
                .args(["-NoLogo", "-NoProfile", "-Command", cmd_str])
                .output()
        })
        .map_err(|e| format!("执行命令失败: {}", e))?;

    #[cfg(not(windows))]
    let out = Command::new("/bin/sh")
        .args(["-c", cmd_str])
        .output()
        .map_err(|e| format!("执行命令失败: {}", e))?;

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
    .trim()
    .to_string();
    if out.status.success() {
        Ok(if combined.is_empty() {
            "执行成功".to_string()
        } else {
            combined
        })
    } else {
        Err(format!(
            "退出码 {}:\n{}",
            out.status.code().unwrap_or(-1),
            combined
        ))
    }
}

/// Safe cross-distro package upgrade (e.g. batch upgrading Nginx for CVE patching across Debian/Ubuntu/Alpine).
fn agent_update_software(package: &str) -> Result<String, String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = package;
        return Err("批量软件更新仅支持 Debian / Ubuntu / Alpine Linux 节点".to_string());
    }

    #[cfg(target_os = "linux")]
    {
        let os = crate::store::detect_os_family();
        let pkg = match package {
            "nginx" => "nginx",
            "redis" => {
                if os == "alpine" {
                    "redis"
                } else {
                    "redis-server"
                }
            }
            "postgresql" => "postgresql",
            "system-security" => "",
            _ => return Err(format!("不支持的批量更新目标: {}", package)),
        };

        let out = if os == "alpine" {
            let _ = Command::new("apk").arg("update").output();
            if pkg.is_empty() {
                Command::new("apk").arg("upgrade").output()
            } else {
                Command::new("apk").args(["upgrade", pkg]).output()
            }
        } else {
            let _ = Command::new("apt-get").arg("update").output();
            if pkg.is_empty() {
                Command::new("apt-get")
                    .args(["upgrade", "-y"])
                    .env("DEBIAN_FRONTEND", "noninteractive")
                    .output()
            } else {
                Command::new("apt-get")
                    .args(["install", "--only-upgrade", "-y", pkg])
                    .env("DEBIAN_FRONTEND", "noninteractive")
                    .output()
            }
        }
        .map_err(|e| format!("执行包管理器升级失败: {}", e))?;

        let mut combined = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
        .trim()
        .to_string();

        if out.status.success() {
            if package == "nginx" {
                let test = Command::new("nginx").arg("-t").output();
                if matches!(test, Ok(o) if o.status.success()) {
                    let _ = Command::new("nginx").args(["-s", "reload"]).output();
                    combined.push_str("\n[zmpanel] nginx -t 校验通过并已热重载");
                }
            }
            Ok(combined)
        } else {
            Err(format!("升级失败:\n{}", combined))
        }
    }
}
