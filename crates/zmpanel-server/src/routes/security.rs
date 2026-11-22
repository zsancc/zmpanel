use axum::{
    Json,
    extract::{Path, State},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use zmpanel_core::{
    ApiResponse, AppError, SecurityBanItem, SecurityConfig, SecurityEventItem,
    SecurityWhitelistItem, SshHardeningStatus,
};

use crate::auth::AuthUser;
use crate::error::ServerResult;
use crate::firewall;
use crate::security;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct SecurityOverviewResponse {
    pub config: SecurityConfig,
    pub bans: Vec<SecurityBanItem>,
    pub events: Vec<SecurityEventItem>,
    pub whitelist: Vec<SecurityWhitelistItem>,
    pub ssh_hardening: SshHardeningStatus,
}

#[derive(Debug, Deserialize)]
pub struct ManualBanRequest {
    pub ip: String,
    pub reason: Option<String>,
    pub duration_secs: Option<i64>, // 0 = permanent
}

#[derive(Debug, Deserialize)]
pub struct WhitelistRequest {
    pub ip: String,
    pub remark: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SshHardeningRequest {
    pub port: u16,
    pub password_auth: bool,
    pub permit_root_login: String,
    pub pubkey_auth: bool,
}

#[derive(Debug, Deserialize)]
pub struct AnalyzeSampleRequest {
    pub kind: String, // ssh / port_scan / web
    pub log_text: String,
}

pub async fn overview(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<SecurityOverviewResponse>>> {
    let config = security::load_security_config(&state);
    let bans = state.db.list_security_bans()?;
    let events = state.db.list_security_events(120)?;
    let whitelist = state.db.list_security_whitelist()?;
    let ssh_hardening = tokio::task::spawn_blocking(security::read_ssh_hardening)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(ApiResponse::success(SecurityOverviewResponse {
        config,
        bans,
        events,
        whitelist,
        ssh_hardening,
    })))
}

pub async fn save_config(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<SecurityConfig>,
) -> ServerResult<Json<ApiResponse<SecurityConfig>>> {
    let raw = serde_json::to_string(&payload)
        .map_err(|e| AppError::Internal(format!("序列化安全配置失败: {}", e)))?;
    state.db.set_setting("security_engine_config", &raw)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "security.config",
        "engine",
        &format!(
            "enabled={}, ssh={}, scan={}, web={}",
            payload.enabled,
            payload.ssh_guard_enabled,
            payload.port_scan_enabled,
            payload.web_guard_enabled
        ),
    );
    Ok(Json(ApiResponse::success(payload)))
}

pub async fn manual_ban(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<ManualBanRequest>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let ip = firewall::validate_ip_or_cidr(&payload.ip)?;
    if ip == "any" {
        return Err(AppError::BadRequest("不能封禁 any，请输入具体 IP 地址".to_string()).into());
    }
    if state.db.is_ip_whitelisted(&ip)? {
        return Err(AppError::Forbidden(format!(
            "IP {} 在安全白名单中，请先移除白名单再封禁",
            ip
        ))
        .into());
    }
    let now = Utc::now().timestamp();
    let duration = payload.duration_secs.unwrap_or(86400);
    let expires_at = if duration > 0 { now + duration } else { 0 };
    let reason = payload
        .reason
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "管理员手动封禁".to_string());

    state.db.upsert_security_ban(&SecurityBanItem {
        ip: ip.clone(),
        reason: reason.clone(),
        category: "manual".to_string(),
        hits: 1,
        banned_at: now,
        expires_at,
    })?;
    let _ = state
        .db
        .insert_security_event("manual", &ip, &reason, "manual_banned", "");

    let sync_msg = {
        let st = state.clone();
        tokio::task::spawn_blocking(move || security::resync_firewall(&st))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
    };

    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "security.ban",
        &ip,
        &format!("{} ({})", reason, sync_msg),
    );
    Ok(Json(ApiResponse::success(sync_msg)))
}

pub async fn unban_ip(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(ip): Path<String>,
) -> ServerResult<Json<ApiResponse<String>>> {
    state.db.delete_security_ban(&ip)?;
    let _ = state
        .db
        .insert_security_event("manual", &ip, "管理员手动解封", "unbanned", "");
    let sync_msg = {
        let st = state.clone();
        tokio::task::spawn_blocking(move || security::resync_firewall(&st))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
    };
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "security.unban",
        &ip,
        &sync_msg,
    );
    Ok(Json(ApiResponse::success(sync_msg)))
}

pub async fn add_whitelist(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<WhitelistRequest>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    let ip = firewall::validate_ip_or_cidr(&payload.ip)?;
    if ip == "any" {
        return Err(AppError::BadRequest("白名单需指定具体 IP 地址".to_string()).into());
    }
    state
        .db
        .upsert_security_whitelist(&ip, payload.remark.as_deref().unwrap_or(""))?;
    // Remove from ban list if present
    let _ = state.db.delete_security_ban(&ip);
    let st = state.clone();
    let _ = tokio::task::spawn_blocking(move || security::resync_firewall(&st)).await;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "security.whitelist_add",
        &ip,
        "加入安全白名单",
    );
    Ok(Json(ApiResponse::success(true)))
}

pub async fn remove_whitelist(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(ip): Path<String>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    state.db.delete_security_whitelist(&ip)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "security.whitelist_del",
        &ip,
        "移出安全白名单",
    );
    Ok(Json(ApiResponse::success(true)))
}

pub async fn scan_now(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ServerResult<Json<ApiResponse<usize>>> {
    let st = state.clone();
    let count = tokio::task::spawn_blocking(move || security::run_security_cycle(&st))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "security.scan_now",
        "engine",
        &format!("立即巡检完成，新增封禁 {} 个 IP", count),
    );
    Ok(Json(ApiResponse::success(count)))
}

/// Analyze supplied log lines through the real built-in detection engine and apply bans (useful for live verification).
pub async fn analyze_sample(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<AnalyzeSampleRequest>,
) -> ServerResult<Json<ApiResponse<usize>>> {
    let cfg = security::load_security_config(&state);
    let lines: Vec<String> = payload.log_text.lines().map(|s| s.to_string()).collect();
    let findings = match payload.kind.as_str() {
        "ssh" => security::analyze_ssh_lines(&lines, cfg.ssh_max_failures),
        "port_scan" => security::analyze_port_scan_lines(&lines, cfg.port_scan_threshold),
        "web" => security::analyze_web_lines(&lines, cfg.web_scan_threshold),
        _ => return Err(AppError::BadRequest("检测类型仅支持 ssh / port_scan / web".to_string()).into()),
    };
    let st = state.clone();
    let count = tokio::task::spawn_blocking(move || {
        let n = security::apply_findings(&st, &cfg, findings);
        if n > 0 {
            let _ = security::resync_firewall(&st);
        }
        n
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "security.analyze",
        &payload.kind,
        &format!("分析完成，新增封禁 {} 个 IP", count),
    );
    Ok(Json(ApiResponse::success(count)))
}

pub async fn update_ssh_hardening(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<SshHardeningRequest>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let msg = tokio::task::spawn_blocking(move || {
        security::apply_ssh_hardening(
            payload.port,
            payload.password_auth,
            &payload.permit_root_login,
            payload.pubkey_auth,
        )
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))??;

    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "security.ssh_hardening",
        "/etc/ssh/sshd_config",
        &msg,
    );
    Ok(Json(ApiResponse::success(msg)))
}
