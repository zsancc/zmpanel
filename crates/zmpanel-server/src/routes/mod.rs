pub mod auth;
pub mod backup;
pub mod cluster;
pub mod cron;
pub mod docker;
pub mod files;
pub mod firewall;
pub mod logs;
pub mod security;
pub mod ssl;
pub mod store;
pub mod system;
pub mod terminal;
pub mod websites;

use axum::{
    Router,
    extract::{DefaultBodyLimit, Request, State},
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use tower::ServiceExt;
use tower_http::trace::TraceLayer;

use crate::assets;
use crate::state::AppState;

pub fn build_app(state: AppState) -> Router {
    let inner = build_inner_router(state.clone());
    Router::new()
        .fallback(entry_dispatcher)
        .with_state((state, inner))
        .layer(TraceLayer::new_for_http())
}

fn build_inner_router(state: AppState) -> Router {
    let api = Router::new()
        // Public auth endpoints (still behind security entry)
        .route("/auth/login", post(auth::login))
        .route("/auth/status", get(auth::auth_status))
        // Authenticated auth/profile endpoints
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        .route("/auth/password", put(auth::change_password))
        .route("/auth/totp/setup", post(auth::totp_setup))
        .route("/auth/totp/enable", post(auth::totp_enable))
        .route("/auth/totp/disable", post(auth::totp_disable))
        .route("/settings/security", put(auth::update_security_settings))
        // System & monitoring
        .route("/system/snapshot", get(system::get_snapshot))
        .route("/system/metrics", get(system::get_metrics))
        .route("/system/processes", get(system::get_processes))
        .route("/system/processes/{pid}", delete(system::kill_process))
        .route("/system/swap", post(system::configure_swap))
        .route("/system/power", post(system::power_action))
        .route("/system/audit", get(system::get_audit_logs))
        .route("/nodes", get(system::list_nodes))
        // Files
        .route("/files/list", get(files::list_dir))
        .route("/files/content", get(files::read_file).put(files::write_file))
        .route("/files/create", post(files::create_entry))
        .route("/files/rename", post(files::rename_entry))
        .route("/files/copy", post(files::copy_entry))
        .route("/files/delete", post(files::delete_entry))
        .route("/files/chmod", post(files::chmod_entry))
        .route("/files/compress", post(files::compress_entries))
        .route("/files/extract", post(files::extract_archive))
        .route(
            "/files/upload",
            post(files::upload_files).layer(DefaultBodyLimit::max(512 * 1024 * 1024)),
        )
        .route("/files/download", get(files::download_file))
        .route("/files/trash", get(files::list_trash).delete(files::empty_trash))
        .route("/files/trash/restore", post(files::restore_trash))
        .route("/files/trash/{id}", delete(files::delete_trash_item))
        // Terminal & Snippets
        .route("/terminal/ws", get(terminal::ws_terminal))
        .route(
            "/terminal/snippets",
            get(terminal::list_snippets).post(terminal::upsert_snippet),
        )
        .route("/terminal/snippets/{id}", delete(terminal::delete_snippet))
        // Stage 2: Websites (Rust / Go / Python / Proxy / Static)
        .route(
            "/websites",
            get(websites::list_websites).post(websites::upsert_website),
        )
        .route("/websites/{id}", delete(websites::delete_website))
        .route("/websites/{id}/action", post(websites::site_action))
        .route("/websites/{id}/logs", get(websites::site_logs))
        .route(
            "/websites/{id}/nginx",
            get(websites::get_site_nginx).put(websites::save_site_nginx),
        )
        // Stage 2: Docker Workspace & Containers
        .route("/docker/status", get(docker::status))
        .route("/docker/install", post(docker::install_engine))
        .route("/docker/containers", get(docker::list_containers))
        .route(
            "/docker/containers/{id}/action",
            post(docker::container_action),
        )
        .route("/docker/containers/{id}/logs", get(docker::container_logs))
        .route("/docker/images", get(docker::list_images))
        .route("/docker/images/{id}", delete(docker::remove_image))
        .route("/docker/prune", post(docker::prune))
        .route(
            "/docker/projects",
            get(docker::list_projects).post(docker::upsert_project),
        )
        .route("/docker/projects/{id}", delete(docker::delete_project))
        .route(
            "/docker/projects/{id}/action",
            post(docker::project_compose_action),
        )
        // Stage 2: SSL Certificate Center
        .route("/ssl/certs", get(ssl::list_certs))
        .route("/ssl/certs/import", post(ssl::import_cert))
        .route("/ssl/certs/self-signed", post(ssl::generate_self_signed))
        .route("/ssl/certs/acme", post(ssl::create_acme_cert))
        .route("/ssl/certs/{id}/export", get(ssl::export_cert))
        .route("/ssl/certs/{id}", delete(ssl::delete_cert))
        // Stage 2: Software Store (Nginx Security + Python uv)
        .route("/store/apps", get(store::list_apps))
        .route("/store/apps/{id}/install", post(store::install_app))
        .route(
            "/store/nginx/security",
            get(store::get_nginx_security).put(store::save_nginx_security),
        )
        .route("/store/nginx/control", post(store::nginx_control))
        .route("/store/uv/overview", get(store::uv_overview))
        .route("/store/uv/python/install", post(store::uv_install_python))
        .route("/store/uv/venvs", post(store::uv_create_venv))
        .route("/store/uv/venvs/{id}", delete(store::uv_delete_venv))
        .route("/store/uv/pip/install", post(store::uv_pip_install))
        .route(
            "/store/services/redis",
            get(store::get_redis_service).put(store::save_redis_service),
        )
        .route(
            "/store/services/postgresql",
            get(store::get_postgres_service).put(store::save_postgres_service),
        )
        .route(
            "/store/services/mongodb",
            get(store::get_mongo_service).put(store::save_mongo_service),
        )
        .route(
            "/store/services/to-docker",
            post(store::create_docker_workspace_for_service),
        )
        // Stage 2: Backup & Migration Framework
        .route(
            "/backups",
            get(backup::list_backups).post(backup::create_backup),
        )
        .route("/backups/{id}", delete(backup::delete_backup))
        .route("/backups/{id}/restore", post(backup::restore_backup))
        .route("/backups/{id}/download", get(backup::download_backup))
        // Stage 3: Firewall (Anti-Lockout + nftables/iptables/ufw)
        .route("/firewall/overview", get(firewall::overview))
        .route("/firewall/rules", post(firewall::upsert_rule))
        .route("/firewall/rules/{id}", delete(firewall::delete_rule))
        .route("/firewall/sync", post(firewall::sync_now))
        // Stage 3: Security Center (Built-in Lightweight IPS + SSH Hardening)
        .route("/security/overview", get(security::overview))
        .route("/security/config", put(security::save_config))
        .route("/security/bans", post(security::manual_ban))
        .route("/security/bans/{ip}", delete(security::unban_ip))
        .route("/security/whitelist", post(security::add_whitelist))
        .route("/security/whitelist/{ip}", delete(security::remove_whitelist))
        .route("/security/scan", post(security::scan_now))
        .route("/security/analyze", post(security::analyze_sample))
        .route("/security/ssh", put(security::update_ssh_hardening))
        .route("/security/vuln/scan", post(security::scan_vulnerabilities))
        .route("/security/vuln/fix", post(security::fix_vulnerabilities))
        .route(
            "/security/vuln/config",
            get(security::get_vuln_watch_config).put(security::save_vuln_watch_config),
        )
        // Stage 3: Log Center
        .route("/logs/sources", get(logs::list_sources))
        .route("/logs/tail", get(logs::tail_log))
        .route("/logs/truncate", post(logs::truncate_log))
        // Stage 3: Cron Jobs
        .route(
            "/cron/jobs",
            get(cron::list_jobs).post(cron::upsert_job),
        )
        .route("/cron/jobs/{id}", delete(cron::delete_job))
        .route("/cron/jobs/{id}/run", post(cron::run_now))
        .route("/cron/runs", get(cron::list_runs))
        // Stage 4: Master/Agent Cluster, NAT Reverse WS, Batch Ops, Central Backups & Self-Update
        .route("/agent/ws", get(cluster::agent_ws))
        .route(
            "/cluster/nodes",
            get(cluster::list_nodes).post(cluster::create_node),
        )
        .route("/cluster/nodes/{id}", delete(cluster::delete_node))
        .route(
            "/cluster/nodes/{id}/pull-backup",
            post(cluster::pull_node_backup),
        )
        .route("/cluster/batch/exec", post(cluster::batch_exec))
        .route(
            "/cluster/batch/update-software",
            post(cluster::batch_update_software),
        )
        .route(
            "/cluster/batch/self-update",
            post(cluster::batch_self_update),
        )
        .route("/cluster/jobs", get(cluster::list_jobs))
        .route("/cluster/backups", get(cluster::list_master_backups))
        .route(
            "/cluster/backups/{id}/push-restore",
            post(cluster::push_restore_backup),
        )
        .route(
            "/cluster/backups/{id}/download",
            get(cluster::download_master_backup),
        )
        .route("/cluster/backups/{id}", delete(cluster::delete_master_backup))
        .route("/update/check", post(cluster::check_update))
        .route("/update/apply", post(cluster::apply_self_update))
        .route(
            "/notify/config",
            get(cluster::get_notify_config).put(cluster::save_notify_config),
        )
        .route("/notify/test", post(cluster::test_notify));

    Router::new()
        .nest("/api", api)
        .route("/", get(assets::serve_index))
        .route("/{*path}", get(assets::serve_static))
        .with_state(state)
}

async fn entry_dispatcher(
    State((state, inner)): State<(AppState, Router)>,
    mut req: Request,
) -> Response {
    let entry = state.current_entry();
    if !entry.is_empty() {
        let orig_path = req.uri().path().to_string();
        let entry_slash = format!("{}/", entry);

        let stripped = if orig_path == entry {
            "/".to_string()
        } else if let Some(rest) = orig_path.strip_prefix(&entry_slash) {
            format!("/{}", rest)
        } else {
            return (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                "404 Not Found",
            )
                .into_response();
        };

        let new_pq = match req.uri().query() {
            Some(q) => format!("{}?{}", stripped, q),
            None => stripped,
        };
        if let Ok(new_uri) = new_pq.parse::<Uri>() {
            *req.uri_mut() = new_uri;
        }
    }

    match inner.oneshot(req).await {
        Ok(resp) => resp,
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response(),
    }
}
