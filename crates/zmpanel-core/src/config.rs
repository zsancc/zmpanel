use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunMode {
    Standalone,
    Master,
    Agent,
}

impl Default for RunMode {
    fn default() -> Self {
        Self::Standalone
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelConfig {
    pub mode: RunMode,
    pub bind_host: String,
    pub port: u16,
    pub data_dir: PathBuf,
    pub security_entry: String,
    pub master_url: Option<String>,
    pub node_token: Option<String>,
    pub session_ttl_hours: i64,
    pub max_login_attempts: u32,
    pub lock_minutes: i64,
}

impl PanelConfig {
    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("zmpanel.db")
    }

    pub fn trash_dir(&self) -> PathBuf {
        self.data_dir.join("trash")
    }

    pub fn nginx_dir(&self) -> PathBuf {
        self.data_dir.join("nginx")
    }

    pub fn nginx_sites_dir(&self) -> PathBuf {
        self.nginx_dir().join("sites")
    }

    pub fn ssl_dir(&self) -> PathBuf {
        self.data_dir.join("ssl")
    }

    pub fn backups_dir(&self) -> PathBuf {
        self.data_dir.join("backups")
    }

    pub fn docker_projects_dir(&self) -> PathBuf {
        self.data_dir.join("docker-projects")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    pub fn store_dir(&self) -> PathBuf {
        self.data_dir.join("store")
    }

    pub fn master_backups_dir(&self) -> PathBuf {
        self.data_dir.join("master-backups")
    }

    pub fn updates_dir(&self) -> PathBuf {
        self.data_dir.join("updates")
    }

    pub fn normalized_entry(&self) -> String {
        let trimmed = self.security_entry.trim().trim_matches('/');
        if trimmed.is_empty() {
            String::new()
        } else {
            format!("/{}", trimmed)
        }
    }
}
