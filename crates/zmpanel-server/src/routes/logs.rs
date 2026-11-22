use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};
use zmpanel_core::{ApiResponse, AppError, LogSourceItem};

use crate::auth::AuthUser;
use crate::error::ServerResult;
use crate::logs;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct LogTailQuery {
    pub path: String,
    pub lines: Option<usize>,
    pub q: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LogTailResponse {
    pub path: String,
    pub size_bytes: u64,
    pub matched_lines: usize,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct TruncateLogRequest {
    pub path: String,
}

pub async fn list_sources(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<Vec<LogSourceItem>>>> {
    let data_dir = state.config_snapshot().data_dir;
    let db = state.db.clone();
    let list = tokio::task::spawn_blocking(move || logs::list_log_sources(&data_dir, &db))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(ApiResponse::success(list)))
}

pub async fn tail_log(
    _auth: AuthUser,
    Query(query): Query<LogTailQuery>,
) -> ServerResult<Json<ApiResponse<LogTailResponse>>> {
    let path = query.path.clone();
    let lines = query.lines.unwrap_or(200);
    let kw = query.q.clone();
    let (content, size_bytes, matched_lines) =
        tokio::task::spawn_blocking(move || logs::tail_log_file(&path, lines, kw.as_deref()))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))??;
    Ok(Json(ApiResponse::success(LogTailResponse {
        path: query.path,
        size_bytes,
        matched_lines,
        content,
    })))
}

pub async fn truncate_log(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<TruncateLogRequest>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    let target = payload.path.clone();
    tokio::task::spawn_blocking(move || logs::truncate_log_file(&payload.path))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "logs.truncate",
        &target,
        "安全截断清空日志文件",
    );
    Ok(Json(ApiResponse::success(true)))
}
