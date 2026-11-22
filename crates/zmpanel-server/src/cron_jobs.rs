use chrono::{Datelike, Local, Timelike, Utc};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use zmpanel_core::{AppError, CronJobItem};

use crate::backup::{self, CreateBackupRequest};
use crate::files;
use crate::state::AppState;

/// Pure Rust 5-field standard cron expression validator & matcher:
/// `minute(0-59) hour(0-23) day_of_month(1-31) month(1-12) day_of_week(0-6, 0=Sun)`
pub fn validate_cron_expr(expr: &str) -> Result<(), AppError> {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() != 5 {
        return Err(AppError::BadRequest(
            "Cron 表达式必须包含 5 段：分 时 日 月 周（例如 0 3 * * * 表示每天凌晨 3 点）"
                .to_string(),
        ));
    }
    let bounds = [(0u32, 59u32), (0, 23), (1, 31), (1, 12), (0, 6)];
    for (part, (min_v, max_v)) in parts.iter().zip(bounds.iter()) {
        validate_field(part, *min_v, *max_v)?;
    }
    Ok(())
}

fn validate_field(field: &str, min_v: u32, max_v: u32) -> Result<(), AppError> {
    for segment in field.split(',') {
        let seg = segment.trim();
        if seg.is_empty() {
            return Err(AppError::BadRequest(format!("无效 Cron 字段: {}", field)));
        }
        let (base, step_opt) = match seg.split_once('/') {
            Some((b, s)) => {
                let step: u32 = s
                    .parse()
                    .map_err(|_| AppError::BadRequest(format!("无效步长: {}", seg)))?;
                if step == 0 {
                    return Err(AppError::BadRequest("Cron 步长不能为 0".to_string()));
                }
                (b, Some(step))
            }
            None => (seg, None),
        };
        if base == "*" {
            let _ = step_opt;
            continue;
        }
        if let Some((a, b)) = base.split_once('-') {
            let start: u32 = a
                .parse()
                .map_err(|_| AppError::BadRequest(format!("无效范围起点: {}", seg)))?;
            let end: u32 = b
                .parse()
                .map_err(|_| AppError::BadRequest(format!("无效范围终点: {}", seg)))?;
            if start < min_v || end > max_v || start > end {
                return Err(AppError::BadRequest(format!("Cron 范围越界: {}", seg)));
            }
        } else {
            let val: u32 = base
                .parse()
                .map_err(|_| AppError::BadRequest(format!("无效数值: {}", seg)))?;
            if val < min_v || val > max_v {
                return Err(AppError::BadRequest(format!("Cron 数值越界: {}", seg)));
            }
        }
    }
    Ok(())
}

pub fn cron_matches_now(expr: &str, minute: u32, hour: u32, dom: u32, month: u32, dow: u32) -> bool {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() != 5 {
        return false;
    }
    field_matches(parts[0], minute, 0)
        && field_matches(parts[1], hour, 0)
        && field_matches(parts[2], dom, 1)
        && field_matches(parts[3], month, 1)
        && field_matches(parts[4], dow, 0)
}

fn field_matches(field: &str, value: u32, min_v: u32) -> bool {
    for segment in field.split(',') {
        let seg = segment.trim();
        let (base, step) = match seg.split_once('/') {
            Some((b, s)) => (b, s.parse::<u32>().unwrap_or(1).max(1)),
            None => (seg, 1),
        };
        if base == "*" {
            if value >= min_v && (value - min_v) % step == 0 {
                return true;
            }
        } else if let Some((a, b)) = base.split_once('-') {
            if let (Ok(start), Ok(end)) = (a.parse::<u32>(), b.parse::<u32>())
                && value >= start
                && value <= end
                && (value - start) % step == 0
            {
                return true;
            }
        } else if let Ok(v) = base.parse::<u32>()
            && value == v
        {
            return true;
        }
    }
    false
}

