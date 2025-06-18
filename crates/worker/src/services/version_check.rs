use crate::TaskHandles;
use log::{error, info, warn};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
    pub published_at: String,
    pub html_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestBinary {
    pub url: String,
    pub checksum: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub release_date: String,
    pub binaries: std::collections::HashMap<String, ManifestBinary>,
}

#[derive(Clone)]
pub struct VersionCheckService {
    client: Client,
    check_interval: Duration,
    cancellation_token: CancellationToken,
    task_handles: TaskHandles,
    current_version: String,
    github_url: String,
    gcs_url: String,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum VersionCheckError {
    #[error("HTTP request failed: {0}")]
    RequestFailed(String),
    #[error("Service initialization failed")]
    InitFailed,
    #[error("JSON parsing failed: {0}")]
    ParseFailed(String),
}

impl VersionCheckService {
    pub fn new(
        check_interval: Duration,
        cancellation_token: CancellationToken,
        task_handles: TaskHandles,
    ) -> Result<Arc<Self>, VersionCheckError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("protocol-worker-version-checker")
            .build()
            .map_err(|_| VersionCheckError::InitFailed)?;

        let current_version = option_env!("WORKER_VERSION")
            .unwrap_or(env!("CARGO_PKG_VERSION"))
            .to_string();

        let github_url = env!("VERSION_CHECK_GITHUB_URL").to_string();
        let gcs_url = env!("VERSION_CHECK_GCS_URL").to_string();

        Ok(Arc::new(Self {
            client,
            check_interval,
            cancellation_token,
            task_handles,
            current_version,
            github_url,
            gcs_url,
        }))
    }

    pub async fn start(self: Arc<Self>) -> Result<(), VersionCheckError> {
        let service = Arc::clone(&self);
        let interval_duration = self.check_interval;
        let cancellation_token = self.cancellation_token.clone();

        let handle = tokio::spawn(async move {
            let mut interval = interval(interval_duration);
            info!(
                "Version check service started - checking every {:?}",
                interval_duration
            );
            info!("Current version: {}", service.current_version);

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if let Err(e) = service.check_for_updates().await {
                            error!("Version check failed: {}", e);
                        }
                    }
                    _ = cancellation_token.cancelled() => {
                        info!("Version check service shutting down");
                        break;
                    }
                }
            }
        });

        let mut task_handles = self.task_handles.lock().await;
        task_handles.push(handle);

        Ok(())
    }

    async fn check_for_updates(&self) -> Result<(), VersionCheckError> {
        // Try dev manifest first (for beta versions)
        if self.current_version.contains("beta") {
            match self.check_dev_version().await {
                Ok(Some(latest_version)) => {
                    if self.is_newer_version(&latest_version) {
                        self.log_update_available(&latest_version, true);
                    }
                    return Ok(());
                }
                Ok(None) => {
                    // No update available in dev, continue to check prod
                }
                Err(e) => {
                    warn!("Dev version check failed, trying production: {}", e);
                }
            }
        }

        // Check production releases
        match self.check_prod_version().await {
            Ok(Some(latest_version)) => {
                if self.is_newer_version(&latest_version) {
                    self.log_update_available(&latest_version, false);
                }
            }
            Ok(None) => {
                // No update available
            }
            Err(e) => {
                error!("Production version check failed: {}", e);
            }
        }

        Ok(())
    }

    async fn check_dev_version(&self) -> Result<Option<String>, VersionCheckError> {
        let response = self
            .client
            .get(&self.gcs_url)
            .send()
            .await
            .map_err(|e| VersionCheckError::RequestFailed(e.to_string()))?;

        if !response.status().is_success() {
            return Err(VersionCheckError::RequestFailed(format!(
                "HTTP {}",
                response.status()
            )));
        }

        let manifest: Manifest = response
            .json()
            .await
            .map_err(|e| VersionCheckError::ParseFailed(e.to_string()))?;

        Ok(Some(manifest.version))
    }

    async fn check_prod_version(&self) -> Result<Option<String>, VersionCheckError> {
        let response = self
            .client
            .get(&self.github_url)
            .send()
            .await
            .map_err(|e| VersionCheckError::RequestFailed(e.to_string()))?;

        if !response.status().is_success() {
            return Err(VersionCheckError::RequestFailed(format!(
                "HTTP {}",
                response.status()
            )));
        }

        let release: GitHubRelease = response
            .json()
            .await
            .map_err(|e| VersionCheckError::ParseFailed(e.to_string()))?;

        Ok(Some(release.tag_name))
    }

    fn is_newer_version(&self, latest_version: &str) -> bool {
        let current = self.normalize_version(&self.current_version);
        let latest = self.normalize_version(latest_version);

        // Simple string comparison for version ordering
        // This works for semver-like versions: v0.1.0, v0.1.0-beta.1, etc.
        latest > current
    }

    fn normalize_version(&self, version: &str) -> String {
        // Remove 'v' prefix if present
        version.strip_prefix('v').unwrap_or(version).to_string()
    }

    fn log_update_available(&self, latest_version: &str, is_dev: bool) {
        let source = if is_dev { "development" } else { "production" };
        warn!(
            "🔄 New {} version available: {} (current: {})",
            source, latest_version, self.current_version
        );

        info!("💡 To update, run: prime-worker update");
        info!("   Then restart your worker to use the new version");
    }
}
