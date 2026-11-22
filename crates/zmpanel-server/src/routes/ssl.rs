use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use zmpanel_core::{ApiResponse, AppError, SslCertificate};

use crate::auth::AuthUser;
use crate::error::ServerResult;
use crate::ssl;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ImportCertRequest {
    pub name: Option<String>,
    pub cert_pem: String,
    pub key_pem: String,
    pub renew_hook: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SelfSignedRequest {
    pub name: Option<String>,
    pub domains: Vec<String>,
    pub valid_days: Option<u32>,
    pub renew_hook: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AcmeCreateRequest {
    pub name: Option<String>,
    pub domains: Vec<String>,
    pub provider: Option<String>,       // letsencrypt / zerossl
    pub challenge_type: Option<String>, // http01 / dns01
    pub dns_provider: Option<String>,
    pub dns_credentials_json: Option<String>,
    pub auto_renew: Option<bool>,
    pub renew_hook: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CertPemExport {
    pub cert_pem: String,
    pub key_pem: String,
}

pub async fn list_certs(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<Vec<SslCertificate>>>> {
    let certs = state.db.list_ssl_certs()?;
    Ok(Json(ApiResponse::success(certs)))
}

pub async fn import_cert(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<ImportCertRequest>,
) -> ServerResult<Json<ApiResponse<SslCertificate>>> {
    let ssl_dir = state.config_snapshot().ssl_dir();
    let name = payload.name.unwrap_or_default();
    let hook = payload.renew_hook.unwrap_or_default();
    let cert = tokio::task::spawn_blocking(move || {
        ssl::import_manual_certificate(&ssl_dir, &name, &payload.cert_pem, &payload.key_pem, &hook)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))??;

    state.db.upsert_ssl_cert(&cert, None)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "ssl.import",
        &cert.name,
        &format!("域名: {}", cert.domains.join(", ")),
    );
    Ok(Json(ApiResponse::success(cert)))
}

pub async fn generate_self_signed(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<SelfSignedRequest>,
) -> ServerResult<Json<ApiResponse<SslCertificate>>> {
    let ssl_dir = state.config_snapshot().ssl_dir();
    let name = payload.name.unwrap_or_default();
    let days = payload.valid_days.unwrap_or(365);
    let hook = payload.renew_hook.unwrap_or_default();
    let cert = tokio::task::spawn_blocking(move || {
        ssl::generate_self_signed_certificate(&ssl_dir, &name, &payload.domains, days, &hook)
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))??;

    state.db.upsert_ssl_cert(&cert, None)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "ssl.self_signed",
        &cert.name,
        &format!("签发自签名证书 ({} 天)", days),
    );
    Ok(Json(ApiResponse::success(cert)))
}

pub async fn create_acme_cert(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<AcmeCreateRequest>,
) -> ServerResult<Json<ApiResponse<SslCertificate>>> {
    let ssl_dir = state.config_snapshot().ssl_dir();
    let name = payload.name.unwrap_or_default();
    let provider = payload
        .provider
        .unwrap_or_else(|| "letsencrypt".to_string());
    let challenge = payload
        .challenge_type
        .unwrap_or_else(|| "dns01".to_string());
    let auto_renew = payload.auto_renew.unwrap_or(true);
    let hook = payload.renew_hook.unwrap_or_default();
    let dns_provider = payload.dns_provider.clone();

    let cert = tokio::task::spawn_blocking(move || {
        ssl::create_acme_certificate_record(
            &ssl_dir,
            &name,
            &payload.domains,
            &provider,
            &challenge,
            dns_provider,
            auto_renew,
            &hook,
        )
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))??;

    state
        .db
        .upsert_ssl_cert(&cert, payload.dns_credentials_json.as_deref())?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "ssl.acme_create",
        &cert.name,
        &format!("{} / {}", cert.provider, cert.challenge_type),
    );
    Ok(Json(ApiResponse::success(cert)))
}

pub async fn export_cert(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<String>,
) -> ServerResult<Json<ApiResponse<CertPemExport>>> {
    let cert = state
        .db
        .get_ssl_cert(&id)?
        .ok_or_else(|| AppError::NotFound("证书不存在".to_string()))?;
    let (cert_pem, key_pem) = ssl::read_cert_files(&cert)?;
    Ok(Json(ApiResponse::success(CertPemExport {
        cert_pem,
        key_pem,
    })))
}

pub async fn delete_cert(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    let folder = state.config_snapshot().ssl_dir().join(&id);
    if folder.is_dir() {
        let _ = std::fs::remove_dir_all(folder);
    }
    state.db.delete_ssl_cert(&id)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "ssl.delete",
        &id,
        "删除 SSL 证书及密钥文件",
    );
    Ok(Json(ApiResponse::success(true)))
}
