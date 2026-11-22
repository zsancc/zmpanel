use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::{HeaderValue, StatusCode, header},
    response::Response,
};
use serde::Deserialize;
use std::path::PathBuf;
use tokio_util::io::ReaderStream;
use zmpanel_core::{ApiResponse, AppError, BackupItem};

use crate::auth::AuthUser;
use crate::backup::{self, CreateBackupRequest};
use crate::error::ServerResult;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct RestoreRequest {
    pub restore_dir: Option<String>,
}

pub async fn list_backups(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<Vec<BackupItem>>>> {
    let list = state.db.list_backups()?;
    Ok(Json(ApiResponse::success(list)))
}

pub async fn create_backup(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<CreateBackupRequest>,
) -> ServerResult<Json<ApiResponse<BackupItem>>> {
    let data_dir = state.config_snapshot().data_dir;
    let db = state.db.clone();
    let item = tokio::task::spawn_blocking(move || {
        backup::create_backup_archive(&data_dir, &db, &payload)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))??;

    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "backup.create",
        &item.name,
        &item.summary,
    );
    Ok(Json(ApiResponse::success(item)))
}

pub async fn restore_backup(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(payload): Json<RestoreRequest>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let item = state
        .db
        .get_backup(&id)?
        .ok_or_else(|| AppError::NotFound("备份快照不存在".to_string()))?;
    let data_dir = state.config_snapshot().data_dir;
    let db = state.db.clone();
    let item_name = item.name.clone();
    let msg = tokio::task::spawn_blocking(move || {
        backup::restore_backup_archive(&item, &data_dir, &db, payload.restore_dir.as_deref())
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))??;

    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "backup.restore",
        &item_name,
        &msg,
    );
    Ok(Json(ApiResponse::success(msg)))
}

pub async fn download_backup(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<String>,
) -> ServerResult<Response> {
    let item = state
        .db
        .get_backup(&id)?
        .ok_or_else(|| AppError::NotFound("备份快照不存在".to_string()))?;
    let path = PathBuf::from(&item.file_path);
    if !path.is_file() {
        return Err(AppError::NotFound("备份归档文件已丢失".to_string()).into());
    }
    let fname = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("backup.tar.gz")
        .replace('"', "");
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|e| AppError::Forbidden(format!("打开备份文件失败: {}", e)))?;
    let stream = ReaderStream::new(file);
    let mut resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/gzip")
        .body(Body::from_stream(stream))
        .map_err(|e| AppError::Internal(e.to_string()))?;
    if let Ok(cd) = HeaderValue::from_str(&format!("attachment; filename=\"{}\"", fname)) {
        resp.headers_mut().insert(header::CONTENT_DISPOSITION, cd);
    }
    Ok(resp)
}

pub async fn delete_backup(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    if let Some(item) = state.db.get_backup(&id)? {
        let p = PathBuf::from(&item.file_path);
        if p.is_file() {
            let _ = std::fs::remove_file(p);
        }
    }
    state.db.delete_backup(&id)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "backup.delete",
        &id,
        "删除备份快照",
    );
    Ok(Json(ApiResponse::success(true)))
}
