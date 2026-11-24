use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;
use zmpanel_core::{PanelConfig, RunMode};
use zmpanel_server::{auth, db::Database, routes, state::AppState};

fn make_test_state() -> (AppState, tempfile::TempDir) {
    let dir = tempdir().expect("create tempdir");
    let db = Database::open(&dir.path().join("zmpanel.db")).expect("open db");
    let hash = auth::hash_password("TestPass123!").expect("hash");
    db.create_user("admin", &hash).expect("create admin");
    let cfg = PanelConfig {
        mode: RunMode::Standalone,
        bind_host: "127.0.0.1".to_string(),
        port: 18080,
        data_dir: dir.path().to_path_buf(),
        security_entry: "/zm_secret".to_string(),
        master_url: None,
        node_token: None,
        session_ttl_hours: 24,
        max_login_attempts: 5,
        lock_minutes: 10,
    };
    (AppState::new(cfg, db), dir)
}

async fn read_json(resp: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .expect("read body");
    serde_json::from_slice(&bytes).expect("parse json")
}

#[tokio::test]
async fn test_security_entry_stealth_404_and_dynamic_update() {
    let (state, _dir) = make_test_state();
    let app = routes::build_app(state);

    // 1. Root & wrong entry must return stealth 404
    let r1 = app
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::NOT_FOUND);

    let r2 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/auth/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::NOT_FOUND);

    // 2. Correct entry works
    let r3 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/zm_secret/api/auth/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r3.status(), StatusCode::OK);

    // 3. Login with valid password
    let login_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"username":"admin","password":"TestPass123!"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login_resp.status(), StatusCode::OK);
    let login_json = read_json(login_resp).await;
    let token = login_json["data"]["token"].as_str().unwrap().to_string();

    // 4. Update security entry dynamically
    let upd_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/zm_secret/api/settings/security")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .body(Body::from(
                    json!({"security_entry":"new_gate_99"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(upd_resp.status(), StatusCode::OK);

    // 5. Old entry immediately returns 404; new entry returns 200
    let old_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/zm_secret/api/auth/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(old_resp.status(), StatusCode::NOT_FOUND);

    let new_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/new_gate_99/api/auth/me")
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(new_resp.status(), StatusCode::OK);
    let me_json = read_json(new_resp).await;
    assert_eq!(me_json["data"]["security_entry"], "/new_gate_99");
}

#[tokio::test]
async fn test_files_crud_archive_and_trash() {
    let (state, dir) = make_test_state();
    let app = routes::build_app(state);

    let login_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"username":"admin","password":"TestPass123!"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let token = read_json(login_resp).await["data"]["token"]
        .as_str()
        .unwrap()
        .to_string();

    let work = dir.path().join("workspace");
    let work_str = work.to_string_lossy().replace('\\', "/");
    let sample_file = format!("{}/hello.txt", work_str);
    let archive_zip = format!("{}/bundle.zip", work_str);
    let extract_dir = format!("{}/extracted", work_str);

    // Create directory
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/files/create")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .body(Body::from(
                    json!({"path": work_str, "is_dir": true}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);

    // Write file content
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/zm_secret/api/files/content")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .body(Body::from(
                    json!({"path": sample_file, "content": "line1\nline2\nline3"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);

    // Compress to zip
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/files/compress")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .body(Body::from(
                    json!({"paths": [sample_file], "dest": archive_zip}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);

    // Extract zip
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/files/extract")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .body(Body::from(
                    json!({"archive": archive_zip, "dest_dir": extract_dir}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let extracted_txt = std::fs::read_to_string(work.join("extracted/hello.txt")).unwrap();
    assert_eq!(extracted_txt, "line1\nline2\nline3");

    // Move to trash & restore
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/files/delete")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .body(Body::from(
                    json!({"paths": [sample_file], "permanent": false}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(!work.join("hello.txt").exists());

    let trash_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/zm_secret/api/files/trash")
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let trash_json = read_json(trash_resp).await;
    let trash_id = trash_json["data"][0]["id"].as_str().unwrap().to_string();

    let restore_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/files/trash/restore")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .body(Body::from(json!({"id": trash_id}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(restore_resp.status(), StatusCode::OK);
    assert!(work.join("hello.txt").exists());
}

#[tokio::test]
async fn test_embedded_spa_snippets_and_audit() {
    let (state, _dir) = make_test_state();
    let app = routes::build_app(state);

    // 1. Embedded SPA HTML serves at security entry and injects base path
    let html_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/zm_secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(html_resp.status(), StatusCode::OK);
    let html_bytes = axum::body::to_bytes(html_resp.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&html_bytes);
    assert!(html.contains("window.__ZMPANEL_BASE__=\"/zm_secret\""));
    assert!(html.contains("<div id=\"app\"></div>"));

    // 2. Login and verify snippets CRUD + audit trail
    let login_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"username":"admin","password":"TestPass123!"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let token = read_json(login_resp).await["data"]["token"]
        .as_str()
        .unwrap()
        .to_string();

    let snips_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/zm_secret/api/terminal/snippets")
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(snips_resp.status(), StatusCode::OK);
    let snips_json = read_json(snips_resp).await;
    assert!(snips_json["data"].as_array().unwrap().len() >= 5);

    let save_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/terminal/snippets")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .body(Body::from(
                    json!({
                        "name": "Nginx 漏洞更新",
                        "category": "安全",
                        "command": "apt update && apt upgrade -y nginx"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(save_resp.status(), StatusCode::OK);

    let audit_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/zm_secret/api/system/audit?limit=20")
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(audit_resp.status(), StatusCode::OK);
    let audit_json = read_json(audit_resp).await;
    assert!(audit_json["data"].as_array().unwrap().len() >= 2);
}

#[tokio::test]
async fn test_stage2_ssl_websites_docker_store_and_backup_roundtrip() {
    let (state, dir) = make_test_state();
    let app = routes::build_app(state);

    // Login
    let login_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"username":"admin","password":"TestPass123!"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let token = read_json(login_resp).await["data"]["token"]
        .as_str()
        .unwrap()
        .to_string();
    let auth_hdr = format!("Bearer {}", token);

    // 1. SSL: Generate self-signed cert & export & re-import with X.509 parser
    let cert_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/ssl/certs/self-signed")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({
                        "name": "test-wildcard",
                        "domains": ["example.test", "*.example.test"],
                        "valid_days": 365
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cert_resp.status(), StatusCode::OK);
    let cert_json = read_json(cert_resp).await;
    let cert_id = cert_json["data"]["id"].as_str().unwrap().to_string();

    let export_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/zm_secret/api/ssl/certs/{}/export", cert_id))
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(export_resp.status(), StatusCode::OK);
    let exported = read_json(export_resp).await;
    let cert_pem = exported["data"]["cert_pem"].as_str().unwrap().to_string();
    let key_pem = exported["data"]["key_pem"].as_str().unwrap().to_string();
    assert!(cert_pem.contains("BEGIN CERTIFICATE"));

    let import_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/ssl/certs/import")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({
                        "name": "reimported-cert",
                        "cert_pem": cert_pem,
                        "key_pem": key_pem
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(import_resp.status(), StatusCode::OK);
    let imported = read_json(import_resp).await;
    assert_eq!(imported["data"]["domains"][0], "example.test");

    // 2. Websites: Create Rust website bound to SSL cert & check generated Nginx config
    let site_root = dir.path().join("rust-site");
    let site_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/websites")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({
                        "name": "my-rust-api",
                        "kind": "rust",
                        "domains": ["api.example.test"],
                        "listen_port": 9090,
                        "root_dir": site_root.to_string_lossy().replace('\\', "/"),
                        "start_cmd": "./my-api",
                        "ssl_cert_id": cert_id,
                        "force_https": true
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(site_resp.status(), StatusCode::OK);
    let site_json = read_json(site_resp).await;
    let site_id = site_json["data"]["id"].as_str().unwrap().to_string();
    std::fs::write(site_root.join("README.md"), "rust site payload").unwrap();

    let nginx_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/zm_secret/api/websites/{}/nginx", site_id))
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(nginx_resp.status(), StatusCode::OK);
    let nginx_txt = read_json(nginx_resp).await["data"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(nginx_txt.contains("proxy_pass http://127.0.0.1:9090;"));
    assert!(nginx_txt.contains("listen 443 ssl http2;"));
    assert!(nginx_txt.contains("return 301 https://"));

    // 3. Docker Workspace: Verify host virtualization/nesting env detection + create project + block non-docker command injection
    let docker_st_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/zm_secret/api/docker/status")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(docker_st_resp.status(), StatusCode::OK);
    let docker_st_json = read_json(docker_st_resp).await;
    assert!(
        !docker_st_json["data"]["env"]["virt_type"]
            .as_str()
            .unwrap_or("")
            .is_empty()
    );
    assert!(
        !docker_st_json["data"]["env"]["docker_mode"]
            .as_str()
            .unwrap_or("")
            .is_empty()
    );
    assert!(
        !docker_st_json["data"]["env"]["docker_nesting_reason"]
            .as_str()
            .unwrap_or("")
            .is_empty()
    );

    let docker_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/docker/projects")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({
                        "name": "memos-demo",
                        "deploy_cmd": "rm -rf /",
                        "compose_content": "services:\n  memos:\n    image: neosmemo/memos:stable\n"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(docker_resp.status(), StatusCode::OK);
    let docker_json = read_json(docker_resp).await;
    let docker_id = docker_json["data"]["id"].as_str().unwrap().to_string();

    // Verify unsafe non-docker command is blocked with 403 Forbidden
    let bad_cmd_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/zm_secret/api/docker/projects/{}/action", docker_id))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(json!({"action": "deploy_cmd"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad_cmd_resp.status(), StatusCode::FORBIDDEN);

    // 4. Software Store: Catalog + Nginx Security Config
    let apps_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/zm_secret/api/store/apps")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(apps_resp.status(), StatusCode::OK);
    let apps_json = read_json(apps_resp).await;
    assert!(apps_json["data"].as_array().unwrap().len() >= 6);
    assert!(
        apps_json["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["id"] == "docker")
    );

    let sec_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/zm_secret/api/store/nginx/security")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({
                        "hide_version": true,
                        "gzip_enabled": true,
                        "client_max_body_mb": 100,
                        "rate_limit_enabled": true,
                        "rate_limit_rps": 15,
                        "rate_limit_burst": 30,
                        "block_sensitive_paths": true,
                        "block_bad_ua": true,
                        "blocked_ips": ["203.0.113.9"],
                        "custom_rules": ""
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(sec_resp.status(), StatusCode::OK);
    let sec_json = read_json(sec_resp).await;
    let rendered_sec = sec_json["data"]["rendered_conf"].as_str().unwrap();
    assert!(rendered_sec.contains("deny 203.0.113.9;"));
    assert!(rendered_sec.contains("rate=15r/s"));

    // 5. Backup & Migration: Create archive of website + docker + ssl + panel db, then restore & verify
    let bk_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/backups")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({
                        "name": "vps-migrate-pack",
                        "website_ids": [site_id],
                        "docker_project_ids": [docker_id],
                        "ssl_cert_ids": [cert_id],
                        "custom_dirs": [],
                        "include_panel_db": true
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bk_resp.status(), StatusCode::OK);
    let bk_json = read_json(bk_resp).await;
    let bk_id = bk_json["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(bk_json["data"]["sha256"].as_str().unwrap().len(), 64);

    let restore_target = dir.path().join("restored-check");
    let res_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/zm_secret/api/backups/{}/restore", bk_id))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({
                        "restore_dir": restore_target.to_string_lossy().replace('\\', "/")
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_resp.status(), StatusCode::OK);
    assert!(restore_target.join("manifest.json").is_file());
    assert!(restore_target.join("panel/zmpanel.db").is_file());
    assert!(restore_target.join(format!("websites/{}/files/README.md", site_id)).is_file());
    assert!(restore_target.join(format!("docker/{}/docker-compose.yml", docker_id)).is_file());
    assert!(restore_target.join(format!("ssl/{}/fullchain.pem", cert_id)).is_file());
}

#[tokio::test]
async fn test_stage3_firewall_security_ips_logs_cron_and_db_store() {
    let (state, dir) = make_test_state();
    let app = routes::build_app(state);

    let login_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"username":"admin","password":"TestPass123!"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let token = read_json(login_resp).await["data"]["token"]
        .as_str()
        .unwrap()
        .to_string();
    let auth_hdr = format!("Bearer {}", token);

    // 1. Firewall anti-lockout: denying panel port (18080) or SSH port (22) MUST fail with 403
    let lockout_panel = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/firewall/rules")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({"port":"18080","protocol":"tcp","action":"deny","source":"any"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(lockout_panel.status(), StatusCode::FORBIDDEN);

    let lockout_ssh = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/firewall/rules")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({"port":"20-25","protocol":"tcp","action":"deny","source":"any"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(lockout_ssh.status(), StatusCode::FORBIDDEN);

    // Allowing port 443 succeeds
    let allow_443 = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/firewall/rules")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({"port":"443","protocol":"tcp","action":"allow","source":"any","remark":"HTTPS"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(allow_443.status(), StatusCode::OK);

    // 2. Security Center: Whitelist + SSH brute force + Port scan + Web scan detection
    let wl_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/security/whitelist")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({"ip":"198.51.100.7","remark":"home-ip"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wl_resp.status(), StatusCode::OK);

    // SSH brute-force sample (6 failures from attacker + 6 from whitelisted IP)
    let ssh_log = (0..6)
        .map(|_| {
            "Nov 22 01:10:01 vps sshd[123]: Failed password for root from 203.0.113.88 port 51234 ssh2\n\
             Nov 22 01:10:02 vps sshd[124]: Failed password for root from 198.51.100.7 port 51235 ssh2"
        })
        .collect::<Vec<_>>()
        .join("\n");
    let ssh_det = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/security/analyze")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({"kind":"ssh","log_text": ssh_log}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ssh_det.status(), StatusCode::OK);
    // Only 203.0.113.88 should be banned; whitelisted 198.51.100.7 must NOT be banned
    assert_eq!(read_json(ssh_det).await["data"], 1);

    // Port scan sample (10 distinct ports from 203.0.113.99)
    let scan_log = (1000..1010)
        .map(|p| format!("kernel: [DROP] IN=eth0 SRC=203.0.113.99 DST=10.0.0.2 PROTO=TCP DPT={}", p))
        .collect::<Vec<_>>()
        .join("\n");
    let scan_det = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/security/analyze")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({"kind":"port_scan","log_text": scan_log}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read_json(scan_det).await["data"], 1);

    // Abnormal web scan sample (5 hits on /.env -> 15 risk score >= 12)
    let web_log = (0..5)
        .map(|_| "192.0.2.44 - - [22/Nov/2026:01:20:00 +0800] \"GET /.env HTTP/1.1\" 404 153 \"-\" \"scanner\"")
        .collect::<Vec<_>>()
        .join("\n");
    let web_det = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/security/analyze")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({"kind":"web","log_text": web_log}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read_json(web_det).await["data"], 1);

    let sec_overview = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/zm_secret/api/security/overview")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let sec_json = read_json(sec_overview).await;

    // Vulnerability & security baseline scan endpoint
    let vuln_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/security/vuln/scan")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(json!({"refresh_index": false}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(vuln_resp.status(), StatusCode::OK);
    let vuln_json = read_json(vuln_resp).await;
    assert!(vuln_json["data"]["security_score"].as_u64().unwrap() <= 100);
    assert!(!vuln_json["data"]["baselines"].as_array().unwrap().is_empty());

    // Save & read scheduled CVE watch config (defaults to alerting only on high/critical)
    let vw_save = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/zm_secret/api/security/vuln/config")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({
                        "enabled": true,
                        "interval_hours": 6,
                        "alert_min_severity": "high",
                        "auto_fix_non_kernel": false
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(vw_save.status(), StatusCode::OK);
    let vw_get = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/zm_secret/api/security/vuln/config")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let vw_json = read_json(vw_get).await;
    assert_eq!(vw_json["data"]["enabled"], true);
    assert_eq!(vw_json["data"]["alert_min_severity"], "high");
    assert_eq!(sec_json["data"]["bans"].as_array().unwrap().len(), 3);

    // 3. Log Center: tail with keyword filter + truncate in-place
    let sample_log = dir.path().join("app.log");
    std::fs::write(&sample_log, "INFO boot ok\nERROR db timeout\nINFO healthy\n").unwrap();
    let sample_log_str = sample_log.to_string_lossy().replace('\\', "/");
    let tail_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/zm_secret/api/logs/tail?path={}&q=ERROR",
                    sample_log_str
                ))
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(tail_resp.status(), StatusCode::OK);
    let tail_json = read_json(tail_resp).await;
    assert_eq!(tail_json["data"]["matched_lines"], 1);
    assert!(tail_json["data"]["content"].as_str().unwrap().contains("db timeout"));

    let trunc_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/logs/truncate")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(json!({"path": sample_log_str}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(trunc_resp.status(), StatusCode::OK);
    assert_eq!(std::fs::metadata(&sample_log).unwrap().len(), 0);

    // 4. Cron Jobs: reject invalid cron expr, create valid backup cron & execute immediately
    let bad_cron = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/cron/jobs")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({"name":"bad","kind":"cleanup","schedule":"invalid expr","target":""}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad_cron.status(), StatusCode::BAD_REQUEST);

    let ok_cron = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/cron/jobs")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({"name":"nightly-bk","kind":"backup","schedule":"0 3 * * *","target":"","keep_count":3}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ok_cron.status(), StatusCode::OK);
    let job_id = read_json(ok_cron).await["data"]["id"].as_str().unwrap().to_string();

    let run_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/zm_secret/api/cron/jobs/{}/run", job_id))
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(run_resp.status(), StatusCode::OK);
    assert_eq!(read_json(run_resp).await["data"]["success"], true);

    // 5. Store Redis/Postgres/MongoDB config & one-click Docker Workspace creation
    let redis_save = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/zm_secret/api/store/services/redis")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({"bind":"127.0.0.1","port":6380,"password":"secret","maxmemory_mb":48,"maxmemory_policy":"allkeys-lru","appendonly":true}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(redis_save.status(), StatusCode::OK);
    assert!(
        read_json(redis_save).await["data"]["rendered_compose"]
            .as_str()
            .unwrap()
            .contains("--maxmemory 48mb")
    );

    let to_docker = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/store/services/to-docker")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(json!({"service":"mongodb"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(to_docker.status(), StatusCode::OK);
    assert_eq!(read_json(to_docker).await["data"]["name"], "store-mongodb");
}

#[tokio::test]
async fn test_stage4_master_agent_reverse_ws_batch_and_cross_backup() {
    let (master_state, _master_dir) = make_test_state();
    let app = routes::build_app(master_state.clone());

    // Start real ephemeral TCP listener for Master so Agent can connect via outbound WebSocket
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Login on Master via in-process router
    let app_rpc = routes::build_app(master_state.clone());
    let login_resp = app_rpc
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"username":"admin","password":"TestPass123!"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let token = read_json(login_resp).await["data"]["token"]
        .as_str()
        .unwrap()
        .to_string();
    let auth_hdr = format!("Bearer {}", token);

    // 1. Register a NAT Agent node on Master
    let create_node_resp = app_rpc
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/cluster/nodes")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({"name": "nat-vps-tokyo", "group_name": "NAT小鸡"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_node_resp.status(), StatusCode::OK);
    let node_json = read_json(create_node_resp).await;
    let node_id = node_json["data"]["node"]["id"].as_str().unwrap().to_string();
    let node_token = node_json["data"]["node_token"].as_str().unwrap().to_string();

    // 2. Spin up a real Agent with its own isolated data dir & sample site, connecting outbound to Master
    let agent_dir = tempdir().unwrap();
    let agent_db = std::sync::Arc::new(
        Database::open(&agent_dir.path().join("zmpanel.db")).unwrap(),
    );
    let agent_site_dir = agent_dir.path().join("agent-web");
    std::fs::create_dir_all(&agent_site_dir).unwrap();
    std::fs::write(agent_site_dir.join("index.html"), "<h1>NAT VPS DATA</h1>").unwrap();
    let now = chrono::Utc::now().timestamp();
    agent_db
        .upsert_website(&zmpanel_core::WebsiteRecord {
            id: "site-nat-1".to_string(),
            name: "nat-demo".to_string(),
            kind: zmpanel_core::WebsiteKind::Static,
            domains: vec!["nat.example.test".to_string()],
            listen_port: 80,
            root_dir: agent_site_dir.to_string_lossy().replace('\\', "/"),
            start_cmd: String::new(),
            env_vars: String::new(),
            python_venv: None,
            proxy_target: None,
            websocket_enabled: false,
            spa_fallback: false,
            ssl_cert_id: None,
            force_https: false,
            auto_restart: false,
            custom_nginx: None,
            remark: String::new(),
            created_at: now,
            updated_at: now,
        })
        .unwrap();

    let agent_cfg = PanelConfig {
        mode: RunMode::Agent,
        bind_host: "127.0.0.1".to_string(),
        port: 0,
        data_dir: agent_dir.path().to_path_buf(),
        security_entry: String::new(),
        master_url: None,
        node_token: None,
        session_ttl_hours: 24,
        max_login_attempts: 5,
        lock_minutes: 10,
    };
    let ws_url = format!(
        "ws://{}/zm_secret/api/agent/ws?node_id={}&token={}",
        addr, node_id, node_token
    );
    let nid_c = node_id.clone();
    let agent_handle = tokio::spawn(async move {
        let _ = zmpanel_server::cluster::run_agent_once(agent_cfg, agent_db, nid_c, ws_url).await;
    });

    // Wait for Agent to connect and send Hello
    for _ in 0..30 {
        if master_state.cluster.is_online(&node_id) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(master_state.cluster.is_online(&node_id));

    // 3. Batch command execution over reverse WS
    let batch_resp = app_rpc
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/zm_secret/api/cluster/batch/exec")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({
                        "node_ids": [node_id],
                        "command": "echo reverse_ws_batch_ok"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(batch_resp.status(), StatusCode::OK);
    let batch_json = read_json(batch_resp).await;
    assert_eq!(batch_json["data"]["results"][0]["success"], true);
    assert!(
        batch_json["data"]["results"][0]["output"]
            .as_str()
            .unwrap()
            .contains("reverse_ws_batch_ok")
    );

    // 4. Pull backup directly from NAT Agent to Master central repository over reverse WS
    let pull_resp = app_rpc
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/zm_secret/api/cluster/nodes/{}/pull-backup", node_id))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({
                        "name": "nat-vps-snapshot",
                        "include_websites": true,
                        "include_docker": false,
                        "include_ssl": false,
                        "include_panel_db": true
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(pull_resp.status(), StatusCode::OK);
    let pull_json = read_json(pull_resp).await;
    let master_bk_id = pull_json["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(pull_json["data"]["sha256"].as_str().unwrap().len(), 64);

    // 5. Push restore from Master central repository back to target node & verify extracted payload
    let cross_restore_dir = agent_dir.path().join("migrated-target");
    let push_resp = app_rpc
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!(
                    "/zm_secret/api/cluster/backups/{}/push-restore",
                    master_bk_id
                ))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth_hdr)
                .body(Body::from(
                    json!({
                        "target_node_id": node_id,
                        "restore_dir": cross_restore_dir.to_string_lossy().replace('\\', "/")
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(push_resp.status(), StatusCode::OK);
    let restored_html =
        std::fs::read_to_string(cross_restore_dir.join("websites/site-nat-1/files/index.html"))
            .unwrap();
    assert_eq!(restored_html, "<h1>NAT VPS DATA</h1>");

    agent_handle.abort();
    server_handle.abort();
}




