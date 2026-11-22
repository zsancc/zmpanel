use axum::{
    Json,
    extract::{Path, State},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zmpanel_core::{
    ApiResponse, AppError, DockerProject, MongoServiceConfig, NginxSecurityConfig,
    PostgresServiceConfig, PythonVenvItem, RedisServiceConfig, StoreAppItem,
};

use crate::auth::AuthUser;
use crate::error::ServerResult;
use crate::files;
use crate::state::AppState;
use crate::store;

#[derive(Debug, Serialize)]
pub struct NginxOverviewResponse {
    pub config: NginxSecurityConfig,
    pub rendered_conf: String,
    pub conf_path: String,
}

#[derive(Debug, Deserialize)]
pub struct NginxControlRequest {
    pub action: String, // test / reload / start / stop
}

#[derive(Debug, Serialize)]
pub struct UvOverviewResponse {
    pub installed: bool,
    pub version: String,
    pub installed_pythons: String,
    pub venvs: Vec<PythonVenvItem>,
}

#[derive(Debug, Deserialize)]
pub struct InstallPythonRequest {
    pub version: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateVenvRequest {
    pub name: String,
    pub path: String,
    pub python_version: Option<String>,
    pub index_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PipInstallRequest {
    pub venv_path: String,
    pub packages: Vec<String>,
    pub index_url: Option<String>,
}

pub async fn list_apps(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<Vec<StoreAppItem>>>> {
    let store_dir = state.config_snapshot().store_dir();
    let list = tokio::task::spawn_blocking(move || store::list_store_apps(&store_dir))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(ApiResponse::success(list)))
}

pub async fn install_app(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let app_id = id.clone();
    let out = tokio::task::spawn_blocking(move || store::install_native_app(&app_id))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "store.install",
        &id,
        "原生包安装软件",
    );
    Ok(Json(ApiResponse::success(out)))
}

pub async fn get_nginx_security(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<NginxOverviewResponse>>> {
    let cfg: NginxSecurityConfig = state
        .db
        .get_setting("nginx_security_config")?
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    let nginx_dir = state.config_snapshot().nginx_dir();
    let rendered = store::render_nginx_security_conf(&cfg);
    let conf_path = files::normalize_display_path(&nginx_dir.join("security.conf"));
    Ok(Json(ApiResponse::success(NginxOverviewResponse {
        config: cfg,
        rendered_conf: rendered,
        conf_path,
    })))
}

pub async fn save_nginx_security(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<NginxSecurityConfig>,
) -> ServerResult<Json<ApiResponse<NginxOverviewResponse>>> {
    let json_str = serde_json::to_string(&payload)
        .map_err(|e| AppError::Internal(format!("序列化配置失败: {}", e)))?;
    state.db.set_setting("nginx_security_config", &json_str)?;

    let nginx_dir = state.config_snapshot().nginx_dir();
    let conf_path = store::write_nginx_security_conf(&nginx_dir, &payload)?;
    let rendered = store::render_nginx_security_conf(&payload);

    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "store.nginx_security",
        "nginx",
        "更新 Nginx 性能与轻量安全防护配置",
    );
    Ok(Json(ApiResponse::success(NginxOverviewResponse {
        config: payload,
        rendered_conf: rendered,
        conf_path: files::normalize_display_path(&conf_path),
    })))
}

pub async fn nginx_control(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<NginxControlRequest>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let act = payload.action.clone();
    let out = tokio::task::spawn_blocking(move || store::nginx_control(&act))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "store.nginx_control",
        "nginx",
        &payload.action,
    );
    Ok(Json(ApiResponse::success(out)))
}

pub async fn uv_overview(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<UvOverviewResponse>>> {
    let (installed, version, installed_pythons) = tokio::task::spawn_blocking(store::uv_status)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let venvs = state.db.list_python_venvs()?;
    Ok(Json(ApiResponse::success(UvOverviewResponse {
        installed,
        version,
        installed_pythons,
        venvs,
    })))
}

pub async fn uv_install_python(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<InstallPythonRequest>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let ver = payload.version.clone();
    let out = tokio::task::spawn_blocking(move || store::uv_install_python(&ver))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "store.uv_python_install",
        &payload.version,
        "通过 uv 安装 Python 版本",
    );
    Ok(Json(ApiResponse::success(out)))
}

pub async fn uv_create_venv(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<CreateVenvRequest>,
) -> ServerResult<Json<ApiResponse<PythonVenvItem>>> {
    if payload.name.trim().is_empty() || payload.path.trim().is_empty() {
        return Err(AppError::BadRequest("虚拟环境名称和路径不能为空".to_string()).into());
    }
    let py_ver = payload.python_version.clone().unwrap_or_default();
    let path_in = payload.path.clone();
    let created_path =
        tokio::task::spawn_blocking(move || store::uv_create_venv(&path_in, &py_ver))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))??;

    let item = PythonVenvItem {
        id: Uuid::new_v4().to_string(),
        name: payload.name.trim().to_string(),
        path: created_path,
        python_version: payload
            .python_version
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "default".to_string()),
        index_url: payload.index_url.unwrap_or_default(),
        created_at: Utc::now().timestamp(),
    };
    state.db.insert_python_venv(&item)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "store.uv_venv_create",
        &item.name,
        &item.path,
    );
    Ok(Json(ApiResponse::success(item)))
}

