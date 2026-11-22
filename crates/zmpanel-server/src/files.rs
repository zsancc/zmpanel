use chrono::Utc;
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use tar::{Archive as TarArchive, Builder as TarBuilder};
use uuid::Uuid;
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};
use zmpanel_core::{AppError, FileEntry, TrashItem};

use crate::db::Database;

const MAX_EDIT_BYTES: u64 = 5 * 1024 * 1024; // 5 MiB

pub fn default_browse_path() -> String {
    #[cfg(windows)]
    {
        std::env::current_dir()
            .map(|p| normalize_display_path(&p))
            .unwrap_or_else(|_| "C:/".to_string())
    }
    #[cfg(not(windows))]
    {
        "/".to_string()
    }
}

pub fn normalize_display_path(p: &Path) -> String {
    let s = p.to_string_lossy().to_string();
    #[cfg(windows)]
    {
        s.strip_prefix(r"\\?\").unwrap_or(&s).replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        s
    }
}

pub fn resolve_safe_path(raw: &str) -> Result<PathBuf, AppError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(PathBuf::from(default_browse_path()));
    }
    if trimmed.contains('\0') {
        return Err(AppError::BadRequest("路径包含非法空字节".to_string()));
    }
    let path = PathBuf::from(trimmed);
    // Reject explicit parent traversal segments after normalization check
    let mut normalized = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(p) => normalized.push(p.as_os_str()),
            Component::RootDir => normalized.push(comp.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(seg) => normalized.push(seg),
        }
    }
    if normalized.as_os_str().is_empty() {
        return Ok(PathBuf::from(default_browse_path()));
    }
    Ok(normalized)
}

pub fn list_directory(raw_path: &str) -> Result<(String, Vec<FileEntry>), AppError> {
    let dir = resolve_safe_path(raw_path)?;
    if !dir.exists() {
        return Err(AppError::NotFound(format!(
            "目录不存在: {}",
            normalize_display_path(&dir)
        )));
    }
    if !dir.is_dir() {
        return Err(AppError::BadRequest("目标路径不是目录".to_string()));
    }

    let mut entries = Vec::new();
    let rd = fs::read_dir(&dir)
        .map_err(|e| AppError::Forbidden(format!("无法读取目录: {}", e)))?;

    for item in rd.flatten() {
        let path = item.path();
        let name = item.file_name().to_string_lossy().to_string();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        let is_symlink = meta.file_type().is_symlink();
        let target_meta = if is_symlink {
            fs::metadata(&path).unwrap_or(meta.clone())
        } else {
            meta.clone()
        };
        let modified = target_meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let (permissions, mode_octal) = format_permissions(&target_meta);

        entries.push(FileEntry {
            name,
            path: normalize_display_path(&path),
            is_dir: target_meta.is_dir(),
            is_symlink,
            size: if target_meta.is_dir() {
                0
            } else {
                target_meta.len()
            },
            modified,
            permissions,
            mode_octal,
        });
    }

    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok((normalize_display_path(&dir), entries))
}

#[cfg(unix)]
fn format_permissions(meta: &fs::Metadata) -> (String, String) {
    use std::os::unix::fs::PermissionsExt;
    let mode = meta.permissions().mode() & 0o777;
    let mut s = String::with_capacity(9);
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 0o7;
        s.push(if bits & 4 != 0 { 'r' } else { '-' });
        s.push(if bits & 2 != 0 { 'w' } else { '-' });
        s.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    (s, format!("{:03o}", mode))
}

#[cfg(not(unix))]
fn format_permissions(meta: &fs::Metadata) -> (String, String) {
    if meta.permissions().readonly() {
        ("r--r--r--".to_string(), "444".to_string())
    } else {
        ("rw-rw-rw-".to_string(), "666".to_string())
    }
}

