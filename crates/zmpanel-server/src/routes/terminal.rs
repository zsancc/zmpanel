use axum::{
    Json,
    extract::{Path, Query, State, WebSocketUpgrade},
    http::HeaderMap,
    response::Response,
};
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;
use zmpanel_core::{ApiResponse, AppError, CommandSnippet};

use crate::auth::{AuthUser, authenticate_token, extract_client_ip, extract_token_from_headers};
use crate::error::ServerResult;
use crate::state::AppState;
use crate::terminal;

#[derive(Debug, Deserialize)]
pub struct TerminalWsQuery {
    pub cwd: Option<String>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpsertSnippetRequest {
    pub id: Option<String>,
    pub name: String,
    pub category: String,
    pub command: String,
    pub description: Option<String>,
}

pub async fn ws_terminal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<TerminalWsQuery>,
    ws: WebSocketUpgrade,
) -> ServerResult<Response> {
    let ip = extract_client_ip(&headers);
    let raw_token = extract_token_from_headers(&headers)
        .or(query.token)
        .ok_or(AppError::Unauthorized)?;
    let auth = authenticate_token(&state, &raw_token, ip)?;

    let cwd = query.cwd.clone();
    let cols = query.cols.unwrap_or(120);
    let rows = query.rows.unwrap_or(32);

    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "terminal.open",
        cwd.as_deref().unwrap_or("default"),
        "打开 Web 终端会话",
    );

    Ok(ws.on_upgrade(move |socket| terminal::handle_terminal_socket(socket, cwd, cols, rows)))
}

pub async fn list_snippets(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> ServerResult<Json<ApiResponse<Vec<CommandSnippet>>>> {
    let items = state.db.list_snippets()?;
    Ok(Json(ApiResponse::success(items)))
}

pub async fn upsert_snippet(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<UpsertSnippetRequest>,
) -> ServerResult<Json<ApiResponse<CommandSnippet>>> {
    if payload.name.trim().is_empty() || payload.command.trim().is_empty() {
        return Err(AppError::BadRequest("名称和命令内容不能为空".to_string()).into());
    }
    let item = CommandSnippet {
        id: payload
            .id
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        name: payload.name.trim().to_string(),
        category: if payload.category.trim().is_empty() {
            "常用命令".to_string()
        } else {
            payload.category.trim().to_string()
        },
        command: payload.command,
        description: payload.description.unwrap_or_default(),
        updated_at: Utc::now().timestamp(),
    };
    state.db.upsert_snippet(&item)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "terminal.snippet_save",
        &item.name,
        "保存命令收藏片段",
    );
    Ok(Json(ApiResponse::success(item)))
}

pub async fn delete_snippet(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> ServerResult<Json<ApiResponse<bool>>> {
    state.db.delete_snippet(&id)?;
    let _ = state.db.write_audit(
        &auth.user.username,
        &auth.client_ip,
        "terminal.snippet_delete",
        &id,
        "删除命令收藏片段",
    );
    Ok(Json(ApiResponse::success(true)))
}
