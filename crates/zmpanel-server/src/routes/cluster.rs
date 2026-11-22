use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State, WebSocketUpgrade},
    http::{HeaderValue, StatusCode, header},
    response::Response,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio_util::io::ReaderStream;
use uuid::Uuid;
use zmpanel_core::{
    ApiResponse, AppError, ClusterJobRecord, ClusterNodeRecord, MasterBackupRecord,
    MasterToAgentMsg, NotifyConfig, UpdateManifest,
};

use crate::auth::{AuthUser, generate_alnum, generate_random_token, sha256_hex};
use crate::cluster;
use crate::error::ServerResult;
use crate::notify;
use crate::state::AppState;
use crate::updater;

#[derive(Debug, Deserialize)]
pub struct AgentWsQuery {
    pub node_id: String,
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateNodeRequest {
    pub name: String,
    pub group_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateNodeResponse {
    pub node: ClusterNodeRecord,
    pub node_token: String,
    pub agent_ws_path: String,
}

#[derive(Debug, Deserialize)]
pub struct BatchExecRequest {
    pub node_ids: Vec<String>,
    pub command: String,
}

#[derive(Debug, Deserialize)]
pub struct BatchUpdateSoftwareRequest {
    pub node_ids: Vec<String>,
    pub package: String, // nginx / redis / postgresql / system-security
}

#[derive(Debug, Deserialize)]
pub struct BatchSelfUpdateRequest {
    pub node_ids: Vec<String>,
    pub download_url: String,
    pub sha256: String,
    pub target_version: String,
}

#[derive(Debug, Deserialize)]
pub struct PullBackupRequest {
    pub name: Option<String>,
    pub include_websites: Option<bool>,
    pub include_docker: Option<bool>,
    pub include_ssl: Option<bool>,
    pub include_panel_db: Option<bool>,
    pub custom_dirs: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct PushRestoreRequest {
    pub target_node_id: String,
    pub restore_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CheckUpdateRequest {
    pub manifest_url: String,
}

#[derive(Debug, Deserialize)]
pub struct ApplySelfUpdateRequest {
    pub download_url: String,
    pub sha256: String,
    pub target_version: String,
}

/// Outbound Agent reverse WebSocket endpoint (authenticated by node_id + node_token hash).
pub async fn agent_ws(
    State(state): State<AppState>,
    Query(query): Query<AgentWsQuery>,
    ws: WebSocketUpgrade,
) -> ServerResult<Response> {
    let token_hash = sha256_hex(query.token.trim());
    let verified = state
        .db
        .verify_cluster_node_token(query.node_id.trim(), &token_hash)?;
    if verified.is_none() {
        return Err(AppError::Unauthorized.into());
    }
    let node_id = query.node_id.trim().to_string();
    Ok(ws.on_upgrade(move |socket| cluster::handle_agent_socket(state, node_id, socket)))
}

pub async fn list_nodes(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<Vec<ClusterNodeRecord>>>> {
    let mut nodes = state.db.list_cluster_nodes()?;
    for n in &mut nodes {
        n.status = if state.cluster.is_online(&n.id) {
            "online".to_string()
        } else {
            "offline".to_string()
        };
    }
    Ok(Json(ApiResponse::success(nodes)))
}

pub async fn create_node(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<CreateNodeRequest>,
) -> ServerResult<Json<ApiResponse<CreateNodeResponse>>> {
    let name = payload.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("节点名称不能为空".to_string()).into());
    }
    let id = format!("node_{}", generate_alnum(8));
    let raw_token = generate_random_token(24);
    let token_hash = sha256_hex(&raw_token);
    let group_name = payload
        .group_name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "默认分组".to_string());

    let node = state
        .db
        .create_cluster_node(&id, &name, &group_name, &token_hash)?;
    let entry = state.current_entry();
    let agent_ws_path = format!("{}/api/agent/ws", entry);

    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "cluster.node_create",
        &node.name,
        &format!("id={}", node.id),
    );
    Ok(Json(ApiResponse::success(CreateNodeResponse {
        node,
        node_token: raw_token,
        agent_ws_path,
    })))
}

pub async fn delete_node(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    state.cluster.unregister(&id);
    state.db.delete_cluster_node(&id)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "cluster.node_delete",
        &id,
        "移除受控节点",
    );
    Ok(Json(ApiResponse::success(true)))
}