pub fn read_text_file(raw_path: &str) -> Result<(String, u64), AppError> {
    let path = resolve_safe_path(raw_path)?;
    let meta = fs::metadata(&path)
        .map_err(|_| AppError::NotFound(format!("文件不存在: {}", raw_path)))?;
    if meta.is_dir() {
        return Err(AppError::BadRequest("不能直接以文本方式打开目录".to_string()));
    }
    if meta.len() > MAX_EDIT_BYTES {
        return Err(AppError::BadRequest(format!(
            "文件过大（{} MB），在线编辑限制 5 MB 以内，请下载查看",
            meta.len() / (1024 * 1024)
        )));
    }
    let bytes = fs::read(&path)
        .map_err(|e| AppError::Forbidden(format!("读取文件失败: {}", e)))?;
    // Heuristic binary check: presence of NUL bytes in first 8KB
    let probe_len = bytes.len().min(8192);
    if bytes[..probe_len].contains(&0) {
        return Err(AppError::BadRequest(
            "检测到二进制文件，不支持在线文本编辑".to_string(),
        ));
    }
    let content = String::from_utf8(bytes)
        .map_err(|_| AppError::BadRequest("文件不是有效的 UTF-8 文本编码".to_string()))?;
    Ok((content, meta.len()))
}

pub fn write_text_file(raw_path: &str, content: &str) -> Result<(), AppError> {
    let path = resolve_safe_path(raw_path)?;
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent)
            .map_err(|e| AppError::Forbidden(format!("创建父目录失败: {}", e)))?;
    }
    fs::write(&path, content.as_bytes())
        .map_err(|e| AppError::Forbidden(format!("保存文件失败: {}", e)))?;
    Ok(())
}

pub fn create_entry(raw_path: &str, is_dir: bool) -> Result<(), AppError> {
    let path = resolve_safe_path(raw_path)?;
    if path.exists() {
        return Err(AppError::Conflict("目标路径已存在".to_string()));
    }
    if is_dir {
        fs::create_dir_all(&path)
            .map_err(|e| AppError::Forbidden(format!("创建目录失败: {}", e)))?;
    } else {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| AppError::Forbidden(format!("创建父目录失败: {}", e)))?;
        }
        File::create(&path)
            .map_err(|e| AppError::Forbidden(format!("创建文件失败: {}", e)))?;
    }
    Ok(())
}

pub fn rename_entry(from_raw: &str, to_raw: &str) -> Result<(), AppError> {
    let from = resolve_safe_path(from_raw)?;
    let to = resolve_safe_path(to_raw)?;
    if !from.exists() {
        return Err(AppError::NotFound("源文件或目录不存在".to_string()));
    }
    if to.exists() {
        return Err(AppError::Conflict("目标路径已存在".to_string()));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| AppError::Forbidden(format!("创建目标目录失败: {}", e)))?;
    }
    fs::rename(&from, &to).map_err(|e| AppError::Forbidden(format!("重命名/移动失败: {}", e)))
}

pub fn copy_entry(from_raw: &str, to_raw: &str) -> Result<(), AppError> {
    let from = resolve_safe_path(from_raw)?;
    let to = resolve_safe_path(to_raw)?;
    if !from.exists() {
        return Err(AppError::NotFound("源文件或目录不存在".to_string()));
    }
    if to.exists() {
        return Err(AppError::Conflict("目标路径已存在".to_string()));
    }
    copy_recursive(&from, &to)
}

fn copy_recursive(from: &Path, to: &Path) -> Result<(), AppError> {
    if from.is_dir() {
        fs::create_dir_all(to)
            .map_err(|e| AppError::Forbidden(format!("创建目录失败: {}", e)))?;
        for entry in fs::read_dir(from)
            .map_err(|e| AppError::Forbidden(format!("读取源目录失败: {}", e)))?
        {
            let entry = entry.map_err(|e| AppError::Internal(e.to_string()))?;
            copy_recursive(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| AppError::Forbidden(format!("创建目标目录失败: {}", e)))?;
        }
        fs::copy(from, to).map_err(|e| AppError::Forbidden(format!("复制文件失败: {}", e)))?;
    }
    Ok(())
}

