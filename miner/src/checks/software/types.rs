#[derive(Debug)]
pub enum SoftwareCheckError {
    DockerNotInstalled,
    DockerNotRunning,
    Other(String),
}
