use anyhow::Result;
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::Mutex;
use zmpanel_core::{
    AuditLogItem, BackupItem, ClusterJobRecord, ClusterJobResult, ClusterNodeRecord,
    CommandSnippet, CronJobItem, CronRunRecord, DockerProject, FirewallRule, MasterBackupRecord,
    MetricPoint, PythonVenvItem, SecurityBanItem, SecurityEventItem, SecurityWhitelistItem,
    SslCertificate, TrashItem, WebsiteKind, WebsiteRecord,
};

pub struct Database {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone)]
pub struct UserRecord {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub totp_secret: Option<String>,
    pub totp_enabled: bool,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;
             PRAGMA cache_size = -1024;
             PRAGMA mmap_size = 0;",
        )?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT UNIQUE NOT NULL,
                password_hash TEXT NOT NULL,
                totp_secret TEXT,
                totp_enabled INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS sessions (
                token_hash TEXT PRIMARY KEY,
                user_id INTEGER NOT NULL,
                ip TEXT NOT NULL,
                user_agent TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS metrics_history (
                ts INTEGER PRIMARY KEY,
                cpu_usage REAL NOT NULL,
                mem_used_pct REAL NOT NULL,
                swap_used_pct REAL NOT NULL,
                load_1m REAL NOT NULL,
                net_rx_rate INTEGER NOT NULL,
                net_tx_rate INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS monthly_traffic (
                month TEXT PRIMARY KEY,
                rx_bytes INTEGER NOT NULL DEFAULT 0,
                tx_bytes INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS command_snippets (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                category TEXT NOT NULL,
                command TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS trash_items (
                id TEXT PRIMARY KEY,
                original_path TEXT NOT NULL,
                name TEXT NOT NULL,
                is_dir INTEGER NOT NULL,
                size INTEGER NOT NULL,
                deleted_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS audit_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts INTEGER NOT NULL,
                actor TEXT NOT NULL,
                ip TEXT NOT NULL,
                action TEXT NOT NULL,
                target TEXT NOT NULL,
                detail TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_audit_ts ON audit_logs(ts DESC);

            CREATE TABLE IF NOT EXISTS websites (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                domains_json TEXT NOT NULL,
                listen_port INTEGER NOT NULL,
                root_dir TEXT NOT NULL,
                start_cmd TEXT NOT NULL DEFAULT '',
                env_vars TEXT NOT NULL DEFAULT '',
                python_venv TEXT,
                proxy_target TEXT,
                websocket_enabled INTEGER NOT NULL DEFAULT 1,
                spa_fallback INTEGER NOT NULL DEFAULT 0,
                ssl_cert_id TEXT,
                force_https INTEGER NOT NULL DEFAULT 0,
                auto_restart INTEGER NOT NULL DEFAULT 1,
                custom_nginx TEXT,
                remark TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS docker_projects (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                work_dir TEXT NOT NULL,
                compose_file TEXT NOT NULL DEFAULT 'docker-compose.yml',
                deploy_cmd TEXT NOT NULL DEFAULT 'docker compose up -d',
                description TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS ssl_certs (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                domains_json TEXT NOT NULL,
                issuer TEXT NOT NULL,
                provider TEXT NOT NULL,
                challenge_type TEXT NOT NULL,
                dns_provider TEXT,
                dns_credentials_json TEXT,
                auto_renew INTEGER NOT NULL DEFAULT 0,
                renew_hook TEXT NOT NULL DEFAULT '',
                not_before INTEGER NOT NULL,
                not_after INTEGER NOT NULL,
                cert_path TEXT NOT NULL,
                key_path TEXT NOT NULL,
                last_renew_status TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS python_venvs (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                path TEXT NOT NULL,
                python_version TEXT NOT NULL,
                index_url TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS backups (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                file_path TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                sha256 TEXT NOT NULL,
                target_repo TEXT NOT NULL DEFAULT 'local',
                scopes_json TEXT NOT NULL,
                summary TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS firewall_rules (
                id TEXT PRIMARY KEY,
                port TEXT NOT NULL,
                protocol TEXT NOT NULL,
                action TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'any',
                remark TEXT NOT NULL DEFAULT '',
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS security_bans (
                ip TEXT PRIMARY KEY,
                reason TEXT NOT NULL,
                category TEXT NOT NULL,
                hits INTEGER NOT NULL DEFAULT 1,
                banned_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS security_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts INTEGER NOT NULL,
                category TEXT NOT NULL,
                source_ip TEXT NOT NULL,
                summary TEXT NOT NULL,
                action_taken TEXT NOT NULL,
                sample TEXT NOT NULL DEFAULT ''
            );
            CREATE INDEX IF NOT EXISTS idx_sec_events_ts ON security_events(ts DESC);

            CREATE TABLE IF NOT EXISTS security_whitelist (
                ip TEXT PRIMARY KEY,
                remark TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS cron_jobs (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                schedule TEXT NOT NULL,
                target TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                keep_count INTEGER NOT NULL DEFAULT 5,
                last_run_at INTEGER NOT NULL DEFAULT 0,
                last_status TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS cron_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                job_id TEXT NOT NULL,
                job_name TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                duration_ms INTEGER NOT NULL,
                success INTEGER NOT NULL,
                output TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_cron_runs_job ON cron_runs(job_id, started_at DESC);

            CREATE TABLE IF NOT EXISTS cluster_nodes (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                group_name TEXT NOT NULL DEFAULT '默认分组',
                token_hash TEXT NOT NULL,
                hostname TEXT NOT NULL DEFAULT '',
                os_name TEXT NOT NULL DEFAULT '',
                arch TEXT NOT NULL DEFAULT '',
                agent_version TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'offline',
                cpu_usage REAL NOT NULL DEFAULT 0,
                mem_used_pct REAL NOT NULL DEFAULT 0,
                mem_total_bytes INTEGER NOT NULL DEFAULT 0,
                agent_rss_bytes INTEGER NOT NULL DEFAULT 0,
                net_rx_month INTEGER NOT NULL DEFAULT 0,
                net_tx_month INTEGER NOT NULL DEFAULT 0,
                last_seen INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS cluster_jobs (
                id TEXT PRIMARY KEY,
                action TEXT NOT NULL,
                payload_summary TEXT NOT NULL,
                results_json TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS master_backups (
                id TEXT PRIMARY KEY,
                node_id TEXT NOT NULL,
                node_name TEXT NOT NULL,
                name TEXT NOT NULL,
                file_path TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                sha256 TEXT NOT NULL,
                summary TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL
            );
            ",
        )?;

        // Seed helpful default terminal snippets if table is empty
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM command_snippets", [], |r| r.get(0))?;
        if count == 0 {
            let now = Utc::now().timestamp();
            let defaults = [
                (
                    "docker-ps",
                    "查看运行中容器",
                    "Docker",
                    "docker ps --format \"table {{.Names}}\\t{{.Status}}\\t{{.Ports}}\"",
                    "简洁列出正在运行的 Docker 容器及端口映射",
                ),
                (
                    "docker-compose-up",
                    "Compose 后台启动并拉取更新",
                    "Docker",
                    "docker compose pull && docker compose up -d",
                    "在当前目录拉取最新镜像并重新编排启动",
                ),
                (
                    "docker-prune",
                    "清理无用镜像与构建缓存",
                    "Docker",
                    "docker system prune -af",
                    "释放未使用的镜像、停止容器与缓存空间",
                ),
                (
                    "sys-ports",
                    "查看监听端口与进程",
                    "系统运维",
                    "ss -tulpn",
                    "快速定位 TCP/UDP 监听端口及对应进程",
                ),
                (
                    "sys-disk",
                    "查看目录空间占用排行",
                    "系统运维",
                    "du -h --max-depth=1 | sort -hr | head -n 20",
                    "排查当前目录下最占空间的文件夹",
                ),
                (
                    "nginx-reload",
                    "校验并热重载 Nginx",
                    "Web",
                    "nginx -t && nginx -s reload",
                    "修改 Nginx 配置后无中断重载",
                ),
            ];
            for (id, name, cat, cmd, desc) in defaults {
                conn.execute(
                    "INSERT INTO command_snippets (id, name, category, command, description, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![id, name, cat, cmd, desc, now],
                )?;
            }
        }
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> rusqlite::Result<Option<String>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, value, now],
        )?;
        Ok(())
    }

    pub fn user_count(&self) -> rusqlite::Result<i64> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))
    }

    pub fn create_user(&self, username: &str, password_hash: &str) -> rusqlite::Result<i64> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO users (username, password_hash, totp_enabled, created_at, updated_at)
             VALUES (?1, ?2, 0, ?3, ?3)",
            params![username, password_hash, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn find_user_by_username(&self, username: &str) -> rusqlite::Result<Option<UserRecord>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.query_row(
            "SELECT id, username, password_hash, totp_secret, totp_enabled FROM users WHERE username = ?1",
            params![username],
            |r| {
                Ok(UserRecord {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    password_hash: r.get(2)?,
                    totp_secret: r.get(3)?,
                    totp_enabled: r.get::<_, i64>(4)? != 0,
                })
            },
        )
        .optional()
    }

    pub fn find_user_by_id(&self, id: i64) -> rusqlite::Result<Option<UserRecord>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.query_row(
            "SELECT id, username, password_hash, totp_secret, totp_enabled FROM users WHERE id = ?1",
            params![id],
            |r| {
                Ok(UserRecord {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    password_hash: r.get(2)?,
                    totp_secret: r.get(3)?,
                    totp_enabled: r.get::<_, i64>(4)? != 0,
                })
            },
        )
        .optional()
    }

    pub fn update_user_password(&self, user_id: i64, password_hash: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let now = Utc::now().timestamp();
        conn.execute(
            "UPDATE users SET password_hash = ?1, updated_at = ?2 WHERE id = ?3",
            params![password_hash, now, user_id],
        )?;
        Ok(())
    }

    pub fn update_user_totp(
        &self,
        user_id: i64,
        secret: Option<&str>,
        enabled: bool,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let now = Utc::now().timestamp();
        conn.execute(
            "UPDATE users SET totp_secret = ?1, totp_enabled = ?2, updated_at = ?3 WHERE id = ?4",
            params![secret, if enabled { 1 } else { 0 }, now, user_id],
        )?;
        Ok(())
    }

    pub fn create_session(
        &self,
        token_hash: &str,
        user_id: i64,
        ip: &str,
        user_agent: &str,
        expires_at: i64,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO sessions (token_hash, user_id, ip, user_agent, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![token_hash, user_id, ip, user_agent, now, expires_at],
        )?;
        Ok(())
    }

    pub fn find_session_user(&self, token_hash: &str) -> rusqlite::Result<Option<UserRecord>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let now = Utc::now().timestamp();
        conn.query_row(
            "SELECT u.id, u.username, u.password_hash, u.totp_secret, u.totp_enabled
             FROM sessions s
             JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = ?1 AND s.expires_at > ?2",
            params![token_hash, now],
            |r| {
                Ok(UserRecord {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    password_hash: r.get(2)?,
                    totp_secret: r.get(3)?,
                    totp_enabled: r.get::<_, i64>(4)? != 0,
                })
            },
        )
        .optional()
    }

    pub fn delete_session(&self, token_hash: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "DELETE FROM sessions WHERE token_hash = ?1",
            params![token_hash],
        )?;
        Ok(())
    }

    pub fn delete_user_sessions(&self, user_id: i64) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute("DELETE FROM sessions WHERE user_id = ?1", params![user_id])?;
        Ok(())
    }

    pub fn insert_metric(&self, point: &MetricPoint) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "INSERT OR REPLACE INTO metrics_history
             (ts, cpu_usage, mem_used_pct, swap_used_pct, load_1m, net_rx_rate, net_tx_rate)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                point.timestamp,
                point.cpu_usage,
                point.mem_used_pct,
                point.swap_used_pct,
                point.load_1m,
                point.net_rx_rate as i64,
                point.net_tx_rate as i64
            ],
        )?;
        // Keep last 7 days of metrics to keep DB tiny on 256MB VPS
        let cutoff = Utc::now().timestamp() - 7 * 86400;
        conn.execute("DELETE FROM metrics_history WHERE ts < ?1", params![cutoff])?;
        Ok(())
    }

    pub fn query_metrics(&self, since_ts: i64, max_points: usize) -> rusqlite::Result<Vec<MetricPoint>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT ts, cpu_usage, mem_used_pct, swap_used_pct, load_1m, net_rx_rate, net_tx_rate
             FROM metrics_history
             WHERE ts >= ?1
             ORDER BY ts ASC",
        )?;
        let rows = stmt
            .query_map(params![since_ts], |r| {
                Ok(MetricPoint {
                    timestamp: r.get(0)?,
                    cpu_usage: r.get(1)?,
                    mem_used_pct: r.get(2)?,
                    swap_used_pct: r.get(3)?,
                    load_1m: r.get(4)?,
                    net_rx_rate: r.get::<_, i64>(5)?.max(0) as u64,
                    net_tx_rate: r.get::<_, i64>(6)?.max(0) as u64,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        if rows.len() <= max_points || max_points == 0 {
            return Ok(rows);
        }
        // Uniform downsampling for smooth rendering
        let step = (rows.len() as f64) / (max_points as f64);
        let mut sampled = Vec::with_capacity(max_points);
        for i in 0..max_points {
            let idx = ((i as f64) * step).floor() as usize;
            if let Some(item) = rows.get(idx) {
                sampled.push(item.clone());
            }
        }
        Ok(sampled)
    }

    pub fn add_monthly_traffic(&self, month: &str, rx_delta: u64, tx_delta: u64) -> rusqlite::Result<(u64, u64)> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO monthly_traffic (month, rx_bytes, tx_bytes, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(month) DO UPDATE SET
               rx_bytes = rx_bytes + excluded.rx_bytes,
               tx_bytes = tx_bytes + excluded.tx_bytes,
               updated_at = excluded.updated_at",
            params![month, rx_delta as i64, tx_delta as i64, now],
        )?;
        let (rx, tx): (i64, i64) = conn.query_row(
            "SELECT rx_bytes, tx_bytes FROM monthly_traffic WHERE month = ?1",
            params![month],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((rx.max(0) as u64, tx.max(0) as u64))
    }

    pub fn get_monthly_traffic(&self, month: &str) -> rusqlite::Result<(u64, u64)> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let res = conn
            .query_row(
                "SELECT rx_bytes, tx_bytes FROM monthly_traffic WHERE month = ?1",
                params![month],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?;
        Ok(res
            .map(|(rx, tx)| (rx.max(0) as u64, tx.max(0) as u64))
            .unwrap_or((0, 0)))
    }

    pub fn list_snippets(&self) -> rusqlite::Result<Vec<CommandSnippet>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, category, command, description, updated_at
             FROM command_snippets ORDER BY category ASC, name ASC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(CommandSnippet {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    category: r.get(2)?,
                    command: r.get(3)?,
                    description: r.get(4)?,
                    updated_at: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn upsert_snippet(&self, item: &CommandSnippet) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "INSERT INTO command_snippets (id, name, category, command, description, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               category = excluded.category,
               command = excluded.command,
               description = excluded.description,
               updated_at = excluded.updated_at",
            params![
                item.id,
                item.name,
                item.category,
                item.command,
                item.description,
                item.updated_at
            ],
        )?;
        Ok(())
    }

    pub fn delete_snippet(&self, id: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute("DELETE FROM command_snippets WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn insert_trash_item(&self, item: &TrashItem) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "INSERT INTO trash_items (id, original_path, name, is_dir, size, deleted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                item.id,
                item.original_path,
                item.name,
                if item.is_dir { 1 } else { 0 },
                item.size as i64,
                item.deleted_at
            ],
        )?;
        Ok(())
    }

    pub fn list_trash_items(&self) -> rusqlite::Result<Vec<TrashItem>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, original_path, name, is_dir, size, deleted_at
             FROM trash_items ORDER BY deleted_at DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(TrashItem {
                    id: r.get(0)?,
                    original_path: r.get(1)?,
                    name: r.get(2)?,
                    is_dir: r.get::<_, i64>(3)? != 0,
                    size: r.get::<_, i64>(4)?.max(0) as u64,
                    deleted_at: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_trash_item(&self, id: &str) -> rusqlite::Result<Option<TrashItem>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.query_row(
            "SELECT id, original_path, name, is_dir, size, deleted_at FROM trash_items WHERE id = ?1",
            params![id],
            |r| {
                Ok(TrashItem {
                    id: r.get(0)?,
                    original_path: r.get(1)?,
                    name: r.get(2)?,
                    is_dir: r.get::<_, i64>(3)? != 0,
                    size: r.get::<_, i64>(4)?.max(0) as u64,
                    deleted_at: r.get(5)?,
                })
            },
        )
        .optional()
    }

    pub fn delete_trash_item(&self, id: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute("DELETE FROM trash_items WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn clear_trash_items(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute("DELETE FROM trash_items", [])?;
        Ok(())
    }

    pub fn write_audit(
        &self,
        actor: &str,
        ip: &str,
        action: &str,
        target: &str,
        detail: &str,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO audit_logs (ts, actor, ip, action, target, detail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![now, actor, ip, action, target, detail],
        )?;
        Ok(())
    }

    pub fn list_audit_logs(&self, limit: usize) -> rusqlite::Result<Vec<AuditLogItem>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, ts, actor, ip, action, target, detail
             FROM audit_logs ORDER BY ts DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                Ok(AuditLogItem {
                    id: r.get(0)?,
                    timestamp: r.get(1)?,
                    actor: r.get(2)?,
                    ip: r.get(3)?,
                    action: r.get(4)?,
                    target: r.get(5)?,
                    detail: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ---------- Websites CRUD ----------

    pub fn list_websites(&self) -> rusqlite::Result<Vec<WebsiteRecord>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, kind, domains_json, listen_port, root_dir, start_cmd, env_vars,
                    python_venv, proxy_target, websocket_enabled, spa_fallback, ssl_cert_id,
                    force_https, auto_restart, custom_nginx, remark, created_at, updated_at
             FROM websites ORDER BY updated_at DESC",
        )?;
        let rows = stmt
            .query_map([], map_website_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_website(&self, id: &str) -> rusqlite::Result<Option<WebsiteRecord>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.query_row(
            "SELECT id, name, kind, domains_json, listen_port, root_dir, start_cmd, env_vars,
                    python_venv, proxy_target, websocket_enabled, spa_fallback, ssl_cert_id,
                    force_https, auto_restart, custom_nginx, remark, created_at, updated_at
             FROM websites WHERE id = ?1",
            params![id],
            map_website_row,
        )
        .optional()
    }

    pub fn upsert_website(&self, site: &WebsiteRecord) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let domains_json = serde_json::to_string(&site.domains).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "INSERT INTO websites (
                id, name, kind, domains_json, listen_port, root_dir, start_cmd, env_vars,
                python_venv, proxy_target, websocket_enabled, spa_fallback, ssl_cert_id,
                force_https, auto_restart, custom_nginx, remark, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                kind = excluded.kind,
                domains_json = excluded.domains_json,
                listen_port = excluded.listen_port,
                root_dir = excluded.root_dir,
                start_cmd = excluded.start_cmd,
                env_vars = excluded.env_vars,
                python_venv = excluded.python_venv,
                proxy_target = excluded.proxy_target,
                websocket_enabled = excluded.websocket_enabled,
                spa_fallback = excluded.spa_fallback,
                ssl_cert_id = excluded.ssl_cert_id,
                force_https = excluded.force_https,
                auto_restart = excluded.auto_restart,
                custom_nginx = excluded.custom_nginx,
                remark = excluded.remark,
                updated_at = excluded.updated_at",
            params![
                site.id,
                site.name,
                site.kind.as_str(),
                domains_json,
                site.listen_port as i64,
                site.root_dir,
                site.start_cmd,
                site.env_vars,
                site.python_venv,
                site.proxy_target,
                if site.websocket_enabled { 1 } else { 0 },
                if site.spa_fallback { 1 } else { 0 },
                site.ssl_cert_id,
                if site.force_https { 1 } else { 0 },
                if site.auto_restart { 1 } else { 0 },
                site.custom_nginx,
                site.remark,
                site.created_at,
                site.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn delete_website(&self, id: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute("DELETE FROM websites WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ---------- Docker Projects CRUD ----------

    pub fn list_docker_projects(&self) -> rusqlite::Result<Vec<DockerProject>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, work_dir, compose_file, deploy_cmd, description, created_at, updated_at
             FROM docker_projects ORDER BY updated_at DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(DockerProject {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    work_dir: r.get(2)?,
                    compose_file: r.get(3)?,
                    deploy_cmd: r.get(4)?,
                    description: r.get(5)?,
                    created_at: r.get(6)?,
                    updated_at: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_docker_project(&self, id: &str) -> rusqlite::Result<Option<DockerProject>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.query_row(
            "SELECT id, name, work_dir, compose_file, deploy_cmd, description, created_at, updated_at
             FROM docker_projects WHERE id = ?1",
            params![id],
            |r| {
                Ok(DockerProject {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    work_dir: r.get(2)?,
                    compose_file: r.get(3)?,
                    deploy_cmd: r.get(4)?,
                    description: r.get(5)?,
                    created_at: r.get(6)?,
                    updated_at: r.get(7)?,
                })
            },
        )
        .optional()
    }

    pub fn upsert_docker_project(&self, p: &DockerProject) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "INSERT INTO docker_projects (id, name, work_dir, compose_file, deploy_cmd, description, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               work_dir = excluded.work_dir,
               compose_file = excluded.compose_file,
               deploy_cmd = excluded.deploy_cmd,
               description = excluded.description,
               updated_at = excluded.updated_at",
            params![
                p.id,
                p.name,
                p.work_dir,
                p.compose_file,
                p.deploy_cmd,
                p.description,
                p.created_at,
                p.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn delete_docker_project(&self, id: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute("DELETE FROM docker_projects WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ---------- SSL Certificates CRUD ----------

    pub fn list_ssl_certs(&self) -> rusqlite::Result<Vec<SslCertificate>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, domains_json, issuer, provider, challenge_type, dns_provider,
                    auto_renew, renew_hook, not_before, not_after, cert_path, key_path,
                    last_renew_status, created_at, updated_at
             FROM ssl_certs ORDER BY not_after ASC",
        )?;
        let rows = stmt
            .query_map([], map_ssl_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_ssl_cert(&self, id: &str) -> rusqlite::Result<Option<SslCertificate>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.query_row(
            "SELECT id, name, domains_json, issuer, provider, challenge_type, dns_provider,
                    auto_renew, renew_hook, not_before, not_after, cert_path, key_path,
                    last_renew_status, created_at, updated_at
             FROM ssl_certs WHERE id = ?1",
            params![id],
            map_ssl_row,
        )
        .optional()
    }

    pub fn upsert_ssl_cert(
        &self,
        cert: &SslCertificate,
        dns_credentials_json: Option<&str>,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let domains_json =
            serde_json::to_string(&cert.domains).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "INSERT INTO ssl_certs (
                id, name, domains_json, issuer, provider, challenge_type, dns_provider,
                dns_credentials_json, auto_renew, renew_hook, not_before, not_after,
                cert_path, key_path, last_renew_status, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                domains_json = excluded.domains_json,
                issuer = excluded.issuer,
                provider = excluded.provider,
                challenge_type = excluded.challenge_type,
                dns_provider = excluded.dns_provider,
                dns_credentials_json = COALESCE(excluded.dns_credentials_json, ssl_certs.dns_credentials_json),
                auto_renew = excluded.auto_renew,
                renew_hook = excluded.renew_hook,
                not_before = excluded.not_before,
                not_after = excluded.not_after,
                cert_path = excluded.cert_path,
                key_path = excluded.key_path,
                last_renew_status = excluded.last_renew_status,
                updated_at = excluded.updated_at",
            params![
                cert.id,
                cert.name,
                domains_json,
                cert.issuer,
                cert.provider,
                cert.challenge_type,
                cert.dns_provider,
                dns_credentials_json,
                if cert.auto_renew { 1 } else { 0 },
                cert.renew_hook,
                cert.not_before,
                cert.not_after,
                cert.cert_path,
                cert.key_path,
                cert.last_renew_status,
                cert.created_at,
                cert.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn delete_ssl_cert(&self, id: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute("DELETE FROM ssl_certs WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ---------- Python Venvs CRUD ----------

    pub fn list_python_venvs(&self) -> rusqlite::Result<Vec<PythonVenvItem>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, path, python_version, index_url, created_at
             FROM python_venvs ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(PythonVenvItem {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    path: r.get(2)?,
                    python_version: r.get(3)?,
                    index_url: r.get(4)?,
                    created_at: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn insert_python_venv(&self, v: &PythonVenvItem) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "INSERT INTO python_venvs (id, name, path, python_version, index_url, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![v.id, v.name, v.path, v.python_version, v.index_url, v.created_at],
        )?;
        Ok(())
    }

    pub fn delete_python_venv(&self, id: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute("DELETE FROM python_venvs WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ---------- Backups CRUD ----------

    pub fn list_backups(&self) -> rusqlite::Result<Vec<BackupItem>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, file_path, size_bytes, sha256, target_repo, scopes_json, summary, created_at
             FROM backups ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                let scopes_raw: String = r.get(6)?;
                let scopes: Vec<String> = serde_json::from_str(&scopes_raw).unwrap_or_default();
                Ok(BackupItem {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    file_path: r.get(2)?,
                    size_bytes: r.get::<_, i64>(3)?.max(0) as u64,
                    sha256: r.get(4)?,
                    target_repo: r.get(5)?,
                    scopes,
                    summary: r.get(7)?,
                    created_at: r.get(8)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_backup(&self, id: &str) -> rusqlite::Result<Option<BackupItem>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.query_row(
            "SELECT id, name, file_path, size_bytes, sha256, target_repo, scopes_json, summary, created_at
             FROM backups WHERE id = ?1",
            params![id],
            |r| {
                let scopes_raw: String = r.get(6)?;
                let scopes: Vec<String> = serde_json::from_str(&scopes_raw).unwrap_or_default();
                Ok(BackupItem {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    file_path: r.get(2)?,
                    size_bytes: r.get::<_, i64>(3)?.max(0) as u64,
                    sha256: r.get(4)?,
                    target_repo: r.get(5)?,
                    scopes,
                    summary: r.get(7)?,
                    created_at: r.get(8)?,
                })
            },
        )
        .optional()
    }

    pub fn insert_backup(&self, item: &BackupItem) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let scopes_json = serde_json::to_string(&item.scopes).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "INSERT INTO backups (id, name, file_path, size_bytes, sha256, target_repo, scopes_json, summary, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                item.id,
                item.name,
                item.file_path,
                item.size_bytes as i64,
                item.sha256,
                item.target_repo,
                scopes_json,
                item.summary,
                item.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn delete_backup(&self, id: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute("DELETE FROM backups WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ---------- Firewall Rules CRUD ----------

    pub fn list_firewall_rules(&self) -> rusqlite::Result<Vec<FirewallRule>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, port, protocol, action, source, remark, enabled, created_at
             FROM firewall_rules ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(FirewallRule {
                    id: r.get(0)?,
                    port: r.get(1)?,
                    protocol: r.get(2)?,
                    action: r.get(3)?,
                    source: r.get(4)?,
                    remark: r.get(5)?,
                    enabled: r.get::<_, i64>(6)? != 0,
                    created_at: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_firewall_rule(&self, id: &str) -> rusqlite::Result<Option<FirewallRule>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.query_row(
            "SELECT id, port, protocol, action, source, remark, enabled, created_at
             FROM firewall_rules WHERE id = ?1",
            params![id],
            |r| {
                Ok(FirewallRule {
                    id: r.get(0)?,
                    port: r.get(1)?,
                    protocol: r.get(2)?,
                    action: r.get(3)?,
                    source: r.get(4)?,
                    remark: r.get(5)?,
                    enabled: r.get::<_, i64>(6)? != 0,
                    created_at: r.get(7)?,
                })
            },
        )
        .optional()
    }

    pub fn upsert_firewall_rule(&self, r: &FirewallRule) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "INSERT INTO firewall_rules (id, port, protocol, action, source, remark, enabled, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
               port = excluded.port,
               protocol = excluded.protocol,
               action = excluded.action,
               source = excluded.source,
               remark = excluded.remark,
               enabled = excluded.enabled",
            params![
                r.id,
                r.port,
                r.protocol,
                r.action,
                r.source,
                r.remark,
                if r.enabled { 1 } else { 0 },
                r.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn delete_firewall_rule(&self, id: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute("DELETE FROM firewall_rules WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ---------- Security Bans / Events / Whitelist ----------

    pub fn list_security_bans(&self) -> rusqlite::Result<Vec<SecurityBanItem>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT ip, reason, category, hits, banned_at, expires_at
             FROM security_bans ORDER BY banned_at DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SecurityBanItem {
                    ip: r.get(0)?,
                    reason: r.get(1)?,
                    category: r.get(2)?,
                    hits: r.get::<_, i64>(3)?.max(0) as u32,
                    banned_at: r.get(4)?,
                    expires_at: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn upsert_security_ban(&self, ban: &SecurityBanItem) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "INSERT INTO security_bans (ip, reason, category, hits, banned_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(ip) DO UPDATE SET
               reason = excluded.reason,
               category = excluded.category,
               hits = security_bans.hits + excluded.hits,
               banned_at = excluded.banned_at,
               expires_at = excluded.expires_at",
            params![
                ban.ip,
                ban.reason,
                ban.category,
                ban.hits as i64,
                ban.banned_at,
                ban.expires_at,
            ],
        )?;
        Ok(())
    }

    pub fn delete_security_ban(&self, ip: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute("DELETE FROM security_bans WHERE ip = ?1", params![ip])?;
        Ok(())
    }

    pub fn purge_expired_bans(&self, now_ts: i64) -> rusqlite::Result<Vec<String>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT ip FROM security_bans WHERE expires_at > 0 AND expires_at <= ?1",
        )?;
        let expired = stmt
            .query_map(params![now_ts], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !expired.is_empty() {
            conn.execute(
                "DELETE FROM security_bans WHERE expires_at > 0 AND expires_at <= ?1",
                params![now_ts],
            )?;
        }
        Ok(expired)
    }

    pub fn insert_security_event(
        &self,
        category: &str,
        source_ip: &str,
        summary: &str,
        action_taken: &str,
        sample: &str,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO security_events (ts, category, source_ip, summary, action_taken, sample)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![now, category, source_ip, summary, action_taken, sample],
        )?;
        // Cap stored security events at latest 2000 rows to protect small VPS disk
        conn.execute(
            "DELETE FROM security_events WHERE id NOT IN (SELECT id FROM security_events ORDER BY id DESC LIMIT 2000)",
            [],
        )?;
        Ok(())
    }

    pub fn list_security_events(&self, limit: usize) -> rusqlite::Result<Vec<SecurityEventItem>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, ts, category, source_ip, summary, action_taken, sample
             FROM security_events ORDER BY ts DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                Ok(SecurityEventItem {
                    id: r.get(0)?,
                    timestamp: r.get(1)?,
                    category: r.get(2)?,
                    source_ip: r.get(3)?,
                    summary: r.get(4)?,
                    action_taken: r.get(5)?,
                    sample: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn list_security_whitelist(&self) -> rusqlite::Result<Vec<SecurityWhitelistItem>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT ip, remark, created_at FROM security_whitelist ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SecurityWhitelistItem {
                    ip: r.get(0)?,
                    remark: r.get(1)?,
                    created_at: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn is_ip_whitelisted(&self, ip: &str) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let cnt: i64 = conn.query_row(
            "SELECT COUNT(*) FROM security_whitelist WHERE ip = ?1",
            params![ip],
            |r| r.get(0),
        )?;
        Ok(cnt > 0)
    }

    pub fn upsert_security_whitelist(&self, ip: &str, remark: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO security_whitelist (ip, remark, created_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(ip) DO UPDATE SET remark = excluded.remark",
            params![ip, remark, now],
        )?;
        Ok(())
    }

    pub fn delete_security_whitelist(&self, ip: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute("DELETE FROM security_whitelist WHERE ip = ?1", params![ip])?;
        Ok(())
    }

    // ---------- Cron Jobs & Run History ----------

    pub fn list_cron_jobs(&self) -> rusqlite::Result<Vec<CronJobItem>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, kind, schedule, target, enabled, keep_count, last_run_at, last_status, created_at, updated_at
             FROM cron_jobs ORDER BY updated_at DESC",
        )?;
        let rows = stmt
            .query_map([], map_cron_job_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_cron_job(&self, id: &str) -> rusqlite::Result<Option<CronJobItem>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.query_row(
            "SELECT id, name, kind, schedule, target, enabled, keep_count, last_run_at, last_status, created_at, updated_at
             FROM cron_jobs WHERE id = ?1",
            params![id],
            map_cron_job_row,
        )
        .optional()
    }

    pub fn upsert_cron_job(&self, job: &CronJobItem) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "INSERT INTO cron_jobs (id, name, kind, schedule, target, enabled, keep_count, last_run_at, last_status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               kind = excluded.kind,
               schedule = excluded.schedule,
               target = excluded.target,
               enabled = excluded.enabled,
               keep_count = excluded.keep_count,
               updated_at = excluded.updated_at",
            params![
                job.id,
                job.name,
                job.kind,
                job.schedule,
                job.target,
                if job.enabled { 1 } else { 0 },
                job.keep_count as i64,
                job.last_run_at,
                job.last_status,
                job.created_at,
                job.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_cron_job_run_state(&self, id: &str, run_at: i64, status: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "UPDATE cron_jobs SET last_run_at = ?1, last_status = ?2 WHERE id = ?3",
            params![run_at, status, id],
        )?;
        Ok(())
    }

    pub fn delete_cron_job(&self, id: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute("DELETE FROM cron_runs WHERE job_id = ?1", params![id])?;
        conn.execute("DELETE FROM cron_jobs WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn insert_cron_run(
        &self,
        job_id: &str,
        job_name: &str,
        started_at: i64,
        duration_ms: i64,
        success: bool,
        output: &str,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "INSERT INTO cron_runs (job_id, job_name, started_at, duration_ms, success, output)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                job_id,
                job_name,
                started_at,
                duration_ms,
                if success { 1 } else { 0 },
                output,
            ],
        )?;
        conn.execute(
            "DELETE FROM cron_runs WHERE job_id = ?1 AND id NOT IN (SELECT id FROM cron_runs WHERE job_id = ?1 ORDER BY id DESC LIMIT 50)",
            params![job_id],
        )?;
        Ok(())
    }

    pub fn list_cron_runs(&self, job_id: Option<&str>, limit: usize) -> rusqlite::Result<Vec<CronRunRecord>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        if let Some(jid) = job_id {
            let mut stmt = conn.prepare(
                "SELECT id, job_id, job_name, started_at, duration_ms, success, output
                 FROM cron_runs WHERE job_id = ?1 ORDER BY started_at DESC, id DESC LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(params![jid, limit as i64], map_cron_run_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, job_id, job_name, started_at, duration_ms, success, output
                 FROM cron_runs ORDER BY started_at DESC, id DESC LIMIT ?1",
            )?;
            let rows = stmt
                .query_map(params![limit as i64], map_cron_run_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        }
    }

    // ---------- Stage 4: Cluster Nodes / Jobs / Master Backups ----------

    pub fn create_cluster_node(
        &self,
        id: &str,
        name: &str,
        group_name: &str,
        token_hash: &str,
    ) -> rusqlite::Result<ClusterNodeRecord> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO cluster_nodes (id, name, group_name, token_hash, status, created_at)
             VALUES (?1, ?2, ?3, ?4, 'offline', ?5)",
            params![id, name, group_name, token_hash, now],
        )?;
        Ok(ClusterNodeRecord {
            id: id.to_string(),
            name: name.to_string(),
            group_name: group_name.to_string(),
            hostname: String::new(),
            os_name: String::new(),
            arch: String::new(),
            agent_version: String::new(),
            status: "offline".to_string(),
            cpu_usage: 0.0,
            mem_used_pct: 0.0,
            mem_total_bytes: 0,
            agent_rss_bytes: 0,
            net_rx_month: 0,
            net_tx_month: 0,
            last_seen: 0,
            created_at: now,
        })
    }

    pub fn verify_cluster_node_token(&self, id: &str, token_hash: &str) -> rusqlite::Result<Option<ClusterNodeRecord>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.query_row(
            "SELECT id, name, group_name, hostname, os_name, arch, agent_version, status,
                    cpu_usage, mem_used_pct, mem_total_bytes, agent_rss_bytes,
                    net_rx_month, net_tx_month, last_seen, created_at
             FROM cluster_nodes WHERE id = ?1 AND token_hash = ?2",
            params![id, token_hash],
            map_cluster_node_row,
        )
        .optional()
    }

    pub fn list_cluster_nodes(&self) -> rusqlite::Result<Vec<ClusterNodeRecord>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, group_name, hostname, os_name, arch, agent_version, status,
                    cpu_usage, mem_used_pct, mem_total_bytes, agent_rss_bytes,
                    net_rx_month, net_tx_month, last_seen, created_at
             FROM cluster_nodes ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map([], map_cluster_node_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_cluster_node(&self, id: &str) -> rusqlite::Result<Option<ClusterNodeRecord>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.query_row(
            "SELECT id, name, group_name, hostname, os_name, arch, agent_version, status,
                    cpu_usage, mem_used_pct, mem_total_bytes, agent_rss_bytes,
                    net_rx_month, net_tx_month, last_seen, created_at
             FROM cluster_nodes WHERE id = ?1",
            params![id],
            map_cluster_node_row,
        )
        .optional()
    }

    pub fn update_cluster_node_hello(
        &self,
        id: &str,
        hostname: &str,
        os_name: &str,
        arch: &str,
        version: &str,
        mem_total_bytes: u64,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let now = Utc::now().timestamp();
        conn.execute(
            "UPDATE cluster_nodes SET hostname = ?1, os_name = ?2, arch = ?3, agent_version = ?4,
             mem_total_bytes = ?5, status = 'online', last_seen = ?6 WHERE id = ?7",
            params![hostname, os_name, arch, version, mem_total_bytes as i64, now, id],
        )?;
        Ok(())
    }

    pub fn update_cluster_node_heartbeat(
        &self,
        id: &str,
        cpu_usage: f32,
        mem_used_pct: f32,
        mem_total_bytes: u64,
        agent_rss_bytes: u64,
        net_rx_month: u64,
        net_tx_month: u64,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let now = Utc::now().timestamp();
        conn.execute(
            "UPDATE cluster_nodes SET cpu_usage = ?1, mem_used_pct = ?2, mem_total_bytes = ?3,
             agent_rss_bytes = ?4, net_rx_month = ?5, net_tx_month = ?6,
             status = 'online', last_seen = ?7 WHERE id = ?8",
            params![
                cpu_usage,
                mem_used_pct,
                mem_total_bytes as i64,
                agent_rss_bytes as i64,
                net_rx_month as i64,
                net_tx_month as i64,
                now,
                id
            ],
        )?;
        Ok(())
    }

    pub fn set_cluster_node_offline(&self, id: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "UPDATE cluster_nodes SET status = 'offline' WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn delete_cluster_node(&self, id: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute("DELETE FROM cluster_nodes WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn insert_cluster_job(&self, job: &ClusterJobRecord) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let res_json = serde_json::to_string(&job.results).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "INSERT INTO cluster_jobs (id, action, payload_summary, results_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![job.id, job.action, job.payload_summary, res_json, job.created_at],
        )?;
        conn.execute(
            "DELETE FROM cluster_jobs WHERE id NOT IN (SELECT id FROM cluster_jobs ORDER BY created_at DESC LIMIT 80)",
            [],
        )?;
        Ok(())
    }

    pub fn list_cluster_jobs(&self, limit: usize) -> rusqlite::Result<Vec<ClusterJobRecord>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, action, payload_summary, results_json, created_at
             FROM cluster_jobs ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                let raw: String = r.get(3)?;
                let results: Vec<ClusterJobResult> =
                    serde_json::from_str(&raw).unwrap_or_default();
                Ok(ClusterJobRecord {
                    id: r.get(0)?,
                    action: r.get(1)?,
                    payload_summary: r.get(2)?,
                    results,
                    created_at: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn insert_master_backup(&self, bk: &MasterBackupRecord) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute(
            "INSERT INTO master_backups (id, node_id, node_name, name, file_path, size_bytes, sha256, summary, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                bk.id,
                bk.node_id,
                bk.node_name,
                bk.name,
                bk.file_path,
                bk.size_bytes as i64,
                bk.sha256,
                bk.summary,
                bk.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn list_master_backups(&self) -> rusqlite::Result<Vec<MasterBackupRecord>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, node_id, node_name, name, file_path, size_bytes, sha256, summary, created_at
             FROM master_backups ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map([], map_master_backup_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_master_backup(&self, id: &str) -> rusqlite::Result<Option<MasterBackupRecord>> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.query_row(
            "SELECT id, node_id, node_name, name, file_path, size_bytes, sha256, summary, created_at
             FROM master_backups WHERE id = ?1",
            params![id],
            map_master_backup_row,
        )
        .optional()
    }

    pub fn delete_master_backup(&self, id: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("db lock poisoned");
        conn.execute("DELETE FROM master_backups WHERE id = ?1", params![id])?;
        Ok(())
    }
}

fn map_cluster_node_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ClusterNodeRecord> {
    Ok(ClusterNodeRecord {
        id: r.get(0)?,
        name: r.get(1)?,
        group_name: r.get(2)?,
        hostname: r.get(3)?,
        os_name: r.get(4)?,
        arch: r.get(5)?,
        agent_version: r.get(6)?,
        status: r.get(7)?,
        cpu_usage: r.get(8)?,
        mem_used_pct: r.get(9)?,
        mem_total_bytes: r.get::<_, i64>(10)?.max(0) as u64,
        agent_rss_bytes: r.get::<_, i64>(11)?.max(0) as u64,
        net_rx_month: r.get::<_, i64>(12)?.max(0) as u64,
        net_tx_month: r.get::<_, i64>(13)?.max(0) as u64,
        last_seen: r.get(14)?,
        created_at: r.get(15)?,
    })
}

fn map_master_backup_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<MasterBackupRecord> {
    Ok(MasterBackupRecord {
        id: r.get(0)?,
        node_id: r.get(1)?,
        node_name: r.get(2)?,
        name: r.get(3)?,
        file_path: r.get(4)?,
        size_bytes: r.get::<_, i64>(5)?.max(0) as u64,
        sha256: r.get(6)?,
        summary: r.get(7)?,
        created_at: r.get(8)?,
    })
}

fn map_cron_job_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<CronJobItem> {
    Ok(CronJobItem {
        id: r.get(0)?,
        name: r.get(1)?,
        kind: r.get(2)?,
        schedule: r.get(3)?,
        target: r.get(4)?,
        enabled: r.get::<_, i64>(5)? != 0,
        keep_count: r.get::<_, i64>(6)?.max(1) as u32,
        last_run_at: r.get(7)?,
        last_status: r.get(8)?,
        created_at: r.get(9)?,
        updated_at: r.get(10)?,
    })
}

fn map_cron_run_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<CronRunRecord> {
    Ok(CronRunRecord {
        id: r.get(0)?,
        job_id: r.get(1)?,
        job_name: r.get(2)?,
        started_at: r.get(3)?,
        duration_ms: r.get(4)?,
        success: r.get::<_, i64>(5)? != 0,
        output: r.get(6)?,
    })
}

fn map_website_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<WebsiteRecord> {
    let kind_str: String = r.get(2)?;
    let domains_raw: String = r.get(3)?;
    let domains: Vec<String> = serde_json::from_str(&domains_raw).unwrap_or_default();
    Ok(WebsiteRecord {
        id: r.get(0)?,
        name: r.get(1)?,
        kind: WebsiteKind::from_str(&kind_str).unwrap_or(WebsiteKind::Static),
        domains,
        listen_port: r.get::<_, i64>(4)?.clamp(0, 65535) as u16,
        root_dir: r.get(5)?,
        start_cmd: r.get(6)?,
        env_vars: r.get(7)?,
        python_venv: r.get(8)?,
        proxy_target: r.get(9)?,
        websocket_enabled: r.get::<_, i64>(10)? != 0,
        spa_fallback: r.get::<_, i64>(11)? != 0,
        ssl_cert_id: r.get(12)?,
        force_https: r.get::<_, i64>(13)? != 0,
        auto_restart: r.get::<_, i64>(14)? != 0,
        custom_nginx: r.get(15)?,
        remark: r.get(16)?,
        created_at: r.get(17)?,
        updated_at: r.get(18)?,
    })
}

fn map_ssl_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SslCertificate> {
    let domains_raw: String = r.get(2)?;
    let domains: Vec<String> = serde_json::from_str(&domains_raw).unwrap_or_default();
    Ok(SslCertificate {
        id: r.get(0)?,
        name: r.get(1)?,
        domains,
        issuer: r.get(3)?,
        provider: r.get(4)?,
        challenge_type: r.get(5)?,
        dns_provider: r.get(6)?,
        auto_renew: r.get::<_, i64>(7)? != 0,
        renew_hook: r.get(8)?,
        not_before: r.get(9)?,
        not_after: r.get(10)?,
        cert_path: r.get(11)?,
        key_path: r.get(12)?,
        last_renew_status: r.get(13)?,
        created_at: r.get(14)?,
        updated_at: r.get(15)?,
    })
}