pub fn spawn_cron_scheduler(state: AppState) {
    let last_fired: Mutex<HashMap<String, i64>> = Mutex::new(HashMap::new());
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(20)).await;
            let now_local = Local::now();
            let minute_bucket = now_local.timestamp() / 60;
            let jobs = state.db.list_cron_jobs().unwrap_or_default();

            for job in jobs.into_iter().filter(|j| j.enabled) {
                let dow = now_local.weekday().num_days_from_sunday();
                if !cron_matches_now(
                    &job.schedule,
                    now_local.minute(),
                    now_local.hour(),
                    now_local.day(),
                    now_local.month(),
                    dow,
                ) {
                    continue;
                }
                {
                    let mut guard = last_fired.lock().expect("cron lock poisoned");
                    if guard.get(&job.id).copied() == Some(minute_bucket) {
                        continue;
                    }
                    guard.insert(job.id.clone(), minute_bucket);
                }
                let st = state.clone();
                tokio::spawn(async move {
                    let _ = execute_cron_job(st, job).await;
                });
            }
        }
    });
}

pub async fn execute_cron_job(state: AppState, job: CronJobItem) -> Result<(bool, String), AppError> {
    let started_at = Utc::now().timestamp();
    let t0 = Instant::now();

    let job_clone = job.clone();
    let st_clone = state.clone();
    let res = tokio::task::spawn_blocking(move || run_job_sync(&st_clone, &job_clone))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let duration_ms = t0.elapsed().as_millis() as i64;
    let (success, output) = match res {
        Ok(out) => (true, out),
        Err(err) => (false, err.to_string()),
    };
    let status_label = if success { "成功" } else { "失败" };
    let _ = state
        .db
        .update_cron_job_run_state(&job.id, started_at, status_label);
    let _ = state.db.insert_cron_run(
        &job.id,
        &job.name,
        started_at,
        duration_ms,
        success,
        &output,
    );
    Ok((success, output))
}

fn run_job_sync(state: &AppState, job: &CronJobItem) -> Result<String, AppError> {
    match job.kind.as_str() {
        "shell" => run_shell_job(&job.target),
        "http" => run_http_job(&job.target),
        "backup" => run_backup_job(state, job),
        "cleanup" => run_cleanup_job(state),
        "site_restart" => run_site_restart_job(state, &job.target),
        other => Err(AppError::BadRequest(format!("未知计划任务类型: {}", other))),
    }
}

fn run_shell_job(script: &str) -> Result<String, AppError> {
    if script.trim().is_empty() {
        return Err(AppError::BadRequest("脚本内容不能为空".to_string()));
    }
    #[cfg(windows)]
    let out = Command::new("pwsh")
        .args(["-NoLogo", "-NoProfile", "-Command", script])
        .output()
        .or_else(|_| {
            Command::new("powershell")
                .args(["-NoLogo", "-NoProfile", "-Command", script])
                .output()
        })
        .map_err(|e| AppError::Internal(format!("执行脚本失败: {}", e)))?;

    #[cfg(not(windows))]
    let out = Command::new("/bin/sh")
        .args(["-c", script])
        .output()
        .map_err(|e| AppError::Internal(format!("执行脚本失败: {}", e)))?;

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!(
        "{}{}",
        stdout,
        if stderr.is_empty() {
            String::new()
        } else {
            format!("\n{}", stderr)
        }
    )
    .trim()
    .to_string();

    if out.status.success() {
        Ok(if combined.is_empty() {
            "脚本执行完成（无输出）".to_string()
        } else {
            combined
        })
    } else {
        Err(AppError::BadRequest(format!(
            "脚本退出码 {}:\n{}",
            out.status.code().unwrap_or(-1),
            combined
        )))
    }
}