pub async fn batch_exec(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<BatchExecRequest>,
) -> ServerResult<Json<ApiResponse<ClusterJobRecord>>> {
    if payload.node_ids.is_empty() {
        return Err(AppError::BadRequest("请至少选择一个目标节点".to_string()).into());
    }
    if payload.command.trim().is_empty() {
        return Err(AppError::BadRequest("下发命令不能为空".to_string()).into());
    }
    let cmd_str = payload.command.clone();
    let results = cluster::dispatch_batch(
        &state,
        payload.node_ids,
        move |req_id| MasterToAgentMsg::ExecCommand {
            request_id: req_id,
            command: cmd_str.clone(),
        },
        90,
    )
    .await;

    let job = ClusterJobRecord {
        id: Uuid::new_v4().to_string(),
        action: "batch_exec".to_string(),
        payload_summary: payload.command,
        created_at: Utc::now().timestamp(),
        results,
    };
    state.db.insert_cluster_job(&job)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "cluster.batch_exec",
        &format!("{} 台节点", job.results.len()),
        &job.payload_summary,
    );
    Ok(Json(ApiResponse::success(job)))
}

pub async fn batch_update_software(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<BatchUpdateSoftwareRequest>,
) -> ServerResult<Json<ApiResponse<ClusterJobRecord>>> {
    if payload.node_ids.is_empty() {
        return Err(AppError::BadRequest("请至少选择一个目标节点".to_string()).into());
    }
    let pkg = payload.package.trim().to_string();
    let pkg_clone = pkg.clone();
    let results = cluster::dispatch_batch(
        &state,
        payload.node_ids,
        move |req_id| MasterToAgentMsg::UpdateSoftware {
            request_id: req_id,
            package: pkg_clone.clone(),
        },
        180,
    )
    .await;

    let job = ClusterJobRecord {
        id: Uuid::new_v4().to_string(),
        action: "batch_update_software".to_string(),
        payload_summary: format!("批量升级软件: {}", pkg),
        created_at: Utc::now().timestamp(),
        results,
    };
    state.db.insert_cluster_job(&job)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "cluster.batch_update_software",
        &pkg,
        &format!("覆盖 {} 台节点", job.results.len()),
    );
    Ok(Json(ApiResponse::success(job)))
}

pub async fn batch_self_update(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<BatchSelfUpdateRequest>,
) -> ServerResult<Json<ApiResponse<ClusterJobRecord>>> {
    if payload.node_ids.is_empty() {
        return Err(AppError::BadRequest("请至少选择一个目标节点".to_string()).into());
    }
    let url = payload.download_url.clone();
    let sha = payload.sha256.clone();
    let ver = payload.target_version.clone();

    let results = cluster::dispatch_batch(
        &state,
        payload.node_ids,
        move |req_id| MasterToAgentMsg::SelfUpdate {
            request_id: req_id,
            download_url: url.clone(),
            sha256: sha.clone(),
            target_version: ver.clone(),
        },
        180,
    )
    .await;

    let job = ClusterJobRecord {
        id: Uuid::new_v4().to_string(),
        action: "batch_self_update".to_string(),
        payload_summary: format!("批量升级 Agent 至 {}", payload.target_version),
        created_at: Utc::now().timestamp(),
        results,
    };
    state.db.insert_cluster_job(&job)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "cluster.batch_self_update",
        &payload.target_version,
        &format!("覆盖 {} 台节点", job.results.len()),
    );
    Ok(Json(ApiResponse::success(job)))
}

pub async fn list_jobs(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<Vec<ClusterJobRecord>>>> {
    let jobs = state.db.list_cluster_jobs(30)?;
    Ok(Json(ApiResponse::success(jobs)))
}

// ---------- Master Central Backup Repository (Pull from NAT VPS / Cross-Machine Restore) ----------

pub async fn pull_node_backup(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(node_id): Path<String>,
    Json(payload): Json<PullBackupRequest>,
) -> ServerResult<Json<ApiResponse<MasterBackupRecord>>> {
    let record = cluster::pull_backup_from_agent_rpc(
        &state,
        &node_id,
        payload.name,
        payload.include_websites.unwrap_or(true),
        payload.include_docker.unwrap_or(true),
        payload.include_ssl.unwrap_or(true),
        payload.include_panel_db.unwrap_or(true),
        payload.custom_dirs.unwrap_or_default(),
    )
    .await?;

    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "cluster.pull_backup",
        &record.node_name,
        &format!("{} ({} bytes)", record.name, record.size_bytes),
    );
    Ok(Json(ApiResponse::success(record)))
}

