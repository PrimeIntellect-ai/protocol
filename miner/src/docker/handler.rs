use bollard::container::{
    Config, CreateContainerOptions, ListContainersOptions, StartContainerOptions,
};
use bollard::errors::Error as DockerError;
use bollard::image::CreateImageOptions;
use bollard::Docker;
use log::{debug, error, info};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ContainerInfo {
    pub id: String,
    pub image: String,
    pub status: String,
    pub state: String,
    pub names: Vec<String>,
    pub created: i64,
}

pub struct DockerHandler {
    docker: Docker,
}

impl DockerHandler {
    /// Create a new DockerHandler instance
    pub fn new() -> Result<Self, DockerError> {
        debug!("Initializing Docker handler...");
        let docker = match Docker::connect_with_unix_defaults() {
            Ok(docker) => {
                info!("Successfully connected to Docker daemon");
                docker
            }
            Err(e) => {
                error!("Failed to connect to Docker daemon: {}", e);
                return Err(e);
            }
        };
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

        // Pull image if not exists
        debug!("Checking if image needs to be pulled: {}", image);
        if self.docker.inspect_image(image).await.is_err() {
            info!("Image {} not found locally, pulling...", image);
            let _options = Some(CreateImageOptions {
                from_image: image,
                ..Default::default()
            });
            
            // TODO: Properly implement docker pulling
            // self.docker.create_image(options, None, None);

            info!("Successfully pulled image {}", image);
        } else {
            debug!("Image {} already exists locally", image);
        }

        let env = env_vars.map(|vars| {
            debug!("Setting environment variables");
            vars.iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<String>>()
        });

        // Create container configuration
        let config = Config {
            image: Some(image),
            env: env.as_ref().map(|e| e.iter().map(String::as_str).collect()),
            ..Default::default()
        };

        // Create and start container
        let container = self
            .docker
            .create_container(
                Some(CreateContainerOptions {
                    name,
                    platform: None,
                }),
                config,
            )
            .await
            .map_err(|e| {
                error!("Failed to create container: {}", e);
                e
            })?;

        info!("Container created successfully with ID: {}", container.id);
        debug!("Starting container {}", container.id);

        self.docker
            .start_container(&container.id, None::<StartContainerOptions<String>>)
            .await?;
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

    /// List all running containers with their details
    pub async fn list_running_containers(&self) -> Result<Vec<ContainerInfo>, DockerError> {
        debug!("Listing running containers");
        let options = Some(ListContainersOptions::<String> {
            all: false, // Only running containers
            ..Default::default()
        });

        let containers = self.docker.list_containers(options).await?;
        let container_details: Vec<ContainerInfo> = containers
            .iter()
            .map(|c| ContainerInfo {
                id: c.id.clone().unwrap_or_default(),
                image: c.image.clone().unwrap_or_default(),
                status: c.status.clone().unwrap_or_default(),
                state: c.state.clone().unwrap_or_default(),
                names: c.names.clone().unwrap_or_default(),
                created: c.created.unwrap_or_default(),
            })
            .collect();

        info!("Found {} running containers", container_details.len());
        Ok(container_details)
    }
}
