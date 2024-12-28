use bollard::Docker;
use bollard::container::{Config, CreateContainerOptions, StartContainerOptions};
use bollard::errors::Error as DockerError;
use std::collections::HashMap;
use log::{debug, info, error};

pub struct DockerHandler {
    docker: Docker,
}

impl DockerHandler {
    /// Create a new DockerHandler instance
    pub fn new() -> Result<Self, DockerError> {
        debug!("Initializing Docker handler...");
        let docker = Docker::connect_with_local_defaults()?;
        info!("Successfully connected to Docker daemon");
        Ok(Self { docker })
    }

    /// Start a new container with the given image and configuration
    pub async fn start_container(
        &self,
        image: &str,
        name: &str,
        env_vars: Option<HashMap<String, String>>,
    ) -> Result<String, DockerError> {
        debug!("Starting container creation process:");
        debug!("  Image: {}", image);
        debug!("  Container name: {}", name);

        // Prepare environment variables
        let env = env_vars.map(|vars| {
            debug!("  Setting environment variables:");
            vars.iter()
                .map(|(k, v)| {
                    debug!("    {}={}", k, v);
                    format!("{}={}", k, v)
                })
                .collect::<Vec<String>>()
        });

        // Convert Vec<String> to Vec<&str> for the config
        let env_str = env.as_ref().map(|e| {
            e.iter()
                .map(|s| s.as_str())
                .collect::<Vec<&str>>()
        });

        // Create container configuration
        let config = Config {
            image: Some(image),
            env: env_str,
            ..Default::default()
        };

        // Create container options
        let options = Some(CreateContainerOptions {
            name,
            platform: None,
        });

        // Create and start the container
        debug!("Creating container with name: {}", name);
        let container = match self.docker.create_container(options, config).await {
            Ok(container) => {
                info!("Container created successfully with ID: {}", container.id);
                container
            },
            Err(e) => {
                error!("Failed to create container: {}", e);
                return Err(DockerError::from(e));
            }
        };
        
        debug!("Starting container {}", container.id);
        self.docker.start_container(&container.id, Some(StartContainerOptions::<String>::default())).await?;
        info!("Container {} started successfully", container.id);

        Ok(container.id)
    }

    /// Stop and remove a container
    pub async fn remove_container(&self, container_id: &str) -> Result<(), DockerError> {
        debug!("Stopping container: {}", container_id);
        self.docker.stop_container(container_id, None).await?;
        info!("Container {} stopped successfully", container_id);
        
        debug!("Removing container: {}", container_id);
        self.docker.remove_container(container_id, None).await?;
        info!("Container {} removed successfully", container_id);
        
        Ok(())
    }
}