pub async fn list_master_backups(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<Vec<MasterBackupRecord>>>> {
    let list = state.db.list_master_backups()?;
    Ok(Json(ApiResponse::success(list)))
}

pub async fn push_restore_backup(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(backup_id): Path<String>,
    Json(payload): Json<PushRestoreRequest>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let msg = cluster::push_restore_to_agent(
        &state,
        &backup_id,
        &payload.target_node_id,
        payload.restore_dir,
    )
    .await?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "cluster.push_restore",
        &payload.target_node_id,
        &msg,
    );
    Ok(Json(ApiResponse::success(msg)))
}

pub async fn download_master_backup(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<String>,
) -> ServerResult<Response> {
    let bk = state
        .db
        .get_master_backup(&id)?
        .ok_or_else(|| AppError::NotFound("主控仓库中不存在该备份".to_string()))?;
    let path = PathBuf::from(&bk.file_path);
    if !path.is_file() {
        return Err(AppError::NotFound("主控备份归档文件已丢失".to_string()).into());
    }
    let fname = format!("{}-{}.tar.gz", bk.node_name, bk.name).replace('"', "");
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|e| AppError::Forbidden(format!("打开文件失败: {}", e)))?;
    let mut resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/gzip")
        .body(Body::from_stream(ReaderStream::new(file)))
        .map_err(|e| AppError::Internal(e.to_string()))?;
    if let Ok(cd) = HeaderValue::from_str(&format!("attachment; filename=\"{}\"", fname)) {
        resp.headers_mut().insert(header::CONTENT_DISPOSITION, cd);
    }
    Ok(resp)
}

pub async fn delete_master_backup(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    if let Some(bk) = state.db.get_master_backup(&id)? {
        let p = PathBuf::from(&bk.file_path);
        if p.is_file() {
            let _ = std::fs::remove_file(p);
        }
    }
    state.db.delete_master_backup(&id)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "cluster.master_backup_delete",
        &id,
        "删除主控中央仓库备份",
    );
    Ok(Json(ApiResponse::success(true)))
}

// ---------- Self-Update & Notification Settings ----------

pub async fn check_update(
    _auth: AuthUser,
    Json(payload): Json<CheckUpdateRequest>,
) -> ServerResult<Json<ApiResponse<UpdateManifest>>> {
    let manifest = tokio::task::spawn_blocking(move || updater::fetch_manifest(&payload.manifest_url))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    Ok(Json(ApiResponse::success(manifest)))
}

pub async fn apply_self_update(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<ApplySelfUpdateRequest>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let updates_dir = state.config_snapshot().updates_dir();
    let ver = payload.target_version.clone();
    let msg = tokio::task::spawn_blocking(move || {
        updater::perform_self_update(
            &updates_dir,
            &payload.download_url,
            &payload.sha256,
            &payload.target_version,
        )
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))??;

    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "system.self_update",
        &ver,
        &msg,
    );
    Ok(Json(ApiResponse::success(msg)))
}

pub async fn get_notify_config(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<NotifyConfig>>> {
    Ok(Json(ApiResponse::success(notify::load_notify_config(&state))))
}

pub async fn save_notify_config(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<NotifyConfig>,
) -> ServerResult<Json<ApiResponse<NotifyConfig>>> {
    let raw = serde_json::to_string(&payload).map_err(|e| AppError::Internal(e.to_string()))?;
    state.db.set_setting("notify_config", &raw)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "settings.notify",
        "notify",
        &format!("enabled={}", payload.enabled),
    );
    Ok(Json(ApiResponse::success(payload)))
}

pub async fn test_notify(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<String>>> {
    let cfg = notify::load_notify_config(&state);
    let msg = tokio::task::spawn_blocking(move || {
        notify::send_notification(
            &cfg,
            "告警测试",
            "这是一条来自 zmpanel 的本地主动测试通知，通道正常。",
        )
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))??;
    Ok(Json(ApiResponse::success(msg)))
}
