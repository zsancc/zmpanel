use chrono::Utc;
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use sysinfo::System;
use tar::{Archive as TarArchive, Builder as TarBuilder, Header};
use uuid::Uuid;
use zmpanel_core::{
    AppError, BackupItem, BackupManifest, DockerProject, SslCertificate, WebsiteRecord,
};

use crate::db::Database;
use crate::files;

#[derive(Debug, Clone, Deserialize)]
pub struct CreateBackupRequest {
    pub name: Option<String>,
    pub website_ids: Vec<String>,
    pub docker_project_ids: Vec<String>,
    pub ssl_cert_ids: Vec<String>,
    pub custom_dirs: Vec<String>,
    pub include_panel_db: bool,
    pub target_repo: Option<String>, // local (stage 2) / master (stage 4)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BackupBundleMeta {
    manifest: BackupManifest,
    websites: Vec<WebsiteRecord>,
    docker_projects: Vec<DockerProject>,
    ssl_certs: Vec<SslCertificate>,
}

pub fn create_backup_archive(
    data_dir: &Path,
    db: &Database,
    req: &CreateBackupRequest,
) -> Result<BackupItem, AppError> {
    let backups_dir = data_dir.join("backups");
    fs::create_dir_all(&backups_dir)
        .map_err(|e| AppError::Internal(format!("创建备份目录失败: {}", e)))?;

    let mut websites = Vec::new();
    for id in &req.website_ids {
        if let Ok(Some(w)) = db.get_website(id) {
            websites.push(w);
        }
    }
    let mut docker_projects = Vec::new();
    for id in &req.docker_project_ids {
        if let Ok(Some(d)) = db.get_docker_project(id) {
            docker_projects.push(d);
        }
    }
    let mut ssl_certs = Vec::new();
    for id in &req.ssl_cert_ids {
        if let Ok(Some(c)) = db.get_ssl_cert(id) {
            ssl_certs.push(c);
        }
    }

    let mut scopes = Vec::new();
    if !websites.is_empty() {
        scopes.push(format!("网站 ({})", websites.len()));
    }
    if !docker_projects.is_empty() {
        scopes.push(format!("Docker 工作区 ({})", docker_projects.len()));
    }
    if !ssl_certs.is_empty() {
        scopes.push(format!("SSL 证书 ({})", ssl_certs.len()));
    }
    if !req.custom_dirs.is_empty() {
        scopes.push(format!("自定义目录 ({})", req.custom_dirs.len()));
    }
    if req.include_panel_db {
        scopes.push("面板数据库".to_string());
    }
    if scopes.is_empty() {
        return Err(AppError::BadRequest(
            "请至少勾选一项要备份的内容（网站、Docker 工作区、SSL、目录或面板数据）".to_string(),
        ));
    }

    let now = Utc::now();
    let id = Uuid::new_v4().to_string();
    let label = req
        .name
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("backup-{}", now.format("%Y%m%d-%H%M%S")));

    let safe_filename: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let archive_path = backups_dir.join(format!("{}-{}.tar.gz", safe_filename, &id[..8]));

    let manifest = BackupManifest {
        format_version: 1,
        created_at: now.timestamp(),
        source_hostname: System::host_name().unwrap_or_else(|| "localhost".to_string()),
        panel_version: env!("CARGO_PKG_VERSION").to_string(),
        scopes: scopes.clone(),
        website_ids: websites.iter().map(|w| w.id.clone()).collect(),
        docker_project_ids: docker_projects.iter().map(|d| d.id.clone()).collect(),
        ssl_cert_ids: ssl_certs.iter().map(|c| c.id.clone()).collect(),
        custom_dirs: req.custom_dirs.clone(),
        include_panel_db: req.include_panel_db,
    };

    let bundle_meta = BackupBundleMeta {
        manifest,
        websites: websites.clone(),
        docker_projects: docker_projects.clone(),
        ssl_certs: ssl_certs.clone(),
    };

    let meta_bytes = serde_json::to_vec_pretty(&bundle_meta)
        .map_err(|e| AppError::Internal(format!("序列化备份清单失败: {}", e)))?;

    {
        let out_file = File::create(&archive_path)
            .map_err(|e| AppError::Forbidden(format!("创建备份归档失败: {}", e)))?;
        let gz = GzEncoder::new(out_file, Compression::default());
        let mut tar = TarBuilder::new(gz);

        // 1. Write manifest.json
        append_bytes(&mut tar, "manifest.json", &meta_bytes)?;

        // 2. Websites root_dir & nginx site configs
        for w in &websites {
            let root = PathBuf::from(&w.root_dir);
            if root.is_dir() {
                let target_prefix = format!("websites/{}/files", w.id);
                let _ = tar.append_dir_all(&target_prefix, &root);
            }
            let conf = data_dir.join("nginx").join("sites").join(format!("{}.conf", w.id));
            if conf.is_file() {
                let _ = tar.append_path_with_name(&conf, format!("websites/{}/nginx.conf", w.id));
            }
        }

        // 3. Docker project workspaces
        for d in &docker_projects {
            let work = PathBuf::from(&d.work_dir);
            if work.is_dir() {
                let target_prefix = format!("docker/{}", d.id);
                let _ = tar.append_dir_all(&target_prefix, &work);
            }
        }

        // 4. SSL certificates
        for c in &ssl_certs {
            let cp = PathBuf::from(&c.cert_path);
            let kp = PathBuf::from(&c.key_path);
            if cp.is_file() {
                let _ = tar.append_path_with_name(&cp, format!("ssl/{}/fullchain.pem", c.id));
            }
            if kp.is_file() {
                let _ = tar.append_path_with_name(&kp, format!("ssl/{}/privkey.pem", c.id));
            }
        }

        // 5. Custom directories
        for (idx, raw) in req.custom_dirs.iter().enumerate() {
            if let Ok(p) = files::resolve_safe_path(raw)
                && p.exists()
            {
                if p.is_dir() {
                    let _ = tar.append_dir_all(format!("custom/{}", idx), &p);
                } else {
                    let fname = p
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("file.bin");
                    let _ = tar.append_path_with_name(&p, format!("custom/{}/{}", idx, fname));
                }
            }
        }

        // 6. Optional panel database snapshot
        if req.include_panel_db {
            let db_file = data_dir.join("zmpanel.db");
            if db_file.is_file() {
                let _ = tar.append_path_with_name(&db_file, "panel/zmpanel.db");
            }
        }

        let gz = tar
            .into_inner()
            .map_err(|e| AppError::Internal(format!("完成归档写入失败: {}", e)))?;
        gz.finish()
            .map_err(|e| AppError::Internal(format!("完成 gzip 压缩失败: {}", e)))?;
    }

    let size_bytes = fs::metadata(&archive_path).map(|m| m.len()).unwrap_or(0);
    let sha256 = compute_file_sha256(&archive_path)?;

    let item = BackupItem {
        id,
        name: label,
        file_path: files::normalize_display_path(&archive_path),
        size_bytes,
        sha256,
        target_repo: req
            .target_repo
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "local".to_string()),
        scopes: scopes.clone(),
        summary: scopes.join(" · "),
        created_at: now.timestamp(),
    };

    db.insert_backup(&item)
        .map_err(|e| AppError::Internal(format!("保存备份记录失败: {}", e)))?;
    Ok(item)
}

