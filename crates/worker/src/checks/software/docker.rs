use crate::checks::issue::{IssueReport, IssueType};
use crate::console::Console;
use bollard::container::ListContainersOptions;
use bollard::Docker;
use std::sync::Arc;
use tokio::sync::RwLock;

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
                format!("Failed to execute 'which docker': {e}"),
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
                "Failed to execute 'docker ps': {e}. You may need to add your user to the docker group."
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
                        format!("Docker API permission denied: {e}. You may need to add your user to the docker group. To fix this, run: 'sudo usermod -aG docker $USER' and then log out and back in."),
                    );
                }
            }
        }
        Err(e) => {
            issue_tracker.add_issue(
                IssueType::DockerNotInstalled,
                format!("Failed to connect to Docker API: {e}. You may need to add your user to the docker group."),
            );
        }
    }

    Console::success("Docker ready");

    // Check if NVIDIA Container Toolkit is installed using which command
    let nvidia_toolkit = std::process::Command::new("which")
        .arg("nvidia-ctk")
        .output()
        .map_err(|e| {
            issue_tracker.add_issue(
                IssueType::ContainerToolkitNotInstalled,
                format!("Failed to check for nvidia-ctk: {e}"),
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
                    format!("Failed to run nvidia-ctk: {e}"),
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

    Ok(())
}
