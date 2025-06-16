use super::storage;
use crate::{
    checks::issue::{IssueReport, IssueType},
    console::Console,
};
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct StoragePathDetector {
    issues: Arc<RwLock<IssueReport>>,
}

impl StoragePathDetector {
    pub fn new(issues: Arc<RwLock<IssueReport>>) -> Self {
        Self { issues }
    }

    pub async fn detect_storage_path(
        &self,
        storage_path_override: Option<String>,
    ) -> Result<(String, Option<u64>), Box<dyn std::error::Error>> {
        if let Some(override_path) = storage_path_override {
            self.validate_override_path(&override_path)?;
            let available_space = if cfg!(target_os = "linux") {
                storage::get_available_space(&override_path)
            } else {
                None
            };
            Ok((override_path, available_space))
        } else if cfg!(target_os = "linux") {
            self.detect_linux_storage_path().await
        } else {
            self.detect_cross_platform_storage_path().await
        }
    }

    fn validate_override_path(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        if !std::path::Path::new(path).exists() {
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Storage path override does not exist: {}", path),
            )));
        }

        if let Err(e) = std::fs::metadata(path) {
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("Storage path override is not accessible: {} ({})", path, e),
            )));
        }

        Ok(())
    }

    async fn detect_linux_storage_path(
        &self,
    ) -> Result<(String, Option<u64>), Box<dyn std::error::Error>> {
        // First try automatic storage detection
        if let Some(mount_point) = storage::find_largest_storage() {
            return Ok((mount_point.path, Some(mount_point.available_space)));
        }

        // Try fallback paths
        let fallback_paths = vec![
            "/var/lib/prime-worker".to_string(),
            "/opt/prime-worker".to_string(),
            "/home/prime-worker".to_string(),
        ];

        // Add user home directory option
        let mut all_paths = fallback_paths;
        if let Ok(home) = std::env::var("HOME") {
            all_paths.push(format!("{}/prime-worker", home));
        }

        for path in all_paths {
            if std::path::Path::new(&path)
                .parent()
                .is_some_and(|p| p.exists())
            {
                Console::warning(&format!(
                    "No suitable storage mount found, using fallback path: {}",
                    path
                ));
                let issue_tracker = self.issues.write().await;
                issue_tracker.add_issue(
                    IssueType::NoStoragePath,
                    "No suitable storage mount found, using fallback path",
                );
                // Get available space for the fallback path
                let available_space = storage::get_available_space(&path);
                return Ok((path, available_space));
            }
        }

        // Last resort - current directory
        let current_dir = std::env::current_dir()
            .map(|p| p.join("prime-worker-data").to_string_lossy().to_string())
            .unwrap_or_else(|_| "./prime-worker-data".to_string());

        Console::warning(&format!(
            "Using current directory fallback: {}",
            current_dir
        ));
        let issue_tracker = self.issues.write().await;
        issue_tracker.add_issue(IssueType::NoStoragePath, "Using current directory fallback");

        // Get available space for current directory fallback
        let available_space = storage::get_available_space(&current_dir);
        Ok((current_dir, available_space))
    }

    async fn detect_cross_platform_storage_path(
        &self,
    ) -> Result<(String, Option<u64>), Box<dyn std::error::Error>> {
        // For non-Linux systems, try user directory first
        let default_path = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map(|home| format!("{}/prime-worker", home))
            .unwrap_or_else(|_| {
                std::env::current_dir()
                    .map(|p| p.join("prime-worker-data").to_string_lossy().to_string())
                    .unwrap_or_else(|_| "./prime-worker-data".to_string())
            });

        Console::info(
            "Storage Path",
            &format!("Using platform-appropriate storage path: {}", default_path),
        );
        let issue_tracker = self.issues.write().await;
        issue_tracker.add_issue(
            IssueType::NoStoragePath,
            "Using platform-appropriate storage path",
        );

        // Non-Linux systems don't have available space detection
        Ok((default_path, None))
    }
}