pub fn restore_backup_archive(
    backup: &BackupItem,
    data_dir: &Path,
    db: &Database,
    restore_dir_override: Option<&str>,
) -> Result<String, AppError> {
    let archive_path = PathBuf::from(&backup.file_path);
    if !archive_path.is_file() {
        return Err(AppError::NotFound("备份归档文件不存在".to_string()));
    }

    // Verify SHA256 integrity before extracting
    let actual_sha = compute_file_sha256(&archive_path)?;
    if !backup.sha256.is_empty() && actual_sha != backup.sha256 {
        return Err(AppError::Forbidden(
            "备份包 SHA256 校验不匹配，文件可能已损坏或被篡改，已中止恢复".to_string(),
        ));
    }

    let extract_root = match restore_dir_override {
        Some(dir) if !dir.trim().is_empty() => files::resolve_safe_path(dir)?,
        _ => data_dir.join("restored").join(&backup.id[..8]),
    };
    fs::create_dir_all(&extract_root)
        .map_err(|e| AppError::Forbidden(format!("创建恢复目录失败: {}", e)))?;
    let canon_root = fs::canonicalize(&extract_root)
        .map_err(|e| AppError::Internal(format!("规范化恢复目录失败: {}", e)))?;

    let file = File::open(&archive_path)
        .map_err(|e| AppError::Forbidden(format!("打开备份包失败: {}", e)))?;
    let gz = GzDecoder::new(file);
    let mut tar = TarArchive::new(gz);

    for entry_res in tar
        .entries()
        .map_err(|e| AppError::BadRequest(format!("读取备份包失败: {}", e)))?
    {
        let mut entry = entry_res.map_err(|e| AppError::Internal(e.to_string()))?;
        let rel = entry
            .path()
            .map_err(|e| AppError::BadRequest(e.to_string()))?
            .into_owned();
        if rel.is_absolute()
            || rel
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
        {
            return Err(AppError::Forbidden(
                "备份包包含非法路径穿越条目，已中止恢复".to_string(),
            ));
        }
        let dst = canon_root.join(&rel);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| AppError::Internal(e.to_string()))?;
        }
        entry
            .unpack(&dst)
            .map_err(|e| AppError::Internal(format!("解压备份条目失败: {}", e)))?;
    }

    // Re-import metadata records if manifest.json exists
    let manifest_path = canon_root.join("manifest.json");
    let mut imported_sites = 0usize;
    let mut imported_docker = 0usize;
    let mut imported_ssl = 0usize;

    if manifest_path.is_file()
        && let Ok(raw) = fs::read(&manifest_path)
        && let Ok(meta) = serde_json::from_slice::<BackupBundleMeta>(&raw)
    {
        for w in meta.websites {
            if db.upsert_website(&w).is_ok() {
                imported_sites += 1;
            }
        }
        for d in meta.docker_projects {
            if db.upsert_docker_project(&d).is_ok() {
                imported_docker += 1;
            }
        }
        for c in meta.ssl_certs {
            if db.upsert_ssl_cert(&c, None).is_ok() {
                imported_ssl += 1;
            }
        }
    }

    Ok(format!(
        "恢复完成：文件已解压至 {}（已还原记录：网站 {}、Docker {}、SSL {}）",
        files::normalize_display_path(&extract_root),
        imported_sites,
        imported_docker,
        imported_ssl
    ))
}

fn append_bytes<W: std::io::Write>(
    tar: &mut TarBuilder<W>,
    path: &str,
    data: &[u8],
) -> Result<(), AppError> {
    let mut header = Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append_data(&mut header, path, data)
        .map_err(|e| AppError::Internal(format!("写入归档条目 {} 失败: {}", path, e)))
}

fn compute_file_sha256(path: &Path) -> Result<String, AppError> {
    let mut f = File::open(path).map_err(|e| AppError::Internal(format!("读取校验文件失败: {}", e)))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 32768];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| AppError::Internal(format!("计算校验和失败: {}", e)))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}
