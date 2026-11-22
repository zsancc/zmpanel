use axum::{
    extract::{Path, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use rust_embed::Embed;

use crate::state::AppState;

#[derive(Embed)]
#[folder = "../../web/dist"]
struct WebAssets;

pub async fn serve_index(State(state): State<AppState>) -> Response {
    render_spa_index(&state.current_entry())
}

pub async fn serve_static(
    State(state): State<AppState>,
    Path(path): Path<String>,
) -> Response {
    let trimmed = path.trim_start_matches('/');
    if trimmed.is_empty() || trimmed == "index.html" {
        return render_spa_index(&state.current_entry());
    }

    if let Some(content) = WebAssets::get(trimmed) {
        let mime = mime_guess::from_path(trimmed).first_or_octet_stream();
        return (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, mime.as_ref().to_string()),
                (
                    header::CACHE_CONTROL,
                    if trimmed.starts_with("assets/") {
                        "public, max-age=31536000, immutable".to_string()
                    } else {
                        "no-cache".to_string()
                    },
                ),
            ],
            content.data.into_owned(),
        )
            .into_response();
    }

    // SPA client-side route fallback
    render_spa_index(&state.current_entry())
}

fn render_spa_index(entry: &str) -> Response {
    match WebAssets::get("index.html") {
        Some(file) => {
            let html = String::from_utf8_lossy(&file.data);
            let base_href = if entry.is_empty() {
                "/".to_string()
            } else {
                format!("{}/", entry)
            };
            let inject = format!(
                "<script>window.__ZMPANEL_BASE__={};</script>",
                serde_json::to_string(entry).unwrap_or_else(|_| "\"\"".to_string())
            );
            let rewritten = html
                .replace("<head>", &format!("<head><base href=\"{}\">{}", base_href, inject))
                .replace("src=\"/assets/", "src=\"assets/")
                .replace("href=\"/assets/", "href=\"assets/");
            Html(rewritten).into_response()
        }
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "前端资源尚未构建，请先在 web 目录运行 pnpm build。",
        )
            .into_response(),
    }
}
