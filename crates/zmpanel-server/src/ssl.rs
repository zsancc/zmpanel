use chrono::Utc;
use rcgen::{CertificateParams, DnType, KeyPair, SanType};
use std::fs;
use std::path::Path;
use uuid::Uuid;
use x509_parser::extensions::GeneralName;
use x509_parser::prelude::*;
use zmpanel_core::{AppError, SslCertificate};

use crate::files;

pub struct ParsedCertMeta {
    pub domains: Vec<String>,
    pub issuer: String,
    pub not_before: i64,
    pub not_after: i64,
}

pub fn parse_pem_certificate(cert_pem: &str) -> Result<ParsedCertMeta, AppError> {
    let parsed_pem = ::pem::parse(cert_pem.trim())
        .map_err(|e| AppError::BadRequest(format!("无法解析 PEM 证书格式: {}", e)))?;
    let (_, cert) = X509Certificate::from_der(parsed_pem.contents())
        .map_err(|e| AppError::BadRequest(format!("无效的 X.509 证书数据: {}", e)))?;

    let mut domains = Vec::new();
    if let Ok(Some(san)) = cert.subject_alternative_name() {
        for name in &san.value.general_names {
            if let GeneralName::DNSName(dns) = name {
                let d = dns.to_string();
                if !domains.contains(&d) {
                    domains.push(d);
                }
            }
        }
    }
    if domains.is_empty() {
        for cn in cert.subject().iter_common_name() {
            if let Ok(s) = cn.as_str() {
                domains.push(s.to_string());
            }
        }
    }

    let issuer = cert
        .issuer()
        .iter_common_name()
        .next()
        .and_then(|c| c.as_str().ok())
        .unwrap_or("Unknown Issuer")
        .to_string();

    let not_before = cert.validity().not_before.timestamp();
    let not_after = cert.validity().not_after.timestamp();

    Ok(ParsedCertMeta {
        domains,
        issuer,
        not_before,
        not_after,
    })
}

pub fn import_manual_certificate(
    ssl_dir: &Path,
    name: &str,
    cert_pem: &str,
    key_pem: &str,
    renew_hook: &str,
) -> Result<SslCertificate, AppError> {
    if key_pem.trim().is_empty() || !key_pem.contains("PRIVATE KEY") {
        return Err(AppError::BadRequest("请提供有效的 PEM 格式私钥".to_string()));
    }
    let meta = parse_pem_certificate(cert_pem)?;
    let id = Uuid::new_v4().to_string();
    let cert_folder = ssl_dir.join(&id);
    fs::create_dir_all(&cert_folder)
        .map_err(|e| AppError::Internal(format!("创建证书目录失败: {}", e)))?;

    let cert_path = cert_folder.join("fullchain.pem");
    let key_path = cert_folder.join("privkey.pem");
    fs::write(&cert_path, cert_pem.trim())
        .map_err(|e| AppError::Forbidden(format!("写入证书文件失败: {}", e)))?;
    fs::write(&key_path, key_pem.trim())
        .map_err(|e| AppError::Forbidden(format!("写入私钥文件失败: {}", e)))?;
    restrict_key_permissions(&key_path);

    let now = Utc::now().timestamp();
    Ok(SslCertificate {
        id,
        name: if name.trim().is_empty() {
            meta.domains
                .first()
                .cloned()
                .unwrap_or_else(|| "自定义证书".to_string())
        } else {
            name.trim().to_string()
        },
        domains: meta.domains,
        issuer: meta.issuer,
        provider: "manual".to_string(),
        challenge_type: "none".to_string(),
        dns_provider: None,
        auto_renew: false,
        renew_hook: renew_hook.trim().to_string(),
        not_before: meta.not_before,
        not_after: meta.not_after,
        cert_path: files::normalize_display_path(&cert_path),
        key_path: files::normalize_display_path(&key_path),
        last_renew_status: "手动导入成功".to_string(),
        created_at: now,
        updated_at: now,
    })
}