pub fn move_to_trash(raw_path: &str, trash_dir: &Path, db: &Database) -> Result<TrashItem, AppError> {
    let path = resolve_safe_path(raw_path)?;
    if !path.exists() {
        return Err(AppError::NotFound("目标不存在".to_string()));
    }
    let meta = fs::symlink_metadata(&path)
        .map_err(|e| AppError::Forbidden(format!("无法读取元信息: {}", e)))?;
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .ok_or_else(|| AppError::BadRequest("禁止删除根目录".to_string()))?;

    fs::create_dir_all(trash_dir)
        .map_err(|e| AppError::Internal(format!("初始化回收站失败: {}", e)))?;

    let id = Uuid::new_v4().to_string();
    let dest = trash_dir.join(&id);
    if fs::rename(&path, &dest).is_err() {
        // Fallback across filesystems: copy then delete
        copy_recursive(&path, &dest)?;
        if meta.is_dir() {
            fs::remove_dir_all(&path)
                .map_err(|e| AppError::Forbidden(format!("移除原目录失败: {}", e)))?;
        } else {
            fs::remove_file(&path)
                .map_err(|e| AppError::Forbidden(format!("移除原文件失败: {}", e)))?;
        }
    }

    let item = TrashItem {
        id,
        original_path: normalize_display_path(&path),
        name,
        is_dir: meta.is_dir(),
        size: if meta.is_dir() { 0 } else { meta.len() },
        deleted_at: Utc::now().timestamp(),
    };
    db.insert_trash_item(&item)
        .map_err(|e| AppError::Internal(format!("记录回收站失败: {}", e)))?;
    Ok(item)
}

pub fn restore_trash_item(id: &str, trash_dir: &Path, db: &Database) -> Result<String, AppError> {
    let item = db
        .get_trash_item(id)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("回收站条目不存在".to_string()))?;
    let src = trash_dir.join(&item.id);
    if !src.exists() {
        let _ = db.delete_trash_item(id);
        return Err(AppError::NotFound("回收站文件已丢失".to_string()));
    }
    let dst = PathBuf::from(&item.original_path);
    if dst.exists() {
        return Err(AppError::Conflict(format!(
            "原路径 {} 已存在同名文件，请先处理后再还原",
            item.original_path
        )));
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| AppError::Forbidden(format!("创建原目录失败: {}", e)))?;
    }
    if fs::rename(&src, &dst).is_err() {
        copy_recursive(&src, &dst)?;
        if src.is_dir() {
            let _ = fs::remove_dir_all(&src);
        } else {
            let _ = fs::remove_file(&src);
        }
    }
    db.delete_trash_item(id)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(item.original_path)
}

pub fn delete_trash_item(id: &str, trash_dir: &Path, db: &Database) -> Result<(), AppError> {
    let src = trash_dir.join(id);
    if src.exists() {
        if src.is_dir() {
            fs::remove_dir_all(&src)
                .map_err(|e| AppError::Internal(format!("删除回收站目录失败: {}", e)))?;
        } else {
            fs::remove_file(&src)
                .map_err(|e| AppError::Internal(format!("删除回收站文件失败: {}", e)))?;
        }
    }
    db.delete_trash_item(id)
        .map_err(|e| AppError::Internal(e.to_string()))
}

pub fn empty_trash(trash_dir: &Path, db: &Database) -> Result<(), AppError> {
    if trash_dir.exists() {
        fs::remove_dir_all(trash_dir)
            .map_err(|e| AppError::Internal(format!("清空回收站目录失败: {}", e)))?;
        fs::create_dir_all(trash_dir)
            .map_err(|e| AppError::Internal(format!("重建回收站目录失败: {}", e)))?;
    }
    db.clear_trash_items()
        .map_err(|e| AppError::Internal(e.to_string()))
}

pub fn delete_permanently(raw_path: &str) -> Result<(), AppError> {
    let path = resolve_safe_path(raw_path)?;
    if path.file_name().is_none() {
        return Err(AppError::BadRequest("禁止删除根目录".to_string()));
    }
    if !path.exists() {
        return Err(AppError::NotFound("路径不存在".to_string()));
    }
    if path.is_dir() {
        fs::remove_dir_all(&path)
            .map_err(|e| AppError::Forbidden(format!("彻底删除目录失败: {}", e)))
    } else {
        fs::remove_file(&path)
            .map_err(|e| AppError::Forbidden(format!("彻底删除文件失败: {}", e)))
    }
}

