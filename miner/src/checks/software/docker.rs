use super::types::SoftwareCheckError;
use colored::*;

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

    let docker_info = std::process::Command::new("docker")
        .arg("info")
        .output()
        .map_err(|e| {
            SoftwareCheckError::Other(format!("Failed to execute 'docker info': {}", e))
        })?;

    if !docker_info.status.success() {
        return Err(SoftwareCheckError::DockerNotRunning);
    }

    println!(
        "{}",
        "✓ Docker check passed - Docker is installed and running".green()
    );
    Ok(())
}