pub async fn uv_delete_venv(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    state.db.delete_python_venv(&id)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "store.uv_venv_delete",
        &id,
        "移除虚拟环境记录",
    );
    Ok(Json(ApiResponse::success(true)))
}

pub async fn uv_pip_install(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<PipInstallRequest>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let target = payload.venv_path.clone();
    let pkgs = payload.packages.clone();
    let out = tokio::task::spawn_blocking(move || {
        store::uv_pip_install(
            &payload.venv_path,
            &payload.packages,
            payload.index_url.as_deref(),
        )
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))??;

    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "store.uv_pip_install",
        &target,
        &pkgs.join(", "),
    );
    Ok(Json(ApiResponse::success(out)))
}

// ---------- Stage 3: Redis / PostgreSQL / MongoDB Service Config ----------

#[derive(Debug, Serialize)]
pub struct ServiceBundleResponse<T> {
    pub config: T,
    pub rendered_conf: String,
    pub rendered_compose: String,
    pub conf_path: String,
}

#[derive(Debug, Deserialize)]
pub struct DeployServiceWorkspaceRequest {
    pub service: String, // redis / postgresql / mongodb
}

pub async fn get_redis_service(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<ServiceBundleResponse<RedisServiceConfig>>>> {
    let cfg: RedisServiceConfig = state
        .db
        .get_setting("store_redis_config")?
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    let conf_path = files::normalize_display_path(
        &state.config_snapshot().store_dir().join("redis/redis.conf"),
    );
    Ok(Json(ApiResponse::success(ServiceBundleResponse {
        rendered_conf: store::render_redis_conf(&cfg),
        rendered_compose: store::render_redis_compose(&cfg),
        config: cfg,
        conf_path,
    })))
}

pub async fn save_redis_service(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<RedisServiceConfig>,
) -> ServerResult<Json<ApiResponse<ServiceBundleResponse<RedisServiceConfig>>>> {
    let raw = serde_json::to_string(&payload).map_err(|e| AppError::Internal(e.to_string()))?;
    state.db.set_setting("store_redis_config", &raw)?;
    let rendered_conf = store::render_redis_conf(&payload);
    let rendered_compose = store::render_redis_compose(&payload);
    let store_dir = state.config_snapshot().store_dir();
    let conf_p = store::write_service_file(&store_dir, "redis", "redis.conf", &rendered_conf)?;
    let _ = store::write_service_file(&store_dir, "redis", "docker-compose.yml", &rendered_compose);
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "store.redis_config",
        "redis",
        &format!("port={}, maxmemory={}MB", payload.port, payload.maxmemory_mb),
    );
    Ok(Json(ApiResponse::success(ServiceBundleResponse {
        config: payload,
        rendered_conf,
        rendered_compose,
        conf_path: files::normalize_display_path(&conf_p),
    })))
}

pub async fn get_postgres_service(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<ServiceBundleResponse<PostgresServiceConfig>>>> {
    let cfg: PostgresServiceConfig = state
        .db
        .get_setting("store_postgres_config")?
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    let conf_path = files::normalize_display_path(
        &state
            .config_snapshot()
            .store_dir()
            .join("postgresql/docker-compose.yml"),
    );
    Ok(Json(ApiResponse::success(ServiceBundleResponse {
        rendered_conf: format!(
            "shared_buffers = {}MB\nmax_connections = {}\n",
            cfg.shared_buffers_mb, cfg.max_connections
        ),
        rendered_compose: store::render_postgres_compose(&cfg),
        config: cfg,
        conf_path,
    })))
}

pub async fn save_postgres_service(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<PostgresServiceConfig>,
) -> ServerResult<Json<ApiResponse<ServiceBundleResponse<PostgresServiceConfig>>>> {
    let raw = serde_json::to_string(&payload).map_err(|e| AppError::Internal(e.to_string()))?;
    state.db.set_setting("store_postgres_config", &raw)?;
    let rendered_compose = store::render_postgres_compose(&payload);
    let rendered_conf = format!(
        "shared_buffers = {}MB\nmax_connections = {}\n",
        payload.shared_buffers_mb, payload.max_connections
    );
    let store_dir = state.config_snapshot().store_dir();
    let conf_p =
        store::write_service_file(&store_dir, "postgresql", "docker-compose.yml", &rendered_compose)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "store.postgres_config",
        "postgresql",
        &format!("port={}, shared_buffers={}MB", payload.port, payload.shared_buffers_mb),
    );
    Ok(Json(ApiResponse::success(ServiceBundleResponse {
        config: payload,
        rendered_conf,
        rendered_compose,
        conf_path: files::normalize_display_path(&conf_p),
    })))
}

