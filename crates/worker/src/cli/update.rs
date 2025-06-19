use crate::console::Console;
use log::{error, info, warn};
use reqwest::Client;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
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
    pub binaries: std::collections::HashMap<String, ManifestBinary>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum UpdateError {
    #[error("HTTP request failed: {0}")]
    RequestFailed(String),
    #[error("File system error: {0}")]
    FileSystemError(String),
    #[error("JSON parsing failed: {0}")]
    ParseFailed(String),
    #[error("No update available")]
    NoUpdateAvailable,
}

pub struct UpdateService {
    client: Client,
    current_version: String,
    github_url: String,
    gcs_url: String,
}

impl UpdateService {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("protocol-worker-updater")
            .build()
            .unwrap();

        let current_version = option_env!("WORKER_VERSION")
            .unwrap_or(env!("CARGO_PKG_VERSION"))
            .to_string();

        let github_url = env!("VERSION_CHECK_GITHUB_URL").to_string();
        let gcs_url = env!("VERSION_CHECK_GCS_URL").to_string();

        Self {
            client,
            current_version,
            github_url,
            gcs_url,
        }
    }

    pub async fn update(&self) -> Result<(), UpdateError> {
        Console::section("Checking for updates");
        info!("Current version: {}", self.current_version);

        let (latest_version, download_url) = self.get_latest_version().await?;

        if !self.is_newer_version(&latest_version) {
            Console::info(
                "No update needed",
                &format!("Already running latest version: {}", self.current_version),
            );
            return Err(UpdateError::NoUpdateAvailable);
        }

        Console::info(
            "Update available",
            &format!(
                "Updating from {} to {}",
                self.current_version, latest_version
            ),
        );

        let binary_path = self.get_current_binary_path()?;
        let temp_path = format!("{}.tmp", binary_path);

        Console::info(
            "Downloading",
            &format!("Fetching {} from {}", latest_version, download_url),
        );
        let binary_data = self.download_binary(&download_url).await?;

        Console::info("Installing", "Writing new binary");
        fs::write(&temp_path, binary_data)
            .map_err(|e| UpdateError::FileSystemError(e.to_string()))?;

        fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o755))
            .map_err(|e| UpdateError::FileSystemError(e.to_string()))?;

        fs::rename(&temp_path, &binary_path)
            .map_err(|e| UpdateError::FileSystemError(e.to_string()))?;

        Console::success(&format!("Successfully updated to {}", latest_version));
        Console::info("Next step", "Restart your worker to use the new version");

        Ok(())
    }

    async fn get_latest_version(&self) -> Result<(String, String), UpdateError> {
        if self.current_version.contains("beta") {
            match self.get_dev_version().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    warn!("Dev version check failed, trying production: {}", e);
                }
            }
        }

        self.get_prod_version().await
    }

    async fn get_dev_version(&self) -> Result<(String, String), UpdateError> {
        let response = self
            .client
            .get(&self.gcs_url)
            .send()
            .await
            .map_err(|e| UpdateError::RequestFailed(e.to_string()))?;

        if !response.status().is_success() {
            return Err(UpdateError::RequestFailed(format!(
                "HTTP {}",
                response.status()
            )));
        }

        let manifest: Manifest = response
            .json()
            .await
            .map_err(|e| UpdateError::ParseFailed(e.to_string()))?;

        let worker_binary = manifest.binaries.get("worker").ok_or_else(|| {
            UpdateError::RequestFailed("Worker binary not found in manifest".to_string())
        })?;

        Ok((manifest.version, worker_binary.url.clone()))
    }

    async fn get_prod_version(&self) -> Result<(String, String), UpdateError> {
        let response = self
            .client
            .get(&self.github_url)
            .send()
            .await
            .map_err(|e| UpdateError::RequestFailed(e.to_string()))?;

        if !response.status().is_success() {
            return Err(UpdateError::RequestFailed(format!(
                "HTTP {}",
                response.status()
            )));
        }

        let release: GitHubRelease = response
            .json()
            .await
            .map_err(|e| UpdateError::ParseFailed(e.to_string()))?;

        let download_url = format!(
            "https://github.com/PrimeProtocol/protocol/releases/download/{}/worker-linux-x86_64",
            release.tag_name
        );

        Ok((release.tag_name, download_url))
    }

    async fn download_binary(&self, url: &str) -> Result<Vec<u8>, UpdateError> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| UpdateError::RequestFailed(e.to_string()))?;

        if !response.status().is_success() {
            return Err(UpdateError::RequestFailed(format!(
                "HTTP {}",
                response.status()
            )));
        }

        response
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| UpdateError::RequestFailed(e.to_string()))
    }

    fn get_current_binary_path(&self) -> Result<String, UpdateError> {
        let current_exe =
            std::env::current_exe().map_err(|e| UpdateError::FileSystemError(e.to_string()))?;

        current_exe
            .to_str()
            .ok_or_else(|| UpdateError::FileSystemError("Invalid binary path".to_string()))
            .map(|s| s.to_string())
    }

    fn is_newer_version(&self, latest_version: &str) -> bool {
        let current_normalized = self.normalize_version(&self.current_version);
        let latest_normalized = self.normalize_version(latest_version);

        match (
            Version::parse(&current_normalized),
            Version::parse(&latest_normalized),
        ) {
            (Ok(current_ver), Ok(latest_ver)) => latest_ver > current_ver,
            _ => {
                warn!("Failed to parse versions as semver, falling back to string comparison");
                latest_normalized > current_normalized
            }
        }
    }

    fn normalize_version(&self, version: &str) -> String {
        version.strip_prefix('v').unwrap_or(version).to_string()
    }
}
