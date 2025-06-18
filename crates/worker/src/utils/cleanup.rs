use crate::console::Console;
use crate::docker::docker_manager::DockerManager;
use log::{debug, error, info};
use std::io::{self, Write};

pub struct CleanupManager {
    docker_manager: Option<DockerManager>,
}

impl CleanupManager {
    pub fn new(storage_path: Option<String>) -> Self {
        let docker_manager = match storage_path {
            Some(path) => match DockerManager::new(path) {
                Ok(manager) => Some(manager),
                Err(e) => {
                    error!("Failed to initialize Docker manager for cleanup: {}", e);
                    None
                }
            },
            None => {
                // Use a default storage path when none is provided
                let default_path = Self::get_default_storage_path();
                match DockerManager::new(default_path) {
                    Ok(manager) => Some(manager),
                    Err(e) => {
                        error!("Failed to initialize Docker manager for cleanup: {}", e);
                        None
                    }
                }
            }
        };

        Self { docker_manager }
    }

    fn get_default_storage_path() -> String {
        const APP_DIR_NAME: &str = "prime-worker";

        // Try user home directory first
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/{}", home, APP_DIR_NAME);
        }

        // Fallback to current directory
        std::env::current_dir()
            .map(|p| {
                p.join(format!("{}-data", APP_DIR_NAME))
                    .to_string_lossy()
                    .to_string()
            })
            .unwrap_or_else(|_| format!("./{}-data", APP_DIR_NAME))
    }

    /// Prompt user for cleanup and perform it if confirmed
    pub async fn prompt_and_cleanup(&self) -> bool {
        if self.docker_manager.is_none() {
            debug!("Docker manager not available, skipping cleanup");
            return false;
        }

        let docker_manager = self.docker_manager.as_ref().unwrap();

        // Check if there's anything to clean up
        let containers_result = docker_manager.list_prime_containers().await;
        let has_containers = match &containers_result {
            Ok(containers) => !containers.is_empty(),
            Err(_) => false,
        };

        if !has_containers {
            debug!("No prime worker containers found to clean up");
            return true;
        }

        // Show what will be cleaned up
        Console::section("🧹 Cleanup Options");
        Console::warning("The following resources were found:");

        if let Ok(containers) = &containers_result {
            Console::info("", &format!("• {} worker containers", containers.len()));
            for container in containers {
                Console::info("", &format!("  - {}", container.names.join(", ")));
            }
        }
        Console::info("", "• Associated Docker volumes");
        Console::info("", "• Task directories");

        print!("\nDo you want to clean up these resources? [y/N]: ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            error!("Failed to read user input");
            return false;
        }

        let input = input.trim().to_lowercase();
        if input != "y" && input != "yes" {
            Console::info("", "Cleanup skipped.");
            return true;
        }

        // Perform cleanup
        Console::title("🐳 Cleaning up containers");
        if let Ok(containers) = containers_result {
            for container in containers {
                Console::info(
                    "",
                    &format!("Removing container: {}", container.names.join(", ")),
                );
                match docker_manager.remove_container(&container.id).await {
                    Ok(_) => {
                        Console::success(&format!("✅ Removed container: {}", container.id));
                    }
                    Err(e) => {
                        Console::user_error(&format!(
                            "❌ Failed to remove container {}: {}",
                            container.id, e
                        ));
                    }
                }
            }
        }

        // Clean up directories
        Console::title("📁 Cleaning up directories");
        match docker_manager.cleanup_task_directories().await {
            Ok(cleaned_dirs) => {
                if cleaned_dirs.is_empty() {
                    Console::info("", "No task directories found to clean");
                } else {
                    Console::success(&format!(
                        "✅ Cleaned {} task directories",
                        cleaned_dirs.len()
                    ));
                }
            }
            Err(e) => {
                Console::user_error(&format!("❌ Failed to clean directories: {}", e));
            }
        }

        Console::success("🎉 Cleanup completed");
        true
    }

    /// Perform cleanup without prompting (for --force flag)
    #[allow(dead_code)]
    pub async fn force_cleanup(&self) -> bool {
        if self.docker_manager.is_none() {
            debug!("Docker manager not available, skipping cleanup");
            return false;
        }

        let docker_manager = self.docker_manager.as_ref().unwrap();

        Console::section("🧹 Force Cleanup");

        // Clean up containers
        Console::title("🐳 Cleaning up containers");
        match docker_manager.list_prime_containers().await {
            Ok(containers) => {
                if containers.is_empty() {
                    Console::info("", "No prime worker containers found");
                } else {
                    for container in containers {
                        match docker_manager.remove_container(&container.id).await {
                            Ok(_) => {
                                info!("Removed container: {}", container.id);
                            }
                            Err(e) => {
                                error!("Failed to remove container {}: {}", container.id, e);
                            }
                        }
                    }
                }
            }
            Err(e) => {
                error!("Failed to list containers: {}", e);
            }
        }

        // Clean up directories
        Console::title("📁 Cleaning up directories");
        match docker_manager.cleanup_task_directories().await {
            Ok(cleaned_dirs) => {
                if !cleaned_dirs.is_empty() {
                    info!("Cleaned {} task directories", cleaned_dirs.len());
                }
            }
            Err(e) => {
                error!("Failed to clean directories: {}", e);
            }
        }

        true
    }
}
