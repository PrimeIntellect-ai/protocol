use bollard::container::{
    Config, CreateContainerOptions,  ListContainersOptions, StartContainerOptions 
};
use bollard::models::DeviceRequest;
use bollard::models::HostConfig;
use bollard::errors::Error as DockerError;
use bollard::image::CreateImageOptions;
use bollard::models::ContainerStateStatusEnum;
use bollard::Docker;
use futures_util::StreamExt;
use log::{debug, error, info};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ContainerInfo {
    pub id: String,
    #[allow(unused)]
    pub image: String,
    pub names: Vec<String>,
    #[allow(unused)]
    pub created: i64,
}

#[derive(Debug, Clone)]
pub struct ContainerDetails {
    #[allow(unused)]
    pub id: String,
    #[allow(unused)]
    pub image: String,
    pub status: Option<ContainerStateStatusEnum>,
    #[allow(unused)]
    pub names: Vec<String>,
    #[allow(unused)]
    pub created: i64,
}

pub struct DockerManager {
    docker: Docker,
}

impl DockerManager {
    /// Create a new DockerManager instance
    pub fn new() -> Result<Self, DockerError> {
        let docker = match Docker::connect_with_unix_defaults() {
            Ok(docker) => docker,
            Err(e) => {
                error!("Failed to connect to Docker daemon: {}", e);
                return Err(e);
            }
        };
        Ok(Self { docker })
    }

    /// Pull a Docker image if it doesn't exist locally
    pub async fn pull_image(&self, image: &str) -> Result<(), DockerError> {
        debug!("Checking if image needs to be pulled: {}", image);
        if self.docker.inspect_image(image).await.is_err() {
            info!("Image {} not found locally, pulling...", image);

            // Split image name and tag
            let (image_name, tag) = match image.split_once(':') {
                Some((name, tag)) => (name, tag),
                None => (image, "latest"), // Default to latest if no tag specified
            };

            let options = CreateImageOptions {
                from_image: image_name,
                tag,
                ..Default::default()
            };

            let mut image_stream = self.docker.create_image(Some(options), None, None);

            while let Some(info) = image_stream.next().await {
                match info {
                    Ok(create_info) => {
                        debug!("Pull progress: {:?}", create_info);
                    }
                    Err(e) => return Err(e),
                }
            }

            info!("Successfully pulled image {}", image);
        } else {
            debug!("Image {} already exists locally", image);
        }
        Ok(())
    }

    /// Start a new container with the given image and configuration
    pub async fn start_container(
        &self,
        image: &str,
        name: &str,
        env_vars: Option<HashMap<String, String>>,
        command: Option<Vec<String>>,
        gpu_enabled: bool,
    ) -> Result<String, DockerError> {
        println!("Starting to pull image: {}", image);
        self.pull_image(image).await?;

        let env = env_vars.map(|vars| {
            println!("Setting environment variables: {:?}", vars);
            debug!("Setting environment variables");
            vars.iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<String>>()
        });

       

        let host_config = if gpu_enabled {
            Some(HostConfig {
                extra_hosts: Some(vec!["host.docker.internal:host-gateway".into()]),
                device_requests: Some(vec![DeviceRequest {
                    driver: Some("".into()),
                    count: Some(-1),
                    device_ids: None,
                    capabilities: Some(vec![vec!["gpu".into()]]),
                    options: Some(HashMap::new()),
                }]),
                ..Default::default()
            })
        } else {
            Some(HostConfig {
                extra_hosts: Some(vec!["host.docker.internal:host-gateway".into()]),
                ..Default::default()
            })
        };
        // Create container configuration
        let config = Config {
            image: Some(image),
            env: env.as_ref().map(|e| e.iter().map(String::as_str).collect()),
            cmd: command
                .as_ref()
                .map(|c| c.iter().map(String::as_str).collect()),
            host_config,
            ..Default::default()
        };

        println!("Creating container with name: {}", name);
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
        println!("Starting container with ID: {}", container.id);
        debug!("Starting container {}", container.id);

        self.docker
            .start_container(&container.id, None::<StartContainerOptions<String>>)
            .await?;
        info!("Container {} started successfully", container.id);
        println!("Container {} started successfully", container.id);

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

    pub async fn list_containers(&self, list_all: bool) -> Result<Vec<ContainerInfo>, DockerError> {
        debug!("Listing running containers");
        let options = Some(ListContainersOptions::<String> {
            all: list_all, // If true, list all containers. If false, only list running containers
            ..Default::default()
        });

        let containers = self.docker.list_containers(options).await?;
        let container_details: Vec<ContainerInfo> = containers
            .iter()
            .map(|c| ContainerInfo {
                id: c.id.clone().unwrap_or_default(),
                image: c.image.clone().unwrap_or_default(),
                names: c.names.clone().unwrap_or_default(),
                created: c.created.unwrap_or_default(),
            })
            .collect();

        Ok(container_details)
    }

    /// Get details for a specific container by ID
    pub async fn get_container_details(
        &self,
        container_id: &str,
    ) -> Result<ContainerDetails, DockerError> {
        debug!("Getting details for container: {}", container_id);
        let container = self.docker.inspect_container(container_id, None).await?;
        let info = ContainerDetails {
            id: container.id.unwrap_or_default(),
            image: container.image.unwrap_or_default(),
            status: container.state.and_then(|s| s.status),
            names: vec![container.name.unwrap_or_default()],
            created: container
                .created
                .and_then(|c| c.parse::<i64>().ok())
                .unwrap_or_default(),
        };

        info!("Retrieved details for container {}", container_id);
        Ok(info)
    }
}
