use crate::console::Console;

use super::types::SoftwareCheckError;

pub fn check_docker_installed() -> Result<(), SoftwareCheckError> {
    let docker_path = std::process::Command::new("which")
        .arg("docker")
        .output()
        .map_err(|e| {
            SoftwareCheckError::Other(format!("Failed to execute 'which docker': {}", e))
        })?;

    if !docker_path.status.success() {
        return Err(SoftwareCheckError::DockerNotInstalled);
    }

    let docker_info = std::process::Command::new("docker").output().map_err(|e| {
        SoftwareCheckError::Other(format!(
            "Failed to execute 'docker ps': {}. You may need to add your user to the docker group.",
            e
        ))
    })?;

    if !docker_info.status.success() {
        return Err(SoftwareCheckError::DockerNotRunning);
    }

    Console::success("Docker ready");

    // Check if NVIDIA Container Toolkit is installed using which command
    let nvidia_toolkit = std::process::Command::new("which")
        .arg("nvidia-ctk")
        .output()
        .map_err(|e| SoftwareCheckError::Other(format!("Failed to check for nvidia-ctk: {}", e)))?;

    if nvidia_toolkit.status.success() {
        // If which succeeds, check if it's working properly
        let version_check = std::process::Command::new("nvidia-ctk")
            .arg("--version")
            .output()
            .map_err(|e| SoftwareCheckError::Other(format!("Failed to run nvidia-ctk: {}", e)))?;

        if version_check.status.success() {
            Console::success("NVIDIA toolkit ready");
        } else {
            Console::error("NVIDIA toolkit not configured");
        }
    } else {
        Console::error("NVIDIA toolkit not found");
    }

    Ok(())
}