#[cfg(unix)]
pub fn chmod_entry(raw_path: &str, mode_octal: &str) -> Result<(), AppError> {
    use std::os::unix::fs::PermissionsExt;
    let path = resolve_safe_path(raw_path)?;
    let mode = u32::from_str_radix(mode_octal.trim(), 8)
        .map_err(|_| AppError::BadRequest("权限格式错误，应为 3~4 位八进制数（如 755）".to_string()))?;
    if mode > 0o7777 {
        return Err(AppError::BadRequest("权限超出有效范围".to_string()));
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(mode))
        .map_err(|e| AppError::Forbidden(format!("修改权限失败: {}", e)))
}

#[cfg(not(unix))]
pub fn chmod_entry(_raw_path: &str, _mode_octal: &str) -> Result<(), AppError> {
    Err(AppError::BadRequest(
        "POSIX 权限修改仅支持 Linux 环境".to_string(),
    ))
}

pub fn compress_entries(sources: &[String], dest_raw: &str) -> Result<String, AppError> {
    if sources.is_empty() {
        return Err(AppError::BadRequest("请至少选择一个待压缩文件或目录".to_string()));
    }
    let dest = resolve_safe_path(dest_raw)?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| AppError::Forbidden(format!("创建目标目录失败: {}", e)))?;
    }
    let lower = dest.to_string_lossy().to_lowercase();
    let src_paths: Vec<PathBuf> = sources
        .iter()
        .map(|s| resolve_safe_path(s))
        .collect::<Result<_, _>>()?;
    for p in &src_paths {
        if !p.exists() {
            return Err(AppError::NotFound(format!(
                "待压缩路径不存在: {}",
                normalize_display_path(p)
            )));
        }
    }

    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        let tar_gz = File::create(&dest)
            .map_err(|e| AppError::Forbidden(format!("创建压缩包失败: {}", e)))?;
        let enc = GzEncoder::new(tar_gz, Compression::default());
        let mut tar = TarBuilder::new(enc);
        for src in &src_paths {
            let name = src
                .file_name()
                .ok_or_else(|| AppError::BadRequest("无效源路径".to_string()))?;
            if src.is_dir() {
                tar.append_dir_all(name, src)
                    .map_err(|e| AppError::Internal(format!("写入目录到 tar.gz 失败: {}", e)))?;
            } else {
                tar.append_path_with_name(src, name)
                    .map_err(|e| AppError::Internal(format!("写入文件到 tar.gz 失败: {}", e)))?;
            }
        }
        let enc = tar
            .into_inner()
            .map_err(|e| AppError::Internal(format!("完成 tar 归档失败: {}", e)))?;
        enc.finish()
            .map_err(|e| AppError::Internal(format!("完成 gzip 压缩失败: {}", e)))?;
    } else if lower.ends_with(".zip") {
        let file = File::create(&dest)
            .map_err(|e| AppError::Forbidden(format!("创建 zip 失败: {}", e)))?;
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for src in &src_paths {
            let base = src.parent().unwrap_or_else(|| Path::new(""));
            add_path_to_zip(&mut zip, src, base, options)?;
        }
        zip.finish()
            .map_err(|e| AppError::Internal(format!("完成 zip 压缩失败: {}", e)))?;
    } else {
        return Err(AppError::BadRequest(
            "仅支持 .zip 或 .tar.gz / .tgz 格式压缩".to_string(),
        ));
    }

    Ok(normalize_display_path(&dest))
}

