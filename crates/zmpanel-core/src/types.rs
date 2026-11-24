use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeEnvInfo {
    /// baremetal | kvm | vmware | hyperv | virtualbox | xen | wsl | docker | podman | lxc | openvz | kubernetes | windows | unknown
    pub virt_type: String,
    /// 人类可读环境标签，如 "Docker 容器 (NAT Docker 鸡)"、"LXC 容器"、"KVM 虚拟机"
    pub virt_label: String,
    /// 当前系统本身是否运行在容器内（Docker / Podman / Kubernetes / LXC / OpenVZ）
    pub inside_container: bool,
    /// 当前环境是否允许运行/嵌套 Docker
    pub docker_nesting_supported: bool,
    /// native | dood | dind | unsupported
    pub docker_mode: String,
    /// 面向用户的清晰中文说明与建议
    pub docker_nesting_reason: String,
}

impl Default for RuntimeEnvInfo {
    fn default() -> Self {
        Self {
            virt_type: "unknown".to_string(),
            virt_label: "未知环境".to_string(),
            inside_container: false,
            docker_nesting_supported: true,
            docker_mode: "native".to_string(),
            docker_nesting_reason: "常规环境，支持运行 Docker".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemSnapshot {
    pub timestamp: i64,
    pub hostname: String,
    pub os_name: String,
    pub os_version: String,
    pub kernel_version: String,
    pub arch: String,
    pub uptime_secs: u64,
    pub load_avg: [f64; 3],
    pub cpu_brand: String,
    pub cpu_cores: usize,
    pub cpu_usage: f32,
    pub mem_total: u64,
    pub mem_used: u64,
    pub mem_available: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub net_rx_rate: u64,
    pub net_tx_rate: u64,
    pub net_rx_month: u64,
    pub net_tx_month: u64,
    pub disks: Vec<DiskInfo>,
    pub panel_rss_bytes: u64,
    #[serde(default)]
    pub runtime_env: RuntimeEnvInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskInfo {
    pub name: String,
    pub mount_point: String,
    pub fs_type: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricPoint {
    pub timestamp: i64,
    pub cpu_usage: f32,
    pub mem_used_pct: f32,
    pub swap_used_pct: f32,
    pub load_1m: f64,
    pub net_rx_rate: u64,
    pub net_tx_rate: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessItem {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub memory_bytes: u64,
    pub status: String,
    pub cmd: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: i64,
    pub permissions: String,
    pub mode_octal: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrashItem {
    pub id: String,
    pub original_path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub deleted_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSnippet {
    pub id: String,
    pub name: String,
    pub category: String,
    pub command: String,
    pub description: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLogItem {
    pub id: i64,
    pub timestamp: i64,
    pub actor: String,
    pub ip: String,
    pub action: String,
    pub target: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSummary {
    pub id: String,
    pub name: String,
    pub mode: String,
    pub status: String,
    pub os_name: String,
    pub arch: String,
    pub cpu_usage: f32,
    pub mem_used_pct: f32,
    pub last_seen: i64,
    pub is_local: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub username: String,
    pub totp_enabled: bool,
    pub security_entry: String,
    pub version: String,
    pub mode: String,
}

// ---------- Stage 2: Websites ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WebsiteKind {
    Rust,
    Go,
    Python,
    Proxy,
    Static,
}

impl WebsiteKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Go => "go",
            Self::Python => "python",
            Self::Proxy => "proxy",
            Self::Static => "static",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "rust" => Some(Self::Rust),
            "go" => Some(Self::Go),
            "python" => Some(Self::Python),
            "proxy" => Some(Self::Proxy),
            "static" => Some(Self::Static),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebsiteRecord {
    pub id: String,
    pub name: String,
    pub kind: WebsiteKind,
    pub domains: Vec<String>,
    pub listen_port: u16,
    pub root_dir: String,
    pub start_cmd: String,
    pub env_vars: String,
    pub python_venv: Option<String>,
    pub proxy_target: Option<String>,
    pub websocket_enabled: bool,
    pub spa_fallback: bool,
    pub ssl_cert_id: Option<String>,
    pub force_https: bool,
    pub auto_restart: bool,
    pub custom_nginx: Option<String>,
    pub remark: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebsiteWithRuntime {
    #[serde(flatten)]
    pub site: WebsiteRecord,
    pub runtime_status: String,
    pub pid: Option<u32>,
    pub restart_count: u32,
    pub nginx_conf_path: String,
}

// ---------- Stage 2: Docker Workspace ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerProject {
    pub id: String,
    pub name: String,
    pub work_dir: String,
    pub compose_file: String,
    pub deploy_cmd: String,
    pub description: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerContainerInfo {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub ports: String,
    pub created: String,
    pub cpu_pct: String,
    pub mem_usage: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerImageInfo {
    pub id: String,
    pub repository: String,
    pub tag: String,
    pub size: String,
    pub created: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerStatusResponse {
    pub available: bool,
    pub version: String,
    pub compose_available: bool,
    pub message: String,
    #[serde(default)]
    pub env: RuntimeEnvInfo,
}

// ---------- Stage 2: SSL Certificates ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SslCertificate {
    pub id: String,
    pub name: String,
    pub domains: Vec<String>,
    pub issuer: String,
    pub provider: String, // manual / self_signed / letsencrypt / zerossl
    pub challenge_type: String, // none / http01 / dns01
    pub dns_provider: Option<String>,
    pub auto_renew: bool,
    pub renew_hook: String,
    pub not_before: i64,
    pub not_after: i64,
    pub cert_path: String,
    pub key_path: String,
    pub last_renew_status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

// ---------- Stage 2: Software Store, Nginx & Python uv ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreAppItem {
    pub id: String,
    pub name: String,
    pub category: String,
    pub description: String,
    pub min_mem_mb: u64,
    pub supported_os: Vec<String>,
    pub install_modes: Vec<String>,
    pub installed: bool,
    pub running: bool,
    pub version: String,
    pub has_manager: bool,
    pub compat_warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NginxSecurityConfig {
    pub hide_version: bool,
    pub gzip_enabled: bool,
    pub client_max_body_mb: u32,
    pub rate_limit_enabled: bool,
    pub rate_limit_rps: u32,
    pub rate_limit_burst: u32,
    pub block_sensitive_paths: bool,
    pub block_bad_ua: bool,
    pub blocked_ips: Vec<String>,
    pub custom_rules: String,
}

impl Default for NginxSecurityConfig {
    fn default() -> Self {
        Self {
            hide_version: true,
            gzip_enabled: true,
            client_max_body_mb: 50,
            rate_limit_enabled: true,
            rate_limit_rps: 20,
            rate_limit_burst: 40,
            block_sensitive_paths: true,
            block_bad_ua: true,
            blocked_ips: vec![],
            custom_rules: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PythonVenvItem {
    pub id: String,
    pub name: String,
    pub path: String,
    pub python_version: String,
    pub index_url: String,
    pub created_at: i64,
}

// ---------- Stage 2: Backup & Migration ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupItem {
    pub id: String,
    pub name: String,
    pub file_path: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub target_repo: String, // local / master
    pub scopes: Vec<String>,
    pub summary: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    pub format_version: u32,
    pub created_at: i64,
    pub source_hostname: String,
    pub panel_version: String,
    pub scopes: Vec<String>,
    pub website_ids: Vec<String>,
    pub docker_project_ids: Vec<String>,
    pub ssl_cert_ids: Vec<String>,
    pub custom_dirs: Vec<String>,
    pub include_panel_db: bool,
}

// ---------- Stage 3: Firewall ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirewallStatus {
    pub backend: String, // nftables / ufw / iptables / unsupported
    pub active: bool,
    pub protected_ports: Vec<u16>,
    pub ssh_port: u16,
    pub panel_port: u16,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirewallRule {
    pub id: String,
    pub port: String,     // e.g. "80", "443", "8000-8010"
    pub protocol: String, // tcp / udp / both
    pub action: String,   // allow / deny
    pub source: String,   // any or IP/CIDR
    pub remark: String,
    pub enabled: bool,
    pub created_at: i64,
}

// ---------- Stage 3: Security Center (Built-in Lightweight IPS) ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    pub enabled: bool,
    pub ssh_guard_enabled: bool,
    pub ssh_max_failures: u32,
    pub ssh_window_secs: i64,
    pub port_scan_enabled: bool,
    pub port_scan_threshold: u32,
    pub web_guard_enabled: bool,
    pub web_scan_threshold: u32,
    pub ban_duration_secs: i64,
    pub nginx_log_path: String,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ssh_guard_enabled: true,
            ssh_max_failures: 5,
            ssh_window_secs: 600,
            port_scan_enabled: true,
            port_scan_threshold: 8,
            web_guard_enabled: true,
            web_scan_threshold: 12,
            ban_duration_secs: 3600,
            nginx_log_path: "/var/log/nginx/access.log".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityBanItem {
    pub ip: String,
    pub reason: String,
    pub category: String, // ssh / port_scan / web / manual
    pub hits: u32,
    pub banned_at: i64,
    pub expires_at: i64, // 0 = permanent
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityEventItem {
    pub id: i64,
    pub timestamp: i64,
    pub category: String,
    pub source_ip: String,
    pub summary: String,
    pub action_taken: String,
    pub sample: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityWhitelistItem {
    pub ip: String,
    pub remark: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshHardeningStatus {
    pub config_path: String,
    pub readable: bool,
    pub port: u16,
    pub password_auth: bool,
    pub permit_root_login: String, // yes / prohibit-password / no
    pub pubkey_auth: bool,
}

// ---------- Stage 3: Log Center ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogSourceItem {
    pub id: String,
    pub name: String,
    pub category: String, // system / ssh / nginx / site / panel
    pub path: String,
    pub exists: bool,
    pub size_bytes: u64,
}

// ---------- Stage 3: Cron Jobs ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJobItem {
    pub id: String,
    pub name: String,
    pub kind: String,     // shell / backup / http / cleanup / site_restart
    pub schedule: String, // 5-field cron expression
    pub target: String,   // command / url / site_id / backup JSON config
    pub enabled: bool,
    pub keep_count: u32,
    pub last_run_at: i64,
    pub last_status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronRunRecord {
    pub id: i64,
    pub job_id: String,
    pub job_name: String,
    pub started_at: i64,
    pub duration_ms: i64,
    pub success: bool,
    pub output: String,
}

// ---------- Stage 3: Store Database/Cache Service Config ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisServiceConfig {
    pub bind: String,
    pub port: u16,
    pub password: String,
    pub maxmemory_mb: u32,
    pub maxmemory_policy: String,
    pub appendonly: bool,
}

impl Default for RedisServiceConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".to_string(),
            port: 6379,
            password: String::new(),
            maxmemory_mb: 64,
            maxmemory_policy: "allkeys-lru".to_string(),
            appendonly: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostgresServiceConfig {
    pub bind: String,
    pub port: u16,
    pub superuser: String,
    pub password: String,
    pub default_db: String,
    pub shared_buffers_mb: u32,
    pub max_connections: u32,
}

impl Default for PostgresServiceConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".to_string(),
            port: 5432,
            superuser: "postgres".to_string(),
            password: String::new(),
            default_db: "appdb".to_string(),
            shared_buffers_mb: 32,
            max_connections: 40,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MongoServiceConfig {
    pub bind: String,
    pub port: u16,
    pub root_user: String,
    pub root_password: String,
    pub cache_size_gb: f32,
    pub image_tag: String, // e.g. "4.4" (no AVX required) or "7"
}

impl Default for MongoServiceConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".to_string(),
            port: 27017,
            root_user: "admin".to_string(),
            root_password: String::new(),
            cache_size_gb: 0.25,
            image_tag: "4.4".to_string(),
        }
    }
}

// ---------- Stage 4: Master / Agent Cluster, Central Backup & Self-Update ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterNodeRecord {
    pub id: String,
    pub name: String,
    pub group_name: String,
    pub hostname: String,
    pub os_name: String,
    pub arch: String,
    pub agent_version: String,
    pub status: String, // online / offline
    pub cpu_usage: f32,
    pub mem_used_pct: f32,
    pub mem_total_bytes: u64,
    pub agent_rss_bytes: u64,
    pub net_rx_month: u64,
    pub net_tx_month: u64,
    pub last_seen: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterJobResult {
    pub node_id: String,
    pub node_name: String,
    pub success: bool,
    pub duration_ms: i64,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterJobRecord {
    pub id: String,
    pub action: String,
    pub payload_summary: String,
    pub created_at: i64,
    pub results: Vec<ClusterJobResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MasterBackupRecord {
    pub id: String,
    pub node_id: String,
    pub node_name: String,
    pub name: String,
    pub file_path: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub summary: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub version: String,
    pub changelog: String,
    pub download_url: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyConfig {
    pub enabled: bool,
    pub webhook_url: String,
    pub telegram_bot_token: String,
    pub telegram_chat_id: String,
    pub notify_on_ban: bool,
    pub notify_on_node_offline: bool,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            webhook_url: String::new(),
            telegram_bot_token: String::new(),
            telegram_chat_id: String::new(),
            notify_on_ban: true,
            notify_on_node_offline: true,
        }
    }
}

/// Bidirectional WebSocket RPC wire protocol between Agent (outbound) and Master.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentToMasterMsg {
    Hello {
        node_id: String,
        hostname: String,
        os_name: String,
        arch: String,
        version: String,
        mem_total_bytes: u64,
    },
    Heartbeat {
        node_id: String,
        cpu_usage: f32,
        mem_used_pct: f32,
        mem_total_bytes: u64,
        agent_rss_bytes: u64,
        net_rx_month: u64,
        net_tx_month: u64,
    },
    TaskResult {
        request_id: String,
        success: bool,
        output: String,
        /// Optional base64-encoded backup archive payload when answering PullBackup
        archive_b64: Option<String>,
        archive_sha256: Option<String>,
        archive_name: Option<String>,
        archive_summary: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MasterToAgentMsg {
    Ping,
    ExecCommand {
        request_id: String,
        command: String,
    },
    UpdateSoftware {
        request_id: String,
        package: String, // nginx / redis / postgresql / system-security
    },
    PullBackup {
        request_id: String,
        name: Option<String>,
        include_websites: bool,
        include_docker: bool,
        include_ssl: bool,
        include_panel_db: bool,
        custom_dirs: Vec<String>,
    },
    PushRestore {
        request_id: String,
        archive_name: String,
        archive_sha256: String,
        archive_b64: String,
        restore_dir: Option<String>,
    },
    SelfUpdate {
        request_id: String,
        download_url: String,
        sha256: String,
        target_version: String,
    },
}

// ---------- Vulnerability Scan & Security Baseline ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnPackageItem {
    pub package: String,
    /// kernel | privilege | ssh | web | runtime | container | database | system
    pub category: String,
    /// critical | high | medium | low
    pub severity: String,
    pub current_version: String,
    pub candidate_version: String,
    pub is_security_repo: bool,
    pub advisory: String,
    #[serde(default)]
    pub cves: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityBaselineCheck {
    pub id: String,
    /// kernel | privilege | ssh | web | runtime | network
    pub category: String,
    pub title: String,
    /// critical | high | medium | info | pass
    pub severity: String,
    /// risk | warn | pass
    pub status: String,
    pub detail: String,
    pub remediation: String,
    #[serde(default)]
    pub cves: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnWatchConfig {
    pub enabled: bool,
    pub interval_hours: u32,
    /// "high" (default: alert on critical + high, suppress medium/low) | "critical"
    pub alert_min_severity: String,
    pub auto_fix_non_kernel: bool,
    #[serde(default)]
    pub last_scan_at: i64,
    #[serde(default)]
    pub last_alert_at: i64,
    #[serde(default)]
    pub last_summary: String,
}

impl Default for VulnWatchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_hours: 6,
            alert_min_severity: "high".to_string(),
            auto_fix_non_kernel: false,
            last_scan_at: 0,
            last_alert_at: 0,
            last_summary: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnScanReport {
    pub scanned_at: i64,
    pub os_family: String,
    pub kernel_running: String,
    pub reboot_required: bool,
    pub security_score: u8,
    pub critical_count: usize,
    pub high_count: usize,
    pub total_upgradable: usize,
    pub packages: Vec<VulnPackageItem>,
    pub baselines: Vec<SecurityBaselineCheck>,
    pub summary: String,
}