pub async fn get_mongo_service(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<ServiceBundleResponse<MongoServiceConfig>>>> {
    let cfg: MongoServiceConfig = state
        .db
        .get_setting("store_mongo_config")?
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    let conf_path = files::normalize_display_path(
        &state
            .config_snapshot()
            .store_dir()
            .join("mongodb/docker-compose.yml"),
    );
    Ok(Json(ApiResponse::success(ServiceBundleResponse {
        rendered_conf: format!(
            "wiredTigerCacheSizeGB = {:.2}\nimage = mongo:{}\n",
            cfg.cache_size_gb, cfg.image_tag
        ),
        rendered_compose: store::render_mongo_compose(&cfg),
        config: cfg,
        conf_path,
    })))
}

pub async fn save_mongo_service(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<MongoServiceConfig>,
) -> ServerResult<Json<ApiResponse<ServiceBundleResponse<MongoServiceConfig>>>> {
    let raw = serde_json::to_string(&payload).map_err(|e| AppError::Internal(e.to_string()))?;
    state.db.set_setting("store_mongo_config", &raw)?;
    let rendered_compose = store::render_mongo_compose(&payload);
    let rendered_conf = format!(
        "wiredTigerCacheSizeGB = {:.2}\nimage = mongo:{}\n",
        payload.cache_size_gb, payload.image_tag
    );
    let store_dir = state.config_snapshot().store_dir();
    let conf_p =
        store::write_service_file(&store_dir, "mongodb", "docker-compose.yml", &rendered_compose)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "store.mongo_config",
        "mongodb",
        &format!("port={}, tag={}", payload.port, payload.image_tag),
    );
    Ok(Json(ApiResponse::success(ServiceBundleResponse {
        config: payload,
        rendered_conf,
        rendered_compose,
        conf_path: files::normalize_display_path(&conf_p),
    })))
}

/// One-click register Redis / PostgreSQL / MongoDB compose config as a Docker Workspace project.
pub async fn create_docker_workspace_for_service(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<DeployServiceWorkspaceRequest>,
) -> ServerResult<Json<ApiResponse<DockerProject>>> {
    let (name, compose) = match payload.service.as_str() {
        "redis" => {
            let cfg: RedisServiceConfig = state
                .db
                .get_setting("store_redis_config")?
                .and_then(|r| serde_json::from_str(&r).ok())
                .unwrap_or_default();
            ("store-redis", store::render_redis_compose(&cfg))
        }
        "postgresql" => {
            let cfg: PostgresServiceConfig = state
                .db
                .get_setting("store_postgres_config")?
                .and_then(|r| serde_json::from_str(&r).ok())
                .unwrap_or_default();
            ("store-postgresql", store::render_postgres_compose(&cfg))
        }
        "mongodb" => {
            let cfg: MongoServiceConfig = state
                .db
                .get_setting("store_mongo_config")?
                .and_then(|r| serde_json::from_str(&r).ok())
                .unwrap_or_default();
            ("store-mongodb", store::render_mongo_compose(&cfg))
        }
        _ => return Err(AppError::BadRequest("仅支持 redis / postgresql / mongodb".to_string()).into()),
    };

    let base_dir = state.config_snapshot().docker_projects_dir();
    let name_s = name.to_string();
    let work_dir = tokio::task::spawn_blocking(move || {
        crate::docker::init_project_workspace(&base_dir, &name_s, None, Some(&compose), None)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))??;

    let now = Utc::now().timestamp();
    let project = DockerProject {
        id: format!("store-{}", payload.service),
        name: name.to_string(),
        work_dir: files::normalize_display_path(&work_dir),
        compose_file: "docker-compose.yml".to_string(),
        deploy_cmd: "docker compose up -d".to_string(),
        description: format!("由软件商店一键生成的 {} Docker 编排工作区", payload.service),
        created_at: now,
        updated_at: now,
    };
    state.db.upsert_docker_project(&project)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "store.to_docker_workspace",
        &project.name,
        &project.work_dir,
    );
    Ok(Json(ApiResponse::success(project)))
}
