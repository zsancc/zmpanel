use axum::{
    Json,
    extract::State,
    http::{HeaderMap, HeaderValue, header},
    response::IntoResponse,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use zmpanel_core::{ApiResponse, AppError, UserProfile};

use crate::auth::{
    AuthUser, SESSION_COOKIE_NAME, build_totp, extract_client_ip, generate_random_token,
    generate_totp_secret, hash_password, sha256_hex, verify_password, verify_totp_code,
};
use crate::error::ServerResult;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    pub totp_code: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub profile: UserProfile,
}

#[derive(Debug, Serialize)]
pub struct AuthStatusResponse {
    pub version: String,
    pub mode: String,
    pub requires_totp_hint: bool,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Debug, Serialize)]
pub struct TotpSetupResponse {
    pub secret: String,
    pub otpauth_url: String,
}

#[derive(Debug, Deserialize)]
pub struct TotpVerifyRequest {
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSecurityRequest {
    pub security_entry: String,
}

pub async fn auth_status(State(state): State<AppState>) -> ServerResult<Json<ApiResponse<AuthStatusResponse>>> {
    let cfg = state.config_snapshot();
    Ok(Json(ApiResponse::success(AuthStatusResponse {
        version: env!("CARGO_PKG_VERSION").to_string(),
        mode: format!("{:?}", cfg.mode).to_lowercase(),
        requires_totp_hint: false,
    })))
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<LoginRequest>,
) -> ServerResult<impl IntoResponse> {
    let cfg = state.config_snapshot();
    let ip = extract_client_ip(&headers);
    state.limiter.check(&ip, cfg.max_login_attempts)?;

    let user = match state.db.find_user_by_username(payload.username.trim())? {
        Some(u) => u,
        None => {
            state.limiter.record_failure(&ip, cfg.lock_minutes);
            let _ = state.db.write_audit(
                payload.username.trim(),
                &ip,
                "auth.login_failed",
                "panel",
                "用户名不存在",
            );
            return Err(AppError::Unauthorized.into());
        }
    };

    if !verify_password(&payload.password, &user.password_hash) {
        state.limiter.record_failure(&ip, cfg.lock_minutes);
        let _ = state
            .db
            .write_audit(&user.username, &ip, "auth.login_failed", "panel", "密码错误");
        return Err(AppError::Unauthorized.into());
    }

    if user.totp_enabled {
        let Some(ref secret) = user.totp_secret else {
            return Err(AppError::Internal("2FA 配置损坏".to_string()).into());
        };
        let code = payload.totp_code.as_deref().unwrap_or("").trim();
        if code.is_empty() {
            return Err(AppError::Forbidden("需要双因素认证（2FA）动态验证码".to_string()).into());
        }
        if !verify_totp_code(secret, &user.username, code) {
            state.limiter.record_failure(&ip, cfg.lock_minutes);
            let _ = state
                .db
                .write_audit(&user.username, &ip, "auth.login_failed", "panel", "2FA 验证码错误");
            return Err(AppError::Forbidden("2FA 动态验证码错误".to_string()).into());
        }
    }

    state.limiter.clear(&ip);

    let raw_token = generate_random_token(32);
    let token_hash = sha256_hex(&raw_token);
    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    let expires_at = Utc::now().timestamp() + cfg.session_ttl_hours * 3600;
    state
        .db
        .create_session(&token_hash, user.id, &ip, ua, expires_at)?;

    let _ = state
        .db
        .write_audit(&user.username, &ip, "auth.login", "panel", "登录成功");

    let profile = UserProfile {
        username: user.username,
        totp_enabled: user.totp_enabled,
        security_entry: state.current_entry(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        mode: format!("{:?}", cfg.mode).to_lowercase(),
    };

    let cookie = format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        SESSION_COOKIE_NAME,
        raw_token,
        cfg.session_ttl_hours * 3600
    );
    let mut resp_headers = HeaderMap::new();
    if let Ok(val) = HeaderValue::from_str(&cookie) {
        resp_headers.insert(header::SET_COOKIE, val);
    }

    Ok((
        resp_headers,
        Json(ApiResponse::success(LoginResponse {
            token: raw_token,
            profile,
        })),
    ))
}

pub async fn logout(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ServerResult<impl IntoResponse> {
    state.db.delete_session(&auth.token_hash)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "auth.logout",
        "panel",
        "退出登录",
    );
    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(
        header::SET_COOKIE,
        HeaderValue::from_static("zmpanel_sid=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0"),
    );
    Ok((resp_headers, Json(ApiResponse::success(true))))
}

pub async fn me(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ServerResult<Json<ApiResponse<UserProfile>>> {
    let cfg = state.config_snapshot();
    Ok(Json(ApiResponse::success(UserProfile {
        username: auth.user.username,
        totp_enabled: auth.user.totp_enabled,
        security_entry: state.current_entry(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        mode: format!("{:?}", cfg.mode).to_lowercase(),
    })))
}

pub async fn change_password(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<ChangePasswordRequest>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    if payload.new_password.len() < 8 {
        return Err(AppError::BadRequest("新密码长度不能少于 8 位".to_string()).into());
    }
    if !verify_password(&payload.current_password, &auth.user.password_hash) {
        return Err(AppError::Forbidden("当前密码不正确".to_string()).into());
    }
    let new_hash = hash_password(&payload.new_password)?;
    state.db.update_user_password(auth.user.id, &new_hash)?;
    state.db.delete_user_sessions(auth.user.id)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "auth.change_password",
        "user",
        "修改管理员密码并注销其他会话",
    );
    Ok(Json(ApiResponse::success(true)))
}

pub async fn totp_setup(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ServerResult<Json<ApiResponse<TotpSetupResponse>>> {
    let secret = generate_totp_secret();
    let totp = build_totp(&secret, &auth.user.username)?;
    let url = totp.get_url();
    // Save pending secret (not enabled yet until verified)
    state
        .db
        .update_user_totp(auth.user.id, Some(&secret), auth.user.totp_enabled)?;
    Ok(Json(ApiResponse::success(TotpSetupResponse {
        secret,
        otpauth_url: url,
    })))
}

pub async fn totp_enable(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<TotpVerifyRequest>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    let fresh = state
        .db
        .find_user_by_id(auth.user.id)?
        .ok_or(AppError::Unauthorized)?;
    let secret = fresh
        .totp_secret
        .ok_or_else(|| AppError::BadRequest("请先生成 2FA 密钥".to_string()))?;
    if !verify_totp_code(&secret, &fresh.username, &payload.code) {
        return Err(AppError::BadRequest("验证码校验失败，请检查系统时间与动态码".to_string()).into());
    }
    state
        .db
        .update_user_totp(auth.user.id, Some(&secret), true)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "auth.totp_enable",
        "user",
        "开启 TOTP 2FA 双因素认证",
    );
    Ok(Json(ApiResponse::success(true)))
}

pub async fn totp_disable(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<TotpVerifyRequest>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    if !auth.user.totp_enabled {
        return Ok(Json(ApiResponse::success(true)));
    }
    let secret = auth
        .user
        .totp_secret
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("未配置 2FA".to_string()))?;
    if !verify_totp_code(secret, &auth.user.username, &payload.code) {
        return Err(AppError::BadRequest("2FA 验证码错误".to_string()).into());
    }
    state.db.update_user_totp(auth.user.id, None, false)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "auth.totp_disable",
        "user",
        "关闭 TOTP 2FA 双因素认证",
    );
    Ok(Json(ApiResponse::success(true)))
}

pub async fn update_security_settings(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<UpdateSecurityRequest>,
) -> ServerResult<Json<ApiResponse<String>>> {
    let cleaned = payload.security_entry.trim().trim_matches('/').to_string();
    if cleaned.len() < 4 || cleaned.len() > 64 {
        return Err(AppError::BadRequest("安全入口长度必须在 4~64 个字符之间".to_string()).into());
    }
    if !cleaned
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(
            AppError::BadRequest("安全入口仅允许字母、数字、下划线和连字符".to_string()).into(),
        );
    }
    let normalized = format!("/{}", cleaned);
    state.db.set_setting("security_entry", &normalized)?;
    state.update_entry(normalized.clone());
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "settings.security_entry",
        "panel",
        &format!("安全入口更新为 {}", normalized),
    );
    Ok(Json(ApiResponse::success(normalized)))
}
