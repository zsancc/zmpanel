use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use axum::{
    extract::{FromRequestParts, Request},
    http::{HeaderMap, header, request::Parts},
    middleware::Next,
    response::Response,
};
use chrono::Utc;
use rand::Rng;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use totp_rs::{Algorithm, Secret, TOTP};
use zmpanel_core::AppError;

use crate::db::UserRecord;
use crate::error::ServerError;
use crate::state::AppState;

pub const SESSION_COOKIE_NAME: &str = "zmpanel_sid";

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user: UserRecord,
    pub token_hash: String,
    pub client_ip: String,
}

#[derive(Debug, Default)]
pub struct LoginRateLimiter {
    attempts: Mutex<HashMap<String, (u32, i64)>>,
}

impl LoginRateLimiter {
    pub fn check(&self, ip: &str, max_attempts: u32) -> Result<(), AppError> {
        let mut guard = self.attempts.lock().expect("limiter poisoned");
        let now = Utc::now().timestamp();
        if let Some((count, unlock_at)) = guard.get(ip).copied() {
            if now >= unlock_at {
                guard.remove(ip);
            } else if count >= max_attempts {
                let remain = (unlock_at - now).max(1);
                return Err(AppError::TooManyRequests(format!(
                    "登录失败次数过多，请 {} 秒后再试",
                    remain
                )));
            }
        }
        Ok(())
    }

    pub fn record_failure(&self, ip: &str, lock_minutes: i64) {
        let mut guard = self.attempts.lock().expect("limiter poisoned");
        let now = Utc::now().timestamp();
        let entry = guard.entry(ip.to_string()).or_insert((0, now + lock_minutes * 60));
        if now >= entry.1 {
            *entry = (1, now + lock_minutes * 60);
        } else {
            entry.0 += 1;
            entry.1 = now + lock_minutes * 60;
        }
    }

    pub fn clear(&self, ip: &str) {
        let mut guard = self.attempts.lock().expect("limiter poisoned");
        guard.remove(ip);
    }
}

/// Return freed large temporary buffers (such as Argon2id's 19 MiB work memory)
/// promptly back to the Linux kernel under glibc malloc so RSS drops right after login.
fn reclaim_heap_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        unsafe extern "C" {
            fn malloc_trim(pad: usize) -> i32;
        }
        // SAFETY: `malloc_trim(0)` is a thread-safe glibc heap trimming function with no pointer args.
        unsafe {
            let _ = malloc_trim(0);
        }
    }
}

pub fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    let res = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AppError::Internal(format!("密码哈希失败: {}", e)));
    reclaim_heap_memory();
    res
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    let ok = Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok();
    reclaim_heap_memory();
    ok
}

pub fn generate_random_token(bytes_len: usize) -> String {
    let mut buf = vec![0u8; bytes_len];
    rand::thread_rng().fill(&mut buf[..]);
    hex::encode(buf)
}

pub fn generate_alnum(len: usize) -> String {
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char)
        .collect()
}

pub fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn generate_totp_secret() -> String {
    let mut bytes = vec![0u8; 20];
    rand::thread_rng().fill(&mut bytes[..]);
    Secret::Raw(bytes).to_encoded().to_string()
}

pub fn build_totp(secret_base32: &str, username: &str) -> Result<TOTP, AppError> {
    let raw = Secret::Encoded(secret_base32.to_string())
        .to_bytes()
        .map_err(|e| AppError::Internal(format!("无效的 TOTP 密钥: {}", e)))?;
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        raw,
        Some("zmpanel".to_string()),
        username.to_string(),
    )
    .map_err(|e| AppError::Internal(format!("初始化 TOTP 失败: {}", e)))
}

pub fn verify_totp_code(secret_base32: &str, username: &str, code: &str) -> bool {
    let Ok(totp) = build_totp(secret_base32, username) else {
        return false;
    };
    totp.check_current(code.trim()).unwrap_or(false)
}

pub fn extract_client_ip(headers: &HeaderMap) -> String {
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
        && let Some(first) = xff.split(',').next()
    {
        let ip = first.trim();
        if !ip.is_empty() {
            return ip.to_string();
        }
    }
    if let Some(xri) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        let ip = xri.trim();
        if !ip.is_empty() {
            return ip.to_string();
        }
    }
    "127.0.0.1".to_string()
}

pub fn extract_token_from_headers(headers: &HeaderMap) -> Option<String> {
    if let Some(auth) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        && let Some(bearer) = auth.strip_prefix("Bearer ")
    {
        let t = bearer.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    if let Some(cookie_hdr) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
        for part in cookie_hdr.split(';') {
            let mut kv = part.trim().splitn(2, '=');
            if let (Some(k), Some(v)) = (kv.next(), kv.next())
                && k == SESSION_COOKIE_NAME
                && !v.trim().is_empty()
            {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

pub fn authenticate_token(state: &AppState, raw_token: &str, client_ip: String) -> Result<AuthUser, AppError> {
    let token_hash = sha256_hex(raw_token);
    let user = state
        .db
        .find_session_user(&token_hash)
        .map_err(|e| AppError::Internal(format!("会话查询失败: {}", e)))?
        .ok_or(AppError::Unauthorized)?;
    Ok(AuthUser {
        user,
        token_hash,
        client_ip,
    })
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ServerError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let client_ip = extract_client_ip(&parts.headers);
        let raw_token = extract_token_from_headers(&parts.headers)
            .ok_or(ServerError(AppError::Unauthorized))?;
        authenticate_token(state, &raw_token, client_ip).map_err(ServerError)
    }
}

/// Security entry path guard middleware:
/// Every non-matching path outside `/<security_entry>` returns a stealth 404.
pub async fn security_entry_guard(
    axum::extract::State(state): axum::extract::State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let entry = state.current_entry();
    if entry.is_empty() {
        return next.run(req).await;
    }
    let path = req.uri().path();
    let matched = path == entry || path.starts_with(&format!("{}/", entry));
    if !matched {
        return (
            axum::http::StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "404 Not Found",
        )
            .into_response();
    }
    next.run(req).await
}

use axum::response::IntoResponse;
