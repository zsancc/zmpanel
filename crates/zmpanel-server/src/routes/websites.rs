use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;
use zmpanel_core::{ApiResponse, AppError, WebsiteKind, WebsiteRecord, WebsiteWithRuntime};

use crate::auth::AuthUser;
use crate::error::ServerResult;
use crate::files;
use crate::state::AppState;
use crate::websites;

#[derive(Debug, Deserialize)]
pub struct UpsertWebsiteRequest {
    pub id: Option<String>,
    pub name: String,
    pub kind: WebsiteKind,
    pub domains: Vec<String>,
    pub listen_port: Option<u16>,
    pub root_dir: String,
    pub start_cmd: Option<String>,
    pub env_vars: Option<String>,
    pub python_venv: Option<String>,
    pub proxy_target: Option<String>,
    pub websocket_enabled: Option<bool>,
    pub spa_fallback: Option<bool>,
    pub ssl_cert_id: Option<String>,
    pub force_https: Option<bool>,
    pub auto_restart: Option<bool>,
    pub custom_nginx: Option<String>,
    pub remark: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SiteActionRequest {
    pub action: String, // start / stop / restart
}

#[derive(Debug, Deserialize)]
pub struct SiteLogsQuery {
    pub lines: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct SaveNginxRequest {
    pub content: String,
}

pub async fn list_websites(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<Vec<WebsiteWithRuntime>>>> {
    let sites = state.db.list_websites()?;
    let nginx_dir = state.config_snapshot().nginx_sites_dir();
    let result = sites
        .into_iter()
        .map(|site| {
            let st = state.supervisor.get_status(&site.id, &site.kind);
            let conf_path = files::normalize_display_path(&nginx_dir.join(format!("{}.conf", site.id)));
            WebsiteWithRuntime {
                site,
                runtime_status: st.status,
                pid: st.pid,
                restart_count: st.restart_count,
                nginx_conf_path: conf_path,
            }
        })
        .collect();
    Ok(Json(ApiResponse::success(result)))
}

pub async fn upsert_website(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<UpsertWebsiteRequest>,
) -> ServerResult<Json<ApiResponse<WebsiteWithRuntime>>> {
    let name = payload.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("网站名称不能为空".to_string()).into());
    }
    let clean_domains: Vec<String> = payload
        .domains
        .into_iter()
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
        .collect();
    if clean_domains.is_empty() {
        return Err(AppError::BadRequest("请至少填写一个域名或访问地址".to_string()).into());
    }

    let root_path = files::resolve_safe_path(&payload.root_dir)?;
    if !root_path.exists() {
        std::fs::create_dir_all(&root_path)
            .map_err(|e| AppError::Forbidden(format!("自动创建项目根目录失败: {}", e)))?;
    }

    let now = Utc::now().timestamp();
    let existing = payload
        .id
        .as_deref()
        .and_then(|id| state.db.get_website(id).ok().flatten());

    let site = WebsiteRecord {
        id: existing
            .as_ref()
            .map(|e| e.id.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        name,
        kind: payload.kind,
        domains: clean_domains,
        listen_port: payload.listen_port.unwrap_or(8000),
        root_dir: files::normalize_display_path(&root_path),
        start_cmd: payload.start_cmd.unwrap_or_default(),
        env_vars: payload.env_vars.unwrap_or_default(),
        python_venv: payload.python_venv.filter(|s| !s.trim().is_empty()),
        proxy_target: payload.proxy_target.filter(|s| !s.trim().is_empty()),
        websocket_enabled: payload.websocket_enabled.unwrap_or(true),
        spa_fallback: payload.spa_fallback.unwrap_or(false),
        ssl_cert_id: payload.ssl_cert_id.filter(|s| !s.trim().is_empty()),
        force_https: payload.force_https.unwrap_or(false),
        auto_restart: payload.auto_restart.unwrap_or(true),
        custom_nginx: payload.custom_nginx.filter(|s| !s.trim().is_empty()),
        remark: payload.remark.unwrap_or_default(),
        created_at: existing.as_ref().map(|e| e.created_at).unwrap_or(now),
        updated_at: now,
    };

    state.db.upsert_website(&site)?;

    let ssl_cert = site
        .ssl_cert_id
        .as_deref()
        .and_then(|cid| state.db.get_ssl_cert(cid).ok().flatten());
    let nginx_dir = state.config_snapshot().nginx_sites_dir();
    let conf_path = websites::write_site_nginx_conf(&nginx_dir, &site, ssl_cert.as_ref())?;

    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "websites.save",
        &site.name,
        &format!("类型: {}", site.kind.as_str()),
    );

