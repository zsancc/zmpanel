use std::process::Command;
use zmpanel_core::{AppError, NotifyConfig};

use crate::state::AppState;

pub fn load_notify_config(state: &AppState) -> NotifyConfig {
    state
        .db
        .get_setting("notify_config")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn send_notification(cfg: &NotifyConfig, title: &str, body: &str) -> Result<String, AppError> {
    if !cfg.enabled {
        return Err(AppError::BadRequest("告警通知开关未启用".to_string()));
    }
    let mut sent_channels = Vec::new();

    if !cfg.webhook_url.trim().is_empty() {
        let url = cfg.webhook_url.trim();
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(AppError::BadRequest("Webhook URL 必须以 http:// 或 https:// 开头".to_string()));
        }
        let payload = serde_json::json!({
            "source": "zmpanel",
            "title": title,
            "message": body,
        })
        .to_string();
        let out = Command::new("curl")
            .args([
                "-fsSL",
                "--max-time",
                "12",
                "-X",
                "POST",
                "-H",
                "Content-Type: application/json",
                "-d",
                &payload,
                url,
            ])
            .output()
            .map_err(|e| AppError::Internal(format!("发送 Webhook 失败: {}", e)))?;
        if !out.status.success() {
            return Err(AppError::BadRequest(format!(
                "Webhook 推送失败: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        sent_channels.push("Webhook");
    }

    if !cfg.telegram_bot_token.trim().is_empty() && !cfg.telegram_chat_id.trim().is_empty() {
        let api_url = format!(
            "https://api.telegram.org/bot{}/sendMessage",
            cfg.telegram_bot_token.trim()
        );
        let text = format!("[zmpanel] {}\n{}", title, body);
        let payload = serde_json::json!({
            "chat_id": cfg.telegram_chat_id.trim(),
            "text": text,
        })
        .to_string();
        let out = Command::new("curl")
            .args([
                "-fsSL",
                "--max-time",
                "12",
                "-X",
                "POST",
                "-H",
                "Content-Type: application/json",
                "-d",
                &payload,
                &api_url,
            ])
            .output()
            .map_err(|e| AppError::Internal(format!("发送 Telegram 告警失败: {}", e)))?;
        if !out.status.success() {
            return Err(AppError::BadRequest(format!(
                "Telegram 推送失败: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        sent_channels.push("Telegram");
    }

    if sent_channels.is_empty() {
        return Err(AppError::BadRequest(
            "请至少配置 Webhook URL 或 Telegram Bot Token + Chat ID".to_string(),
        ));
    }

    Ok(format!("已推送至: {}", sent_channels.join(", ")))
}
