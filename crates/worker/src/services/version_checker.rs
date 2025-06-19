use crate::console::Console;
use anyhow::Result;
use chrono::{DateTime, Utc};
use directories::UserDirs;
use log::debug;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tokio::time::Duration;

const GITHUB_API_URL: &str = "https://api.github.com/repos/PrimeIntellect-ai/protocol/releases/latest";
const VERSION_CHECK_FILE: &str = ".prime_worker_version_check";
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct VersionCheckCache {
    last_check: DateTime<Utc>,
    last_notified_version: Option<String>,
}

pub struct VersionChecker {
    current_version: String,
    cache_file_path: PathBuf,
    client: Client,
}

impl VersionChecker {
    pub fn new() -> Self {
        let current_version = option_env!("WORKER_VERSION")
            .unwrap_or(env!("CARGO_PKG_VERSION"))
            .to_string();

        let cache_file_path = UserDirs::new()
            .and_then(|dirs| Some(dirs.home_dir().to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
            .join(VERSION_CHECK_FILE);

        let client = Client::builder()
            .timeout(HTTP_TIMEOUT)
            .user_agent(format!("prime-worker/{}", current_version))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            current_version,
            cache_file_path,
            client,
        }
    }

    /// Check for updates if it's been more than a day since last check
    pub async fn check_for_updates_if_needed(&self) {
        if !self.should_check_today() {
            debug!("Version check already performed today, skipping");
            return;
        }

        if let Err(e) = self.perform_version_check().await {
            debug!("Version check failed: {}", e);
        }
    }

    fn should_check_today(&self) -> bool {
        match self.load_cache() {
            Ok(cache) => {
                let now = Utc::now();
                let time_since_check = now.signed_duration_since(cache.last_check);
                time_since_check.num_hours() >= 24
            }
            Err(_) => true, // If we can't read cache, perform check
        }
    }

    async fn perform_version_check(&self) -> Result<()> {
        debug!("Checking for Prime Worker updates...");

        let latest_release = self.fetch_latest_release().await?;
        let latest_version = self.clean_version_tag(&latest_release.tag_name);

        // Update cache with current check time
        let mut cache = self.load_cache().unwrap_or_else(|_| VersionCheckCache {
            last_check: Utc::now(),
            last_notified_version: None,
        });
        cache.last_check = Utc::now();

        if self.is_newer_version(&latest_version) {
            // Only show notification if we haven't already notified about this version
            if cache.last_notified_version.as_ref() != Some(&latest_version) {
                self.show_update_notification(&latest_version, &latest_release.html_url);
                cache.last_notified_version = Some(latest_version);
            }
        } else {
            debug!("Prime Worker is up to date ({})", self.current_version);
        }

        // Save updated cache
        if let Err(e) = self.save_cache(&cache) {
            debug!("Failed to save version check cache: {}", e);
        }

        Ok(())
    }

    async fn fetch_latest_release(&self) -> Result<GitHubRelease> {
        let response = self
            .client
            .get(GITHUB_API_URL)
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "GitHub API request failed with status: {}",
                response.status()
            ));
        }

        let release: GitHubRelease = response.json().await?;
        Ok(release)
    }

    fn clean_version_tag(&self, tag: &str) -> String {
        // Remove 'v' prefix if present (e.g., "v0.2.12" -> "0.2.12")
        tag.strip_prefix('v').unwrap_or(tag).to_string()
    }

    fn is_newer_version(&self, latest_version: &str) -> bool {
        match self.compare_versions(&self.current_version, latest_version) {
            Ok(is_older) => is_older,
            Err(_) => {
                // If version comparison fails, assume we should show the notification
                // This handles cases with non-standard version formats
                debug!(
                    "Could not compare versions: current='{}', latest='{}'",
                    self.current_version, latest_version
                );
                latest_version != self.current_version
            }
        }
    }

    fn compare_versions(&self, current: &str, latest: &str) -> Result<bool> {
        // Simple semantic version comparison
        let current_parts: Vec<u32> = current
            .split('.')
            .take(3)
            .map(|s| s.parse().unwrap_or(0))
            .collect();
        
        let latest_parts: Vec<u32> = latest
            .split('.')
            .take(3)
            .map(|s| s.parse().unwrap_or(0))
            .collect();

        // Pad with zeros to ensure we have exactly 3 numbers [major, minor, patch]
        let current_parts = [
            current_parts.get(0).copied().unwrap_or(0),
            current_parts.get(1).copied().unwrap_or(0),
            current_parts.get(2).copied().unwrap_or(0),
        ];
        
        let latest_parts = [
            latest_parts.get(0).copied().unwrap_or(0),
            latest_parts.get(1).copied().unwrap_or(0),
            latest_parts.get(2).copied().unwrap_or(0),
        ];

        Ok(latest_parts > current_parts)
    }

    fn show_update_notification(&self, latest_version: &str, download_url: &str) {
        Console::section("📦 UPDATE AVAILABLE");
        Console::warning(&format!(
            "A newer version of Prime Worker is available!"
        ));
        Console::info("Current Version", &self.current_version);
        Console::info("Latest Version", latest_version);
        Console::info("Update Command", "curl -sSL https://raw.githubusercontent.com/PrimeIntellect-ai/protocol/main/crates/worker/scripts/install.sh | bash");
        Console::info("Release Notes", download_url);
        println!();
    }

    fn load_cache(&self) -> Result<VersionCheckCache> {
        let content = fs::read_to_string(&self.cache_file_path)?;
        let cache: VersionCheckCache = serde_json::from_str(&content)?;
        Ok(cache)
    }

    fn save_cache(&self, cache: &VersionCheckCache) -> Result<()> {
        let content = serde_json::to_string_pretty(cache)?;
        if let Some(parent) = self.cache_file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.cache_file_path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_comparison() {
        let checker = VersionChecker::new();

        // Test basic version comparison
        assert!(checker.compare_versions("0.2.11", "0.2.12").unwrap());
        assert!(checker.compare_versions("0.2.11", "0.3.0").unwrap());
        assert!(checker.compare_versions("1.0.0", "1.0.1").unwrap());
        
        // Test equal versions
        assert!(!checker.compare_versions("0.2.12", "0.2.12").unwrap());
        
        // Test current is newer
        assert!(!checker.compare_versions("0.2.12", "0.2.11").unwrap());
        assert!(!checker.compare_versions("1.0.0", "0.9.9").unwrap());
    }

    #[test]
    fn test_clean_version_tag() {
        let checker = VersionChecker::new();
        
        assert_eq!(checker.clean_version_tag("v0.2.12"), "0.2.12");
        assert_eq!(checker.clean_version_tag("0.2.12"), "0.2.12");
        assert_eq!(checker.clean_version_tag("v1.0.0-beta"), "1.0.0-beta");
    }

    #[tokio::test]
    async fn test_version_checker_creation() {
        let checker = VersionChecker::new();
        assert!(!checker.current_version.is_empty());
        assert!(checker.cache_file_path.to_string_lossy().contains(VERSION_CHECK_FILE));
    }
} 