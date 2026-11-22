use std::sync::{Arc, RwLock};
use zmpanel_core::{PanelConfig, SystemSnapshot};

use crate::auth::LoginRateLimiter;
use crate::cluster::ClusterHub;
use crate::db::Database;
use crate::websites::ProcessSupervisor;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RwLock<PanelConfig>>,
    pub db: Arc<Database>,
    pub limiter: Arc<LoginRateLimiter>,
    pub latest_snapshot: Arc<RwLock<Option<SystemSnapshot>>>,
    pub supervisor: Arc<ProcessSupervisor>,
    pub cluster: Arc<ClusterHub>,
}

impl AppState {
    pub fn new(config: PanelConfig, db: Database) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            db: Arc::new(db),
            limiter: Arc::new(LoginRateLimiter::default()),
            latest_snapshot: Arc::new(RwLock::new(None)),
            supervisor: ProcessSupervisor::new(),
            cluster: ClusterHub::new(),
        }
    }

    pub fn current_entry(&self) -> String {
        self.config
            .read()
            .expect("config lock poisoned")
            .normalized_entry()
    }

    pub fn update_entry(&self, new_entry: String) {
        let mut cfg = self.config.write().expect("config lock poisoned");
        cfg.security_entry = new_entry;
    }

    pub fn config_snapshot(&self) -> PanelConfig {
        self.config.read().expect("config lock poisoned").clone()
    }
}