fn add_path_to_zip(
    zip: &mut ZipWriter<File>,
    path: &Path,
    base: &Path,
    options: SimpleFileOptions,
) -> Result<(), AppError> {
    let rel = path.strip_prefix(base).unwrap_or(path);
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    if path.is_dir() {
        let dir_name = format!("{}/", rel_str.trim_end_matches('/'));
        zip.add_directory(&dir_name, options)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        for entry in fs::read_dir(path)
            .map_err(|e| AppError::Forbidden(format!("读取目录失败: {}", e)))?
        {
            let entry = entry.map_err(|e| AppError::Internal(e.to_string()))?;
            add_path_to_zip(zip, &entry.path(), base, options)?;
        }
    } else {
        zip.start_file(&rel_str, options)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let mut f = File::open(path)
            .map_err(|e| AppError::Forbidden(format!("打开文件失败: {}", e)))?;
        let mut buf = [0u8; 32768];
        loop {
            let n = f.read(&mut buf).map_err(|e| AppError::Internal(e.to_string()))?;
            if n == 0 {
                break;
            }
            zip.write_all(&buf[..n])
                .map_err(|e| AppError::Internal(e.to_string()))?;
        }
    }
    Ok(())
}

pub fn extract_archive(archive_raw: &str, dest_dir_raw: &str) -> Result<String, AppError> {
    let archive_path = resolve_safe_path(archive_raw)?;
    let dest_dir = resolve_safe_path(dest_dir_raw)?;
    if !archive_path.is_file() {
        return Err(AppError::NotFound("压缩包文件不存在".to_string()));
    }
    fs::create_dir_all(&dest_dir)
        .map_err(|e| AppError::Forbidden(format!("创建解压目录失败: {}", e)))?;
    let canon_dest = fs::canonicalize(&dest_dir)
        .map_err(|e| AppError::Internal(format!("解析解压目录失败: {}", e)))?;

    let lower = archive_path.to_string_lossy().to_lowercase();
    if lower.ends_with(".zip") {
        let file = File::open(&archive_path)
            .map_err(|e| AppError::Forbidden(format!("打开压缩包失败: {}", e)))?;
        let mut archive = ZipArchive::new(file)
            .map_err(|e| AppError::BadRequest(format!("无效的 zip 文件: {}", e)))?;
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| AppError::Internal(e.to_string()))?;
            let Some(enclosed) = entry.enclosed_name() else {
                return Err(AppError::Forbidden(
                    "检测到恶意压缩包路径穿越（Zip Slip），已拦截".to_string(),
                ));
            };
            let out_path = canon_dest.join(enclosed);
            if entry.is_dir() {
                fs::create_dir_all(&out_path)
                    .map_err(|e| AppError::Internal(e.to_string()))?;
            } else {
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| AppError::Internal(e.to_string()))?;
                }
                let mut outfile = File::create(&out_path)
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                std::io::copy(&mut entry, &mut outfile)
                    .map_err(|e| AppError::Internal(e.to_string()))?;
            }
        }
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        let file = File::open(&archive_path)
            .map_err(|e| AppError::Forbidden(format!("打开压缩包失败: {}", e)))?;
        let gz = GzDecoder::new(file);
        let mut archive = TarArchive::new(gz);
        for entry_res in archive
            .entries()
            .map_err(|e| AppError::BadRequest(format!("无效的 tar.gz 文件: {}", e)))?
        {
            let mut entry = entry_res.map_err(|e| AppError::Internal(e.to_string()))?;
            let rel = entry
                .path()
                .map_err(|e| AppError::BadRequest(e.to_string()))?
                .into_owned();
            // Prevent tar path traversal
            if rel.is_absolute()
                || rel
                    .components()
                    .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
            {
                return Err(AppError::Forbidden(
                    "检测到恶意 tar 路径穿越，已拦截".to_string(),
                ));
            }
            let out_path = canon_dest.join(&rel);
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| AppError::Internal(e.to_string()))?;
            }
            entry
                .unpack(&out_path)
                .map_err(|e| AppError::Internal(format!("解压条目失败: {}", e)))?;
        }
    } else {
        return Err(AppError::BadRequest(
            "仅支持解压 .zip 或 .tar.gz / .tgz 格式".to_string(),
        ));
    }

    Ok(normalize_display_path(&dest_dir))
}
