use axum::{
    Json,
    body::Body,
    extract::{Multipart, Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::Response,
};
use serde::{Deserialize, Serialize};
use tokio_util::io::ReaderStream;
use zmpanel_core::{ApiResponse, AppError, FileEntry, TrashItem};

use crate::auth::AuthUser;
use crate::error::ServerResult;
use crate::files;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct PathQuery {
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ListDirResponse {
    pub path: String,
    pub entries: Vec<FileEntry>,
}

#[derive(Debug, Serialize)]
pub struct FileContentResponse {
    pub path: String,
    pub size: u64,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct WriteFileRequest {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateEntryRequest {
    pub path: String,
    pub is_dir: bool,
}

#[derive(Debug, Deserialize)]
pub struct TransferRequest {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Deserialize)]
pub struct DeleteRequest {
    pub paths: Vec<String>,
    pub permanent: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ChmodRequest {
    pub path: String,
    pub mode: String,
}

#[derive(Debug, Deserialize)]
pub struct CompressRequest {
    pub paths: Vec<String>,
    pub dest: String,
}

#[derive(Debug, Deserialize)]
pub struct ExtractRequest {
    pub archive: String,
    pub dest_dir: String,
}

#[derive(Debug, Deserialize)]
pub struct RestoreTrashRequest {
    pub id: String,
}

pub async fn list_dir(
    _auth: AuthUser,
    Query(query): Query<PathQuery>,
) -> ServerResult<Json<ApiResponse<ListDirResponse>>> {
    let raw = query.path.unwrap_or_default();
    let (path, entries) = tokio::task::spawn_blocking(move || files::list_directory(&raw))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    Ok(Json(ApiResponse::success(ListDirResponse { path, entries })))
}

pub async fn read_file(
    _auth: AuthUser,
    Query(query): Query<PathQuery>,
) -> ServerResult<Json<ApiResponse<FileContentResponse>>> {
    let raw = query
        .path
        .ok_or_else(|| AppError::BadRequest("缺少 path 参数".to_string()))?;
    let raw_clone = raw.clone();
    let (content, size) = tokio::task::spawn_blocking(move || files::read_text_file(&raw_clone))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    Ok(Json(ApiResponse::success(FileContentResponse {
        path: raw,
        size,
        content,
    })))
}

pub async fn write_file(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<WriteFileRequest>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    let target = payload.path.clone();
    tokio::task::spawn_blocking(move || files::write_text_file(&payload.path, &payload.content))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "files.write",
        &target,
        "在线保存文件内容",
    );
    Ok(Json(ApiResponse::success(true)))
}

pub async fn create_entry(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<CreateEntryRequest>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    let target = payload.path.clone();
    let kind = if payload.is_dir { "目录" } else { "文件" };
    tokio::task::spawn_blocking(move || files::create_entry(&payload.path, payload.is_dir))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "files.create",
        &target,
        &format!("新建{}", kind),
    );
    Ok(Json(ApiResponse::success(true)))
}

pub async fn rename_entry(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<TransferRequest>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    let detail = format!("{} -> {}", payload.from, payload.to);
    let target = payload.to.clone();
    tokio::task::spawn_blocking(move || files::rename_entry(&payload.from, &payload.to))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "files.rename",
        &target,
        &detail,
    );
    Ok(Json(ApiResponse::success(true)))
}

pub async fn copy_entry(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<TransferRequest>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    let detail = format!("{} -> {}", payload.from, payload.to);
    let target = payload.to.clone();
    tokio::task::spawn_blocking(move || files::copy_entry(&payload.from, &payload.to))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "files.copy",
        &target,
        &detail,
    );
    Ok(Json(ApiResponse::success(true)))
}

pub async fn delete_entry(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<DeleteRequest>,
) -> ServerResult<Json<ApiResponse<usize>>> {
    if payload.paths.is_empty() {
        return Err(AppError::BadRequest("未指定要删除的路径".to_string()).into());
    }
    let permanent = payload.permanent.unwrap_or(false);
    let trash_dir = state.config_snapshot().trash_dir();
    let db = state.db.clone();
    let paths = payload.paths.clone();

    let count = tokio::task::spawn_blocking(move || -> Result<usize, AppError> {
        let mut ok = 0;
        for p in &paths {
            if permanent {
                files::delete_permanently(p)?;
            } else {
                files::move_to_trash(p, &trash_dir, &db)?;
            }
            ok += 1;
        }
        Ok(ok)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))??;

    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        if permanent {
            "files.delete_permanent"
        } else {
            "files.trash"
        },
        &payload.paths.join(", "),
        &format!("共删除 {} 项", count),
    );
    Ok(Json(ApiResponse::success(count)))
}