    let st = state.supervisor.get_status(&site.id, &site.kind);
    Ok(Json(ApiResponse::success(WebsiteWithRuntime {
        site,
        runtime_status: st.status,
        pid: st.pid,
        restart_count: st.restart_count,
        nginx_conf_path: files::normalize_display_path(&conf_path),
    })))
}

pub async fn delete_website(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    let _ = state.supervisor.stop_site(&id);
    let conf = state
        .config_snapshot()
        .nginx_sites_dir()
        .join(format!("{}.conf", id));
    if conf.exists() {
        let _ = std::fs::remove_file(conf);
    }
    state.db.delete_website(&id)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "websites.delete",
        &id,
        "删除网站记录及对应 Nginx 配置（保留源码目录）",
    );
    Ok(Json(ApiResponse::success(true)))
}

pub async fn site_action(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(payload): Json<SiteActionRequest>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let site = state
        .db
        .get_website(&id)?
        .ok_or_else(|| AppError::NotFound("网站不存在".to_string()))?;
    let logs_dir = state.config_snapshot().logs_dir();
    let msg = match payload.action.as_str() {
        "start" => {
            let pid = state.supervisor.start_site(&site, &logs_dir)?;
            format!("进程已启动 (PID {})", pid)
        }
        "stop" => {
            state.supervisor.stop_site(&site.id)?;
            "进程已停止".to_string()
        }
        "restart" => {
            let pid = state.supervisor.start_site(&site, &logs_dir)?;
            format!("进程已重启 (PID {})", pid)
        }
        _ => return Err(AppError::BadRequest("未知进程动作".to_string()).into()),
    };
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "websites.process",
        &site.name,
        &msg,
    );
    Ok(Json(ApiResponse::success(msg)))
}

pub async fn site_logs(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<String>,
    Query(q): Query<SiteLogsQuery>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let logs_dir = state.config_snapshot().logs_dir();
    let lines = q.lines.unwrap_or(200);
    let txt = tokio::task::spawn_blocking(move || websites::read_site_logs(&logs_dir, &id, lines))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
    Ok(Json(ApiResponse::success(txt)))
}

pub async fn get_site_nginx(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<String>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let site = state
        .db
        .get_website(&id)?
        .ok_or_else(|| AppError::NotFound("网站不存在".to_string()))?;
    let ssl_cert = site
        .ssl_cert_id
        .as_deref()
        .and_then(|cid| state.db.get_ssl_cert(cid).ok().flatten());
    let conf = websites::generate_nginx_config(&site, ssl_cert.as_ref());
    Ok(Json(ApiResponse::success(conf)))
}

pub async fn save_site_nginx(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(payload): Json<SaveNginxRequest>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    let mut site = state
        .db
        .get_website(&id)?
        .ok_or_else(|| AppError::NotFound("网站不存在".to_string()))?;
    site.custom_nginx = Some(payload.content);
    site.updated_at = Utc::now().timestamp();
    state.db.upsert_website(&site)?;
    let nginx_dir = state.config_snapshot().nginx_sites_dir();
    websites::write_site_nginx_conf(&nginx_dir, &site, None)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "websites.nginx_edit",
        &site.name,
        "自定义保存网站 Nginx 配置",
    );
    Ok(Json(ApiResponse::success(true)))
}
