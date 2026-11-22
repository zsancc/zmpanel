use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zmpanel_core::{ApiResponse, AppError, CronJobItem, CronRunRecord};

use crate::auth::AuthUser;
use crate::cron_jobs;
use crate::error::ServerResult;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct UpsertCronRequest {
    pub id: Option<String>,
    pub name: String,
    pub kind: String,     // shell / backup / http / cleanup / site_restart
    pub schedule: String, // 5-field cron expression
    pub target: String,
    pub enabled: Option<bool>,
    pub keep_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct CronRunsQuery {
    pub job_id: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct CronExecResponse {
    pub success: bool,
    pub output: String,
}

pub async fn list_jobs(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<Vec<CronJobItem>>>> {
    let list = state.db.list_cron_jobs()?;
    Ok(Json(ApiResponse::success(list)))
}

pub async fn upsert_job(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<UpsertCronRequest>,
) -> ServerResult<Json<ApiResponse<CronJobItem>>> {
    let name = payload.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("任务名称不能为空".to_string()).into());
    }
    let kind = match payload.kind.as_str() {
        "shell" | "backup" | "http" | "cleanup" | "site_restart" => payload.kind.clone(),
        _ => {
            return Err(AppError::BadRequest(
                "任务类型仅支持 shell / backup / http / cleanup / site_restart".to_string(),
            )
            .into());
        }
    };
    cron_jobs::validate_cron_expr(&payload.schedule)?;

    let now = Utc::now().timestamp();
    let existing = payload
        .id
        .as_deref()
        .and_then(|id| state.db.get_cron_job(id).ok().flatten());

    let job = CronJobItem {
        id: existing
            .as_ref()
            .map(|e| e.id.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        name,
        kind,
        schedule: payload.schedule.trim().to_string(),
        target: payload.target,
        enabled: payload.enabled.unwrap_or(true),
        keep_count: payload.keep_count.unwrap_or(5).clamp(1, 100),
        last_run_at: existing.as_ref().map(|e| e.last_run_at).unwrap_or(0),
        last_status: existing
            .as_ref()
            .map(|e| e.last_status.clone())
            .unwrap_or_default(),
        created_at: existing.as_ref().map(|e| e.created_at).unwrap_or(now),
        updated_at: now,
    };

    state.db.upsert_cron_job(&job)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "cron.save",
        &job.name,
        &format!("{} ({})", job.schedule, job.kind),
    );
    Ok(Json(ApiResponse::success(job)))
}

pub async fn delete_job(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    state.db.delete_cron_job(&id)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "cron.delete",
        &id,
        "删除计划任务及执行历史",
    );
    Ok(Json(ApiResponse::success(true)))
}

pub async fn run_now(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> ServerResult<Json<ApiResponse<CronExecResponse>>> {
    let job = state
        .db
        .get_cron_job(&id)?
        .ok_or_else(|| AppError::NotFound("计划任务不存在".to_string()))?;
    let job_name = job.name.clone();
    let (success, output) = cron_jobs::execute_cron_job(state.clone(), job).await?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "cron.run_now",
        &job_name,
        if success { "手动执行成功" } else { "手动执行失败" },
    );
    Ok(Json(ApiResponse::success(CronExecResponse {
        success,
        output,
    })))
}

pub async fn list_runs(
    State(state): State<AppState>,
    _auth: AuthUser,
    Query(q): Query<CronRunsQuery>,
) -> ServerResult<Json<ApiResponse<Vec<CronRunRecord>>>> {
    let limit = q.limit.unwrap_or(50).clamp(5, 200);
    let runs = state.db.list_cron_runs(q.job_id.as_deref(), limit)?;
    Ok(Json(ApiResponse::success(runs)))
}
