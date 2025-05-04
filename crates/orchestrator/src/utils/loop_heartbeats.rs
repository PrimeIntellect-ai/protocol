use serde::Serialize;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ServerMode;

#[derive(Serialize)]
pub struct HealthStatus {
    pub healthy: bool,
    pub inviter_last_run_seconds_ago: i64,
    pub monitor_last_run_seconds_ago: i64,
    pub status_updater_last_run_seconds_ago: i64,
}

pub struct LoopHeartbeats {
    last_inviter_iteration: Arc<AtomicI64>,
    last_monitor_iteration: Arc<AtomicI64>,
    last_status_updater_iteration: Arc<AtomicI64>,
    server_mode: ServerMode,
}

impl LoopHeartbeats {
    pub fn new(server_mode: &ServerMode) -> Self {
        Self {
            last_inviter_iteration: Arc::new(AtomicI64::new(-1)),
            last_monitor_iteration: Arc::new(AtomicI64::new(-1)),
            last_status_updater_iteration: Arc::new(AtomicI64::new(-1)),
            server_mode: *server_mode,
        }
    }

    pub fn update_inviter(&self) {
        self.last_inviter_iteration.store(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
            Ordering::SeqCst,
        );
    }

    pub fn update_monitor(&self) {
        self.last_monitor_iteration.store(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
            Ordering::SeqCst,
        );
    }

    pub fn update_status_updater(&self) {
        self.last_status_updater_iteration.store(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
            Ordering::SeqCst,
        );
    }

    pub fn health_status(&self) -> HealthStatus {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let _two_minutes = 120;

        let inviter_last = self.last_inviter_iteration.load(Ordering::SeqCst);
        let monitor_last = self.last_monitor_iteration.load(Ordering::SeqCst);
        let status_updater_last = self.last_status_updater_iteration.load(Ordering::SeqCst);

        // Calculate seconds ago for each operation
        let inviter_seconds_ago = if inviter_last > 0 {
            now - inviter_last
        } else {
            -1
        };
        let monitor_seconds_ago = if monitor_last > 0 {
            now - monitor_last
        } else {
            -1
        };
        let status_updater_seconds_ago = if status_updater_last > 0 {
            now - status_updater_last
        } else {
            -1
        };

        let processes_healthy = inviter_seconds_ago < _two_minutes
            && inviter_seconds_ago != -1
            && monitor_seconds_ago < _two_minutes
            && monitor_seconds_ago != -1
            && status_updater_seconds_ago < _two_minutes
            && status_updater_seconds_ago != -1;

        let healthy = match self.server_mode {
            // TODO: in the future we might want to check if the redis connection is healthy
            ServerMode::ApiOnly => true,
            _ => processes_healthy,
        };

        HealthStatus {
            healthy,
            inviter_last_run_seconds_ago: inviter_seconds_ago,
            monitor_last_run_seconds_ago: monitor_seconds_ago,
            status_updater_last_run_seconds_ago: status_updater_seconds_ago,
        }
    }
}