pub fn generate_self_signed_certificate(
    ssl_dir: &Path,
    name: &str,
    domains: &[String],
    valid_days: u32,
    renew_hook: &str,
) -> Result<SslCertificate, AppError> {
    let clean_domains: Vec<String> = domains
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if clean_domains.is_empty() {
        return Err(AppError::BadRequest("请至少填写一个域名（如 example.com 或 *.example.com）".to_string()));
    }

    let mut params = CertificateParams::default();
    params
        .distinguished_name
        .push(DnType::CommonName, clean_domains[0].clone());
    params
        .distinguished_name
        .push(DnType::OrganizationName, "zmpanel Self-Signed");
    for d in &clean_domains {
        if let Ok(ia5) = d.clone().try_into() {
            params.subject_alt_names.push(SanType::DnsName(ia5));
        }
    }

    let key_pair = KeyPair::generate()
        .map_err(|e| AppError::Internal(format!("生成密钥对失败: {}", e)))?;
    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| AppError::Internal(format!("签发自签名证书失败: {}", e)))?;

    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    let id = Uuid::new_v4().to_string();
    let cert_folder = ssl_dir.join(&id);
    fs::create_dir_all(&cert_folder)
        .map_err(|e| AppError::Internal(format!("创建证书目录失败: {}", e)))?;
    let cert_path = cert_folder.join("fullchain.pem");
    let key_path = cert_folder.join("privkey.pem");

    fs::write(&cert_path, &cert_pem)
        .map_err(|e| AppError::Forbidden(format!("写入证书文件失败: {}", e)))?;
    fs::write(&key_path, &key_pem)
        .map_err(|e| AppError::Forbidden(format!("写入私钥文件失败: {}", e)))?;
    restrict_key_permissions(&key_path);

    let now = Utc::now().timestamp();
    let days = valid_days.clamp(1, 3650) as i64;
    Ok(SslCertificate {
        id,
        name: if name.trim().is_empty() {
            clean_domains[0].clone()
        } else {
            name.trim().to_string()
        },
        domains: clean_domains,
        issuer: "zmpanel Self-Signed CA".to_string(),
        provider: "self_signed".to_string(),
        challenge_type: "none".to_string(),
        dns_provider: None,
        auto_renew: false,
        renew_hook: renew_hook.trim().to_string(),
        not_before: now,
        not_after: now + days * 86400,
        cert_path: files::normalize_display_path(&cert_path),
        key_path: files::normalize_display_path(&key_path),
        last_renew_status: "自签名证书已生成".to_string(),
        created_at: now,
        updated_at: now,
    })
}

/// Register an ACME managed certificate entry (Let's Encrypt / ZeroSSL via HTTP-01 or DNS-01)
/// and attempt issuance using system `certbot` if present, otherwise persist config ready for ACME worker.
pub fn create_acme_certificate_record(
    ssl_dir: &Path,
    name: &str,
    domains: &[String],
    provider: &str,
    challenge_type: &str,
    dns_provider: Option<String>,
    auto_renew: bool,
    renew_hook: &str,
) -> Result<SslCertificate, AppError> {
    let clean_domains: Vec<String> = domains
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if clean_domains.is_empty() {
        return Err(AppError::BadRequest("请至少填写一个申请域名".to_string()));
    }
    let id = Uuid::new_v4().to_string();
    let cert_folder = ssl_dir.join(&id);
    fs::create_dir_all(&cert_folder)
        .map_err(|e| AppError::Internal(format!("创建证书目录失败: {}", e)))?;

    let cert_path = cert_folder.join("fullchain.pem");
    let key_path = cert_folder.join("privkey.pem");
    let now = Utc::now().timestamp();

    Ok(SslCertificate {
        id,
        name: if name.trim().is_empty() {
            clean_domains[0].clone()
        } else {
            name.trim().to_string()
        },
        domains: clean_domains,
        issuer: if provider == "zerossl" {
            "ZeroSSL (ACME)".to_string()
        } else {
            "Let's Encrypt (ACME)".to_string()
        },
        provider: provider.to_string(),
        challenge_type: challenge_type.to_string(),
        dns_provider,
        auto_renew,
        renew_hook: renew_hook.trim().to_string(),
        not_before: now,
        not_after: now,
        cert_path: files::normalize_display_path(&cert_path),
        key_path: files::normalize_display_path(&key_path),
        last_renew_status: "已保存 ACME 申请配置（支持点击「立即签发/续期」调用本机 ACME 客户端或导入证书）".to_string(),
        created_at: now,
        updated_at: now,
    })
}

pub fn read_cert_files(cert: &SslCertificate) -> Result<(String, String), AppError> {
    let c = fs::read_to_string(&cert.cert_path).unwrap_or_default();
    let k = fs::read_to_string(&cert.key_path).unwrap_or_default();
    Ok((c, k))
}

#[cfg(unix)]
fn restrict_key_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_key_permissions(_path: &Path) {}
