use super::docker::check_docker_installed;
use super::types::SoftwareCheckError;
use crate::console::Console;
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
    Console::section("Software Checks");

    // Check Docker installation and connectivity
    Console::title("Docker:");
    check_docker_installed()?;

    Ok(())
}
