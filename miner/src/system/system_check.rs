use colored::*;
use std::error::Error;

#[derive(Debug)]
pub enum SystemCheckError {
    DockerNotInstalled,
    InsufficientDiskSpace,
    InsufficientMemory,
    GPUDriversNotFound,
    Other(String),
}

impl std::fmt::Display for SystemCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        // Match on the error variant and write appropriate message
        match self {
            SystemCheckError::DockerNotInstalled => write!(f, "Docker is not installed"),
            SystemCheckError::InsufficientDiskSpace => write!(f, "Insufficient disk space"),
            SystemCheckError::InsufficientMemory => write!(f, "Insufficient system memory"),
            SystemCheckError::GPUDriversNotFound => write!(f, "GPU drivers not found"),
            SystemCheckError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl Error for SystemCheckError {}

pub fn check_docker_installed() -> Result<(), SystemCheckError> {
    // First check if docker binary exists in PATH
    let docker_exists = std::process::Command::new("which")
        .arg("docker")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);

    if !docker_exists {
        return Err(SystemCheckError::DockerNotInstalled);
    }

    // Simply check if we can connect to docker daemon
    match std::process::Command::new("docker").arg("info").output() {
        Ok(output) if output.status.success() => {
            println!(
                "{}",
                "✓ Docker check passed - Docker is installed and running".green()
            );
            Ok(())
        }
        _ => Err(SystemCheckError::Other(
            "Docker daemon is not running".to_string(),
        )),
    }
}

pub fn run_system_check() -> Result<(), SystemCheckError> {
    check_docker_installed()?;
    println!("{}", "✓ All system checks passed".green().bold());
    Ok(())
}
