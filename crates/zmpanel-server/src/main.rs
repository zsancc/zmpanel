use anyhow::Result;
use clap::{Parser, ValueEnum};
use std::net::SocketAddr;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;
use zmpanel_core::{PanelConfig, RunMode};
use zmpanel_server::{auth, cluster, cron_jobs, db::Database, monitor, routes, security, state::AppState};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliMode {
    Standalone,
    Master,
    Agent,
}

impl From<CliMode> for RunMode {
    fn from(value: CliMode) -> Self {
        match value {
            CliMode::Standalone => RunMode::Standalone,
            CliMode::Master => RunMode::Master,
            CliMode::Agent => RunMode::Agent,
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "zmpanel", version, about = "轻量、安全、无外联的现代化 Linux 运维面板")]
struct Cli {
    /// 运行模式: standalone / master / agent
    #[arg(long, value_enum, default_value = "standalone", env = "ZMPANEL_MODE")]
    mode: CliMode,

    /// 监听地址（安全建议：搭配反代时可改为 127.0.0.1）
    #[arg(long, default_value = "0.0.0.0", env = "ZMPANEL_HOST")]
    host: String,

    /// 监听端口
    #[arg(long, default_value_t = 18080, env = "ZMPANEL_PORT")]
    port: u16,

    /// 数据与配置存储目录
    #[arg(long, default_value = "./zmpanel-data", env = "ZMPANEL_DATA_DIR")]
    data_dir: PathBuf,

    /// 显式指定安全入口路径（留空则读取数据库或首次自动随机生成）
    #[arg(long, env = "ZMPANEL_ENTRY")]
    entry: Option<String>,

    /// 首次初始化管理员用户名
    #[arg(long, default_value = "admin", env = "ZMPANEL_ADMIN_USER")]
    admin_user: String,

    /// 首次初始化管理员密码（留空则自动生成强随机密码并打印）
    #[arg(long, env = "ZMPANEL_ADMIN_PASS")]
    admin_pass: Option<String>,

    /// Agent 模式下连接的主控地址（阶段 4 启用）
    #[arg(long, env = "ZMPANEL_MASTER_URL")]
    master_url: Option<String>,

    /// Agent 节点认证令牌（阶段 4 启用）
    #[arg(long, env = "ZMPANEL_NODE_TOKEN")]
    node_token: Option<String>,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let run_mode: RunMode = cli.mode.into();
    std::fs::create_dir_all(&cli.data_dir)?;

    let db_path = cli.data_dir.join("zmpanel.db");
    let db = Database::open(&db_path)?;

    // Build a bounded Tokio runtime tailored for low-RAM NAT VPS (even on 32/64-core container hosts):
    // - Agent mode uses a single current-thread reactor + small blocking pool, avoiding per-core
    //   thread stacks and glibc malloc arenas, and skips admin Argon2id bootstrapping completely.
    // - Master/Standalone uses 2 reactor workers, more than enough for personal ops panels.
    let runtime = if run_mode == RunMode::Agent {
        tokio::runtime::Builder::new_current_thread()
            .max_blocking_threads(4)
            .enable_all()
            .build()?
    } else {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .max_blocking_threads(8)
            .enable_all()
            .build()?
    };

    runtime.block_on(async_main(cli, run_mode, db))
}

async fn async_main(cli: Cli, run_mode: RunMode, db: Database) -> Result<()> {
    if run_mode == RunMode::Agent {
        let config = PanelConfig {
            mode: RunMode::Agent,
            bind_host: cli.host,
            port: cli.port,
            data_dir: cli.data_dir.clone(),
            security_entry: String::new(),
            master_url: cli.master_url,
            node_token: cli.node_token,
            session_ttl_hours: 72,
            max_login_attempts: 8,
            lock_minutes: 15,
        };
        println!("============================================================");
        println!("  zmpanel Agent v{} 启动（极简无前端模式，主动反向连接 Master）", env!("CARGO_PKG_VERSION"));
        println!("  数据目录 : {}", cli.data_dir.display());
        println!("  主控地址 : {}", config.master_url.as_deref().unwrap_or("(未配置 --master-url)"));
        println!("============================================================");
        let db_arc = std::sync::Arc::new(db);
        cluster::run_agent_loop(config, db_arc).await;
        return Ok(());
    }

    // Resolve or initialize security entry
    let security_entry = if let Some(explicit) = cli.entry {
        let trimmed = explicit.trim().trim_matches('/');
        let normalized = if trimmed.is_empty() {
            String::new()
        } else {
            format!("/{}", trimmed)
        };
        db.set_setting("security_entry", &normalized)?;
        normalized
    } else if let Some(saved) = db.get_setting("security_entry")? {
        saved
    } else {
        let generated = format!("/zm_{}", auth::generate_alnum(8));
        db.set_setting("security_entry", &generated)?;
        generated
    };

    // Bootstrap default admin account on first launch
    let mut initial_password_notice: Option<(String, String)> = None;
    if db.user_count()? == 0 {
        let raw_pass = cli
            .admin_pass
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| auth::generate_alnum(14));
        let hash = auth::hash_password(&raw_pass)?;
        db.create_user(&cli.admin_user, &hash)?;
        initial_password_notice = Some((cli.admin_user.clone(), raw_pass));
    }

    let config = PanelConfig {
        mode: run_mode,
        bind_host: cli.host.clone(),
        port: cli.port,
        data_dir: cli.data_dir.clone(),
        security_entry: security_entry.clone(),
        master_url: cli.master_url,
        node_token: cli.node_token,
        session_ttl_hours: 72,
        max_login_attempts: 8,
        lock_minutes: 15,
    };

    let state = AppState::new(config, db);
    monitor::spawn_monitor_task(state.clone());
    security::spawn_security_engine(state.clone());
    cron_jobs::spawn_cron_scheduler(state.clone());

    let app = routes::build_app(state);
    let addr: SocketAddr = format!("{}:{}", cli.host, cli.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;

    let display_host = if cli.host == "0.0.0.0" {
        "127.0.0.1"
    } else {
        &cli.host
    };
    println!("============================================================");
    println!("  zmpanel v{} 已启动 ({:?})", env!("CARGO_PKG_VERSION"), cli.mode);
    println!("  访问地址 : http://{}:{}{}", display_host, cli.port, security_entry);
    println!("  数据目录 : {}", cli.data_dir.display());
    if let Some((user, pass)) = initial_password_notice {
        println!("  初始账号 : {}", user);
        println!("  初始密码 : {}  (请登录后尽快修改并开启 2FA)", pass);
    }
    println!("  安全提示 : 访问非安全入口路径将直接返回 404 伪装响应");
    println!("============================================================");

    axum::serve(listener, app).await?;
    Ok(())
}