fn run_http_job(url: &str) -> Result<String, AppError> {
    let trimmed = url.trim();
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err(AppError::BadRequest(
            "URL 必须以 http:// 或 https:// 开头".to_string(),
        ));
    }
    let out = Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "20",
            "-w",
            "\nHTTP %{http_code} (%{time_total}s)",
            trimmed,
        ])
        .output()
        .map_err(|e| AppError::Internal(format!("执行 HTTP 探测失败 (需 curl): {}", e)))?;
    let body = String::from_utf8_lossy(&out.stdout).to_string();
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    if out.status.success() {
        Ok(body.chars().take(2000).collect())
    } else {
        Err(AppError::BadRequest(format!("HTTP 探测失败: {}", err.trim())))
    }
}

fn run_backup_job(state: &AppState, job: &CronJobItem) -> Result<String, AppError> {
    let cfg = state.config_snapshot();
    // Parse custom CreateBackupRequest JSON from job.target, or fallback to backing up all websites + docker + ssl + panel db
    let req: CreateBackupRequest = serde_json::from_str(&job.target).unwrap_or_else(|_| {
        let website_ids = state
            .db
            .list_websites()
            .unwrap_or_default()
            .into_iter()
            .map(|w| w.id)
            .collect();
        let docker_project_ids = state
            .db
            .list_docker_projects()
            .unwrap_or_default()
            .into_iter()
            .map(|d| d.id)
            .collect();
        let ssl_cert_ids = state
            .db
            .list_ssl_certs()
            .unwrap_or_default()
            .into_iter()
            .map(|c| c.id)
            .collect();
        CreateBackupRequest {
            name: Some(format!("cron-{}", job.name)),
            website_ids,
            docker_project_ids,
            ssl_cert_ids,
            custom_dirs: vec![],
            include_panel_db: true,
            target_repo: Some("local".to_string()),
        }
    });

    let created = backup::create_backup_archive(&cfg.data_dir, &state.db, &req)?;

    // Enforce retention policy (keep_count) for backups created by this cron job prefix
    let prefix = format!("cron-{}", job.name);
    let keep = job.keep_count.max(1) as usize;
    let matching: Vec<_> = state
        .db
        .list_backups()
        .unwrap_or_default()
        .into_iter()
        .filter(|b| b.name.starts_with(&prefix))
        .collect();
    let mut pruned = 0usize;
    if matching.len() > keep {
        for old in &matching[keep..] {
            let p = PathBuf::from(&old.file_path);
            if p.is_file() {
                let _ = std::fs::remove_file(p);
            }
            let _ = state.db.delete_backup(&old.id);
            pruned += 1;
        }
    }

    Ok(format!(
        "定时备份成功：{} ({} bytes, SHA256 {}...)，轮转清理旧备份 {} 份",
        created.name,
        created.size_bytes,
        &created.sha256[..12],
        pruned
    ))
}

fn run_cleanup_job(state: &AppState) -> Result<String, AppError> {
    let cfg = state.config_snapshot();
    let _ = files::empty_trash(&cfg.trash_dir(), &state.db);
    // Truncate site logs larger than 10MB
    let mut truncated = 0usize;
    let site_logs = cfg.logs_dir().join("sites");
    if let Ok(rd) = std::fs::read_dir(&site_logs) {
        for entry in rd.flatten() {
            if let Ok(meta) = entry.metadata()
                && meta.len() > 10 * 1024 * 1024
                && std::fs::OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(entry.path())
                    .is_ok()
            {
                truncated += 1;
            }
        }
    }
    Ok(format!(
        "定时清理完成：已清空回收站，截断超过 10MB 的站点日志 {} 个",
        truncated
    ))
}

fn run_site_restart_job(state: &AppState, site_id: &str) -> Result<String, AppError> {
    let site = state
        .db
        .get_website(site_id.trim())
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("未找到网站 ID: {}", site_id)))?;
    let logs_dir = state.config_snapshot().logs_dir();
    let pid = state.supervisor.start_site(&site, &logs_dir)?;
    Ok(format!("定时重启网站「{}」成功 (PID {})", site.name, pid))
}