pub async fn chmod_entry(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<ChmodRequest>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    let target = payload.path.clone();
    let mode = payload.mode.clone();
    tokio::task::spawn_blocking(move || files::chmod_entry(&payload.path, &payload.mode))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "files.chmod",
        &target,
        &format!("权限调整为 {}", mode),
    );
    Ok(Json(ApiResponse::success(true)))
}

pub async fn compress_entries(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<CompressRequest>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let dest = tokio::task::spawn_blocking(move || {
        files::compress_entries(&payload.paths, &payload.dest)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))??;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "files.compress",
        &dest,
        "创建压缩包",
    );
    Ok(Json(ApiResponse::success(dest)))
}

pub async fn extract_archive(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<ExtractRequest>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let archive_name = payload.archive.clone();
    let dest = tokio::task::spawn_blocking(move || {
        files::extract_archive(&payload.archive, &payload.dest_dir)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))??;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "files.extract",
        &dest,
        &format!("解压 {}", archive_name),
    );
    Ok(Json(ApiResponse::success(dest)))
}

pub async fn upload_files(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<PathQuery>,
    mut multipart: Multipart,
) -> ServerResult<Json<ApiResponse<Vec<String>>>> {
    let raw_dir = query
        .path
        .ok_or_else(|| AppError::BadRequest("缺少目标目录 path 参数".to_string()))?;
    let target_dir = files::resolve_safe_path(&raw_dir)?;
    if !target_dir.is_dir() {
        return Err(AppError::BadRequest("目标上传路径不是有效目录".to_string()).into());
    }

    let mut saved = Vec::new();
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("解析上传数据失败: {}", e)))?
    {
        let Some(orig_name) = field.file_name().map(|s| s.to_string()) else {
            continue;
        };
        let safe_name = std::path::Path::new(&orig_name)
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| AppError::BadRequest("非法上传文件名".to_string()))?
            .to_string();
        let dest_path = target_dir.join(&safe_name);
        let mut out = tokio::fs::File::create(&dest_path)
            .await
            .map_err(|e| AppError::Forbidden(format!("创建上传目标文件失败: {}", e)))?;

        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| AppError::BadRequest(format!("读取上传分片失败: {}", e)))?
        {
            tokio::io::AsyncWriteExt::write_all(&mut out, &chunk)
                .await
                .map_err(|e| AppError::Internal(format!("写入文件失败: {}", e)))?;
        }
        saved.push(files::normalize_display_path(&dest_path));
    }

    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "files.upload",
        &raw_dir,
        &format!("上传 {} 个文件", saved.len()),
    );
    Ok(Json(ApiResponse::success(saved)))
}

pub async fn download_file(
    _auth: AuthUser,
    Query(query): Query<PathQuery>,
) -> ServerResult<Response> {
    let raw = query
        .path
        .ok_or_else(|| AppError::BadRequest("缺少 path 参数".to_string()))?;
    let path = files::resolve_safe_path(&raw)?;
    if !path.is_file() {
        return Err(AppError::NotFound("文件不存在或不是普通文件".to_string()).into());
    }
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("download.bin")
        .replace('"', "");
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|e| AppError::Forbidden(format!("打开下载文件失败: {}", e)))?;
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);
    let mime = mime_guess::from_path(&path).first_or_octet_stream();

    let mut resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime.as_ref())
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    if let Ok(cd) = HeaderValue::from_str(&format!("attachment; filename=\"{}\"", filename)) {
        resp.headers_mut().insert(header::CONTENT_DISPOSITION, cd);
    }
    Ok(resp)
}

pub async fn list_trash(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<Vec<TrashItem>>>> {
    let items = state.db.list_trash_items()?;
    Ok(Json(ApiResponse::success(items)))
}

pub async fn restore_trash(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<RestoreTrashRequest>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let trash_dir = state.config_snapshot().trash_dir();
    let db = state.db.clone();
    let id = payload.id.clone();
    let restored = tokio::task::spawn_blocking(move || files::restore_trash_item(&id, &trash_dir, &db))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "files.trash_restore",
        &restored,
        "从回收站还原",
    );
    Ok(Json(ApiResponse::success(restored)))
}

pub async fn delete_trash_item(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    let trash_dir = state.config_snapshot().trash_dir();
    let db = state.db.clone();
    let id_clone = id.clone();
    tokio::task::spawn_blocking(move || files::delete_trash_item(&id_clone, &trash_dir, &db))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "files.trash_delete",
        &id,
        "彻底清理回收站条目",
    );
    Ok(Json(ApiResponse::success(true)))
}

pub async fn empty_trash(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ServerResult<Json<ApiResponse<bool>>> {
    let trash_dir = state.config_snapshot().trash_dir();
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || files::empty_trash(&trash_dir, &db))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "files.trash_empty",
        "trash",
        "清空回收站",
    );
    Ok(Json(ApiResponse::success(true)))
}
