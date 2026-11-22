use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;
use zmpanel_core::{
    ApiResponse, AppError, DockerContainerInfo, DockerImageInfo, DockerProject,
    DockerStatusResponse,
};

use crate::auth::AuthUser;
use crate::docker;
use crate::error::ServerResult;
use crate::files;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ContainerListQuery {
    pub all: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ContainerActionRequest {
    pub action: String,
}

#[derive(Debug, Deserialize)]
pub struct LogsQuery {
    pub tail: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct UpsertProjectRequest {
    pub id: Option<String>,
    pub name: String,
    pub work_dir: Option<String>,
    pub compose_file: Option<String>,
    pub deploy_cmd: Option<String>,
    pub description: Option<String>,
    pub compose_content: Option<String>,
    pub env_content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ComposeActionRequest {
    pub action: String,
}

pub async fn status(
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<DockerStatusResponse>>> {
    let st = tokio::task::spawn_blocking(docker::check_docker_status)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(ApiResponse::success(st)))
}

pub async fn list_containers(
    _auth: AuthUser,
    Query(q): Query<ContainerListQuery>,
) -> ServerResult<Json<ApiResponse<Vec<DockerContainerInfo>>>> {
    let include_all = q.all.unwrap_or(true);
    let list = tokio::task::spawn_blocking(move || docker::list_containers(include_all))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    Ok(Json(ApiResponse::success(list)))
}

pub async fn container_action(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(payload): Json<ContainerActionRequest>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let cid = id.clone();
    let act = payload.action.clone();
    let out = tokio::task::spawn_blocking(move || docker::container_action(&cid, &act))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "docker.container_action",
        &id,
        &payload.action,
    );
    Ok(Json(ApiResponse::success(out)))
}

pub async fn container_logs(
    _auth: AuthUser,
    Path(id): Path<String>,
    Query(q): Query<LogsQuery>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let tail = q.tail.unwrap_or(200);
    let logs = tokio::task::spawn_blocking(move || docker::container_logs(&id, tail))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    Ok(Json(ApiResponse::success(logs)))
}

pub async fn list_images(
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<Vec<DockerImageInfo>>>> {
    let list = tokio::task::spawn_blocking(docker::list_images)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    Ok(Json(ApiResponse::success(list)))
}

pub async fn remove_image(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let img = id.clone();
    let out = tokio::task::spawn_blocking(move || docker::remove_image(&img))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "docker.image_remove",
        &id,
        "删除镜像",
    );
    Ok(Json(ApiResponse::success(out)))
}

pub async fn prune(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ServerResult<Json<ApiResponse<String>>> {
    let out = tokio::task::spawn_blocking(docker::prune_system)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "docker.prune",
        "system",
        "清理无用 Docker 资源",
    );
    Ok(Json(ApiResponse::success(out)))
}

pub async fn list_projects(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<Vec<DockerProject>>>> {
    let items = state.db.list_docker_projects()?;
    Ok(Json(ApiResponse::success(items)))
}

pub async fn upsert_project(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<UpsertProjectRequest>,
) -> ServerResult<Json<ApiResponse<DockerProject>>> {
    let name = payload.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("项目名称不能为空".to_string()).into());
    }
    let default_base = state.config_snapshot().docker_projects_dir();
    let custom_dir = payload.work_dir.clone();
    let compose_content = payload.compose_content.clone();
    let env_content = payload.env_content.clone();
    let name_clone = name.clone();

    let work_dir = tokio::task::spawn_blocking(move || {
        docker::init_project_workspace(
            &default_base,
            &name_clone,
            custom_dir.as_deref(),
            compose_content.as_deref(),
            env_content.as_deref(),
        )
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))??;

    let now = Utc::now().timestamp();
    let existing = payload
        .id
        .as_deref()
        .and_then(|id| state.db.get_docker_project(id).ok().flatten());

    let project = DockerProject {
        id: existing
            .as_ref()
            .map(|e| e.id.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        name,
        work_dir: files::normalize_display_path(&work_dir),
        compose_file: payload
            .compose_file
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "docker-compose.yml".to_string()),
        deploy_cmd: payload
            .deploy_cmd
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "docker compose up -d".to_string()),
        description: payload.description.unwrap_or_default(),
        created_at: existing.as_ref().map(|e| e.created_at).unwrap_or(now),
        updated_at: now,
    };

    state.db.upsert_docker_project(&project)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "docker.project_save",
        &project.name,
        &project.work_dir,
    );
    Ok(Json(ApiResponse::success(project)))
}

pub async fn delete_project(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    state.db.delete_docker_project(&id)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "docker.project_delete",
        &id,
        "移除 Docker 工作区记录（保留目录文件）",
    );
    Ok(Json(ApiResponse::success(true)))
}

pub async fn project_compose_action(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(payload): Json<ComposeActionRequest>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let project = state
        .db
        .get_docker_project(&id)?
        .ok_or_else(|| AppError::NotFound("未找到该 Docker 工作区".to_string()))?;
    let p_clone = project.clone();
    let act = payload.action.clone();
    let out = tokio::task::spawn_blocking(move || {
        if act == "deploy_cmd" {
            docker::run_project_deploy_cmd(&p_clone)
        } else {
            docker::run_compose_action(&p_clone, &act)
        }
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))??;

    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "docker.project_action",
        &project.name,
        &payload.action,
    );
    Ok(Json(ApiResponse::success(out)))
}
