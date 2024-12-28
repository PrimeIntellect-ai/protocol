use super::docker::check_docker_installed;
use super::types::SoftwareCheckError;
use colored::*;
use std::error::Error;

impl std::fmt::Display for SoftwareCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::DockerNotInstalled => write!(f, "Docker is not installed"),
            Self::DockerNotRunning => write!(f, "Docker daemon is not running"),
            Self::Other(msg) => write!(f, "Software error: {}", msg),
        }
    }
}

impl Error for SoftwareCheckError {}

pub fn run_software_check() -> Result<(), SoftwareCheckError> {
    println!("\n{}", "Software Requirements Check:".blue().bold());

    // Check Docker installation and connectivity
    println!("\n{}", "Docker:".blue().bold());
    check_docker_installed()?;

    println!("\n{}", "✓ All software checks passed".green().bold());
    Ok(())
}
