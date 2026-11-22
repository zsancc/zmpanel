use axum::{
    Json,
    extract::{Path, State},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zmpanel_core::{ApiResponse, AppError, FirewallRule, FirewallStatus};

use crate::auth::AuthUser;
use crate::error::ServerResult;
use crate::firewall;
use crate::security;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct FirewallOverviewResponse {
    pub status: FirewallStatus,
    pub rules: Vec<FirewallRule>,
}

#[derive(Debug, Deserialize)]
pub struct UpsertRuleRequest {
    pub id: Option<String>,
    pub port: String,
    pub protocol: String, // tcp / udp / both
    pub action: String,   // allow / deny
    pub source: Option<String>,
    pub remark: Option<String>,
    pub enabled: Option<bool>,
}

pub async fn overview(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<FirewallOverviewResponse>>> {
    let panel_port = state.config_snapshot().port;
    let status = tokio::task::spawn_blocking(move || firewall::get_status(panel_port))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let rules = state.db.list_firewall_rules()?;
    Ok(Json(ApiResponse::success(FirewallOverviewResponse {
        status,
        rules,
    })))
}

pub async fn upsert_rule(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<UpsertRuleRequest>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let (p_start, p_end, norm_port) = firewall::validate_port_spec(&payload.port)?;
    let protocol = match payload.protocol.to_lowercase().as_str() {
        "tcp" => "tcp".to_string(),
        "udp" => "udp".to_string(),
        "both" => "both".to_string(),
        _ => return Err(AppError::BadRequest("协议仅支持 tcp / udp / both".to_string()).into()),
    };
    let action = match payload.action.to_lowercase().as_str() {
        "allow" => "allow".to_string(),
        "deny" => "deny".to_string(),
        _ => return Err(AppError::BadRequest("动作仅支持 allow / deny".to_string()).into()),
    };
    let source = firewall::validate_ip_or_cidr(payload.source.as_deref().unwrap_or("any"))?;
    let enabled = payload.enabled.unwrap_or(true);

    let panel_port = state.config_snapshot().port;
    if enabled {
        firewall::check_anti_lockout(p_start, p_end, &action, panel_port)?;
    }

    let now = Utc::now().timestamp();
    let rule = FirewallRule {
        id: payload
            .id
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        port: norm_port.clone(),
        protocol: protocol.clone(),
        action: action.clone(),
        source: source.clone(),
        remark: payload.remark.unwrap_or_default(),
        enabled,
        created_at: now,
    };

    state.db.upsert_firewall_rule(&rule)?;
    let sync_msg = {
        let st = state.clone();
        tokio::task::spawn_blocking(move || security::resync_firewall(&st))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
    };

    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "firewall.rule_save",
        &format!("{}/{}", norm_port, protocol),
        &format!("{} from {} ({})", action, source, sync_msg),
    );
    Ok(Json(ApiResponse::success(sync_msg)))
}

pub async fn delete_rule(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> ServerResult<Json<ApiResponse<String>>> {
    state.db.delete_firewall_rule(&id)?;
    let sync_msg = {
        let st = state.clone();
        tokio::task::spawn_blocking(move || security::resync_firewall(&st))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
    };
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "firewall.rule_delete",
        &id,
        &sync_msg,
    );
    Ok(Json(ApiResponse::success(sync_msg)))
}

pub async fn sync_now(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ServerResult<Json<ApiResponse<String>>> {
    let sync_msg = {
        let st = state.clone();
        tokio::task::spawn_blocking(move || security::resync_firewall(&st))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
    };
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "firewall.sync",
        "system",
        &sync_msg,
    );
    Ok(Json(ApiResponse::success(sync_msg)))
}
