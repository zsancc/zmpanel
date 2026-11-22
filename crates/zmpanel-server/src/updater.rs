use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::process::Command;
use zmpanel_core::{AppError, UpdateManifest};

/// Fetch user-configured self-update manifest JSON (zero hardcoded telemetry; only called on explicit user action).
pub fn fetch_manifest(manifest_url: &str) -> Result<UpdateManifest, AppError> {
    let trimmed = manifest_url.trim();
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err(AppError::BadRequest(
            "更新清单 URL 必须以 http:// 或 https:// 开头".to_string(),
        ));
    }
    let out = Command::new("curl")
        .args(["-fsSL", "--max-time", "20", trimmed])
        .output()
        .map_err(|e| AppError::Internal(format!("获取更新清单失败 (需 curl): {}", e)))?;
    if !out.status.success() {
        return Err(AppError::BadRequest(format!(
            "无法拉取更新清单: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    serde_json::from_slice::<UpdateManifest>(&out.stdout)
        .map_err(|e| AppError::BadRequest(format!("解析更新清单 JSON 失败: {}", e)))
}

/// Download, verify SHA256, smoke-test `--version`, back up `.bak`, and atomically replace binary.
pub fn perform_self_update(
    updates_dir: &Path,
    download_url: &str,
    expected_sha256: &str,
    target_version: &str,
) -> Result<String, AppError> {
    let url = download_url.trim();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(AppError::BadRequest(
            "下载地址必须以 http:// 或 https:// 开头".to_string(),
        ));
    }
    let expected = expected_sha256.trim().to_lowercase();
    if expected.len() != 64 || !expected.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::BadRequest(
            "必须提供有效的 64 位 SHA256 校验值以确保二进制完整性".to_string(),
        ));
    }

    fs::create_dir_all(updates_dir)
        .map_err(|e| AppError::Internal(format!("创建更新目录失败: {}", e)))?;
    let staging = updates_dir.join("zmpanel.new");
    let _ = fs::remove_file(&staging);

    let dl = Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "180",
            "-o",
            &staging.to_string_lossy(),
            url,
        ])
        .output()
        .map_err(|e| AppError::Internal(format!("下载更新包失败: {}", e)))?;
    if !dl.status.success() {
        let _ = fs::remove_file(&staging);
        return Err(AppError::BadRequest(format!(
            "下载更新包失败: {}",
            String::from_utf8_lossy(&dl.stderr).trim()
        )));
    }

    let actual_sha = sha256_file(&staging)?;
    if actual_sha != expected {
        let _ = fs::remove_file(&staging);
        return Err(AppError::Forbidden(format!(
            "SHA256 校验失败！期望 {}，实际 {}，已删除可疑文件并中止更新",
            expected, actual_sha
        )));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staging, fs::Permissions::from_mode(0o755))
            .map_err(|e| AppError::Internal(format!("设置可执行权限失败: {}", e)))?;
    }

    // Smoke-test the downloaded binary before replacing current executable
    let probe = Command::new(&staging)
        .arg("--version")
        .output()
        .map_err(|e| {
            let _ = fs::remove_file(&staging);
            AppError::BadRequest(format!("新版本二进制无法执行（可能架构不匹配）: {}", e))
        })?;
    if !probe.status.success() {
        let _ = fs::remove_file(&staging);
        return Err(AppError::BadRequest(
            "新版本二进制 --version 自检返回非零状态，已中止替换".to_string(),
        ));
    }
    let new_ver_str = String::from_utf8_lossy(&probe.stdout).trim().to_string();

    let current_exe = std::env::current_exe()
        .map_err(|e| AppError::Internal(format!("定位当前运行路径失败: {}", e)))?;
    let backup_exe = current_exe.with_extension("bak");

    #[cfg(unix)]
    {
        let _ = fs::copy(&current_exe, &backup_exe);
        // Atomic rename on Unix replaces directory entry cleanly while old inode keeps running until restart
        if fs::rename(&staging, &current_exe).is_err() {
            fs::copy(&staging, &current_exe)
                .map_err(|e| AppError::Forbidden(format!("覆盖二进制失败（需写权限）: {}", e)))?;
            let _ = fs::remove_file(&staging);
        }
        Ok(format!(
            "自更新替换完成 ({} / 目标 {})，旧版本已备份至 {}。请重启服务或由 systemd/OpenRC 自动拉起新版。",
            new_ver_str,
            target_version,
            backup_exe.display()
        ))
    }

    #[cfg(not(unix))]
    {
        let _ = backup_exe;
        Ok(format!(
            "新版本已下载并校验通过 ({} / 目标 {})，位于 {}。Windows 无法覆盖运行中的 exe，请停止面板后手动替换。",
            new_ver_str,
            target_version,
            staging.display()
        ))
    }
}

pub fn sha256_file(path: &Path) -> Result<String, AppError> {
    let mut f = File::open(path).map_err(|e| AppError::Internal(e.to_string()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 32768];
    loop {
        let n = f.read(&mut buf).map_err(|e| AppError::Internal(e.to_string()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}
