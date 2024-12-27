use super::types::SystemCheckError;
use colored::*;

pub fn check_docker_installed() -> Result<(), SystemCheckError> {
    let docker_path = std::process::Command::new("which")
        .arg("docker")
        .output()
        .map_err(|e| SystemCheckError::Other(format!("Failed to execute 'which docker': {}", e)))?;

    if !docker_path.status.success() {
        return Err(SystemCheckError::DockerNotInstalled);
    }

    let docker_info = std::process::Command::new("docker")
        .arg("info")
        .output()
        .map_err(|e| SystemCheckError::Other(format!("Failed to execute 'docker info': {}", e)))?;

    if !docker_info.status.success() {
        return Err(SystemCheckError::Other(
            "Docker daemon is not running".to_string(),
        ));
    }

    println!(
        "{}",
        "✓ Docker check passed - Docker is installed and running".green()
    );
    Ok(())
}
