use std::fs::{self, OpenOptions};
use std::path::Path;
use zmpanel_core::{AppError, LogSourceItem};

use crate::db::Database;
use crate::files;

pub fn list_log_sources(data_dir: &Path, db: &Database) -> Vec<LogSourceItem> {
    let mut sources = Vec::new();

    // Standard Linux system / SSH / Nginx logs
    let candidates = [
        ("sys-auth", "SSH / 认证日志 (auth.log)", "ssh", "/var/log/auth.log"),
        ("sys-messages", "Alpine / 系统消息 (messages)", "system", "/var/log/messages"),
        ("sys-syslog", "系统日志 (syslog)", "system", "/var/log/syslog"),
        ("sys-kern", "内核与防火墙日志 (kern.log)", "system", "/var/log/kern.log"),
        ("nginx-access", "Nginx 访问日志", "nginx", "/var/log/nginx/access.log"),
        ("nginx-error", "Nginx 错误日志", "nginx", "/var/log/nginx/error.log"),
    ];

    for (id, name, cat, path_str) in candidates {
        let p = Path::new(path_str);
        let (exists, size_bytes) = fs::metadata(p)
            .map(|m| (m.is_file(), m.len()))
            .unwrap_or((false, 0));
        sources.push(LogSourceItem {
            id: id.to_string(),
            name: name.to_string(),
            category: cat.to_string(),
            path: path_str.to_string(),
            exists,
            size_bytes,
        });
    }

    // Managed website logs
    if let Ok(sites) = db.list_websites() {
        let site_log_dir = data_dir.join("logs").join("sites");
        for site in sites {
            let p = site_log_dir.join(format!("{}.log", site.id));
            let (exists, size_bytes) = fs::metadata(&p)
                .map(|m| (m.is_file(), m.len()))
                .unwrap_or((false, 0));
            sources.push(LogSourceItem {
                id: format!("site-{}", site.id),
                name: format!("网站日志: {} ({})", site.name, site.kind.as_str()),
                category: "site".to_string(),
                path: files::normalize_display_path(&p),
                exists,
                size_bytes,
            });
        }
    }

    sources
}

pub fn tail_log_file(
    raw_path: &str,
    lines: usize,
    keyword: Option<&str>,
) -> Result<(String, u64, usize), AppError> {
    let path = files::resolve_safe_path(raw_path)?;
    if !path.is_file() {
        return Err(AppError::NotFound(format!(
            "日志文件不存在: {}",
            files::normalize_display_path(&path)
        )));
    }
    let meta = fs::metadata(&path)
        .map_err(|e| AppError::Forbidden(format!("无法读取日志元信息: {}", e)))?;
    let bytes = fs::read(&path)
        .map_err(|e| AppError::Forbidden(format!("无法读取日志内容: {}", e)))?;

    // Read only tail 2 MiB if file is huge, keeping RAM safe on 256MB VPS
    let slice = if bytes.len() > 2 * 1024 * 1024 {
        &bytes[bytes.len() - 2 * 1024 * 1024..]
    } else {
        &bytes[..]
    };
    let text = String::from_utf8_lossy(slice);
    let kw = keyword.map(|k| k.trim().to_lowercase()).unwrap_or_default();

    let mut matched: Vec<&str> = text
        .lines()
        .filter(|line| kw.is_empty() || line.to_lowercase().contains(&kw))
        .collect();
    let total_matched = matched.len();
    let cap = lines.clamp(20, 3000);
    if matched.len() > cap {
        matched = matched[matched.len() - cap..].to_vec();
    }

    Ok((matched.join("\n"), meta.len(), total_matched))
}

/// Safely truncate a log file in-place without unlinking its inode (keeps open handles in nginx/apps valid).
pub fn truncate_log_file(raw_path: &str) -> Result<(), AppError> {
    let path = files::resolve_safe_path(raw_path)?;
    if !path.is_file() {
        return Err(AppError::NotFound("日志文件不存在".to_string()));
    }
    OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&path)
        .map_err(|e| AppError::Forbidden(format!("清空日志文件失败: {}", e)))?;
    Ok(())
}
