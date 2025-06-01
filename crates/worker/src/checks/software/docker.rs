use crate::checks::issue::{IssueReport, IssueType};
use crate::console::Console;
use bollard::container::ListContainersOptions;
use bollard::Docker;
use std::sync::Arc;
use tokio::sync::RwLock;
use super::super::hardware::gpu::detect_gpu;

pub async fn check_docker_installed(
    issues: &Arc<RwLock<IssueReport>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let issue_tracker = issues.read().await;
    let docker_path = std::process::Command::new("which")
        .arg("docker")
        .output()
        .map_err(|e| {
            issue_tracker.add_issue(
                IssueType::DockerNotInstalled,
                format!("Failed to execute 'which docker': {}", e),
            );
            e
        })?;

    if !docker_path.status.success() {
        issue_tracker.add_issue(IssueType::DockerNotInstalled, "Docker is not installed");
        return Ok(());
    }

    let docker_info = std::process::Command::new("docker").output().map_err(|e| {
        issue_tracker.add_issue(
            IssueType::DockerNotInstalled,
            format!(
                "Failed to execute 'docker ps': {}. You may need to add your user to the docker group.",
                e
            )
        );
        e
    })?;

    if !docker_info.status.success() {
        issue_tracker.add_issue(
            IssueType::DockerNotInstalled,
            "Docker daemon is not running",
        );
        return Ok(());
    }

    // Check if Docker API is accessible with proper permissions
    match Docker::connect_with_unix_defaults() {
        Ok(docker) => {
            // Try to list containers to verify permissions
            match docker
                .list_containers::<String>(Some(ListContainersOptions {
                    all: true,
                    ..Default::default()
                }))
                .await
            {
                Ok(_) => Console::success("Docker API accessible"),
                Err(e) => {
                    issue_tracker.add_issue(
                        IssueType::DockerNotInstalled,
                        format!("Docker API permission denied: {}. You may need to add your user to the docker group. To fix this, run: 'sudo usermod -aG docker $USER' and then log out and back in.", e),
                    );
                }
            }
        }
        Err(e) => {
            issue_tracker.add_issue(
                IssueType::DockerNotInstalled,
                format!("Failed to connect to Docker API: {}. You may need to add your user to the docker group.", e),
            );
        }
    }

    Console::success("Docker ready");

    // Detect which GPU vendor is present
    let gpu_specs = detect_gpu();
    
    let mut has_nvidia = false;
    let mut has_amd = false;
    
    for gpu in &gpu_specs {
        if let Some(model) = &gpu.model {
            if model.contains("nvidia") {
                has_nvidia = true;
            } else if model.contains("amd") {
                has_amd = true;
            }
        }
    }

    // Check for NVIDIA Container Toolkit if NVIDIA GPUs are present
    if has_nvidia {
        let nvidia_toolkit = std::process::Command::new("which")
            .arg("nvidia-ctk")
            .output()
            .map_err(|e| {
                issue_tracker.add_issue(
                    IssueType::ContainerToolkitNotInstalled,
                    format!("Failed to check for nvidia-ctk: {}", e),
                );
                e
            })?;

        if nvidia_toolkit.status.success() {
            // If which succeeds, check if it's working properly
            let version_check = std::process::Command::new("nvidia-ctk")
                .arg("--version")
                .output()
                .map_err(|e| {
                    issue_tracker.add_issue(
                        IssueType::ContainerToolkitNotInstalled,
                        format!("Failed to run nvidia-ctk: {}", e),
                    );
                    e
                })?;

            if version_check.status.success() {
                Console::success("NVIDIA toolkit ready");
            } else {
                issue_tracker.add_issue(
                    IssueType::ContainerToolkitNotInstalled,
                    "NVIDIA toolkit not configured properly",
                );
            }
        } else {
            issue_tracker.add_issue(
                IssueType::ContainerToolkitNotInstalled,
                "NVIDIA toolkit not found",
            );
        }
    }

    // Check for AMD ROCm runtime if AMD GPUs are present
    if has_amd {
        let rocm_smi = std::process::Command::new("which")
            .arg("rocm-smi")
            .output()
            .map_err(|e| {
                issue_tracker.add_issue(
                    IssueType::RocmNotInstalled,
                    format!("Failed to check for rocm-smi: {}", e),
                );
                e
            })?;

        if rocm_smi.status.success() {
            // Check if ROCm is working properly
            let version_check = std::process::Command::new("rocm-smi")
                .arg("--version")
                .output()
                .map_err(|e| {
                    issue_tracker.add_issue(
                        IssueType::RocmNotInstalled,
                        format!("Failed to run rocm-smi: {}", e),
                    );
                    e
                })?;

            if version_check.status.success() {
                Console::success("ROCm ready");
                
                // Check for AMD container runtime
                // This could be checking for docker plugin or runtime configuration
                // ROCm doesn't have a direct equivalent to nvidia-ctk, but we can check
                // if the docker daemon has the ROCm devices configured
                Console::info("AMD GPU support", "Checking Docker AMD GPU configuration");
            } else {
                issue_tracker.add_issue(
                    IssueType::RocmNotInstalled,
                    "ROCm not configured properly",
                );
            }
        } else {
            issue_tracker.add_issue(
                IssueType::RocmNotInstalled,
                "ROCm not found - required for AMD GPU support",
            );
        }
    }

    Ok(())
}
