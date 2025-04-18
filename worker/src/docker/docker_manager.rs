use bollard::container::{
    Config, CreateContainerOptions, ListContainersOptions, LogsOptions, StartContainerOptions,
};
use bollard::container::{InspectContainerOptions, LogOutput};
use bollard::errors::Error as DockerError;
use bollard::image::CreateImageOptions;
use bollard::models::ContainerStateStatusEnum;
use bollard::models::DeviceRequest;
use bollard::models::HostConfig;
use bollard::volume::CreateVolumeOptions;
use bollard::Docker;
use futures_util::StreamExt;
use log::{debug, error, info};
use shared::models::node::GpuSpecs;
use std::collections::HashMap;
use std::time::Duration;
use strip_ansi_escapes::strip;

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
    storage_path: Option<String>,
}

impl DockerManager {
    const DEFAULT_LOG_TAIL: i64 = 300;
    /// Create a new DockerManager instance
    pub fn new(storage_path: Option<String>) -> Result<Self, DockerError> {
        let docker = match Docker::connect_with_unix_defaults() {
            Ok(docker) => docker,
            Err(e) => {
                error!("Failed to connect to Docker daemon: {}", e);
                return Err(e);
            }
        };
        Ok(Self {
            docker,
            storage_path,
        })
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

    #[allow(clippy::too_many_arguments)]
    /// Start a new container with the given image and configuration
    pub async fn start_container(
        &self,
        image: &str,
        name: &str,
        env_vars: Option<HashMap<String, String>>,
        command: Option<Vec<String>>,
        gpu: Option<GpuSpecs>,
        // Simple Vec of (host_path, container_path, read_only)
        volumes: Option<Vec<(String, String, bool)>>,
        shm_size: Option<u64>,
    ) -> Result<String, DockerError> {
        info!("Starting to pull image: {}", image);

        let mut final_volumes = Vec::new();
        if self.storage_path.is_some() {
            // Create task-specific data volume
            let volume_name = format!("{}_data", name);
            let path = format!(
                "{}/{}",
                self.storage_path.clone().unwrap(),
                name.trim_start_matches('/')
            );
            std::fs::create_dir_all(&path)?;

            self.docker
                .create_volume(CreateVolumeOptions {
                    name: volume_name.clone(),
                    driver: "local".to_string(),
                    driver_opts: HashMap::from([
                        ("type".to_string(), "none".to_string()),
                        ("o".to_string(), "bind".to_string()),
                        ("device".to_string(), path),
                    ]),
                    labels: HashMap::new(),
                })
                .await?;

            final_volumes.push((volume_name, "/data".to_string(), false));

            // Create shared volume if it doesn't exist
            let shared_path = format!("{}/shared", self.storage_path.clone().unwrap());
            std::fs::create_dir_all(&shared_path)?;

            self.docker
                .create_volume(CreateVolumeOptions {
                    name: "shared_data".to_string(),
                    driver: "local".to_string(),
                    driver_opts: HashMap::from([
                        ("type".to_string(), "none".to_string()),
                        ("o".to_string(), "bind".to_string()),
                        ("device".to_string(), shared_path),
                    ]),
                    labels: HashMap::new(),
                })
                .await?;

            final_volumes.push(("shared_data".to_string(), "/shared".to_string(), false));
        }

        self.pull_image(image).await?;

        let env = env_vars.map(|vars| {
            vars.iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<String>>()
        });
        let volume_binds = {
            let mut binds = final_volumes
                .iter()
                .map(|(vol, container, read_only)| {
                    if *read_only {
                        format!("{}:{}:ro", vol, container)
                    } else {
                        format!("{}:{}", vol, container)
                    }
                })
                .collect::<Vec<String>>();

            if let Some(vols) = volumes {
                binds.extend(vols.into_iter().map(|(host, container, read_only)| {
                    if read_only {
                        format!("{}:{}:ro", host, container)
                    } else {
                        format!("{}:{}", host, container)
                    }
                }));
            }

            Some(binds)
        };

        let host_config = if gpu.is_some() {
            let gpu = gpu.unwrap();
            let device_ids = match &gpu.indices {
                Some(indices) if !indices.is_empty() => {
                    // Use specific GPU indices if available
                    indices.iter().map(|i| i.to_string()).collect()
                }
                _ => {
                    // Request all available GPUs if no specific indices
                    vec!["all".to_string()]
                }
            };

            Some(HostConfig {
                extra_hosts: Some(vec!["host.docker.internal:host-gateway".into()]),
                device_requests: Some(vec![DeviceRequest {
                    driver: Some("nvidia".into()),
                    count: None,
                    device_ids: Some(device_ids),
                    capabilities: Some(vec![vec!["gpu".into()]]),
                    options: Some(HashMap::new()),
                }]),
                binds: volume_binds,
                shm_size: shm_size.map(|s| s as i64),
                ..Default::default()
            })
        } else {
            Some(HostConfig {
                extra_hosts: Some(vec!["host.docker.internal:host-gateway".into()]),
                binds: volume_binds,
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

        info!("Creating container with name: {}", name);
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

    pub async fn remove_container(&self, container_id: &str) -> Result<(), DockerError> {
        let container = match self.get_container_details(container_id).await {
            Ok(c) => Some(c),
            Err(_) => None,
        };

        if container.is_some() {
            if let Err(e) = self.docker.stop_container(container_id, None).await {
                error!("Failed to stop container: {}", e);
            }
        }

        let max_retries = 10;

        // --- Step 1: Remove container with retries ---
        for attempt in 0..max_retries {
            match self.docker.remove_container(container_id, None).await {
                Ok(_) => {
                    info!("Container {} removed successfully", container_id);
                    break;
                }
                Err(DockerError::DockerResponseServerError {
                    status_code: 409, ..
                }) => {
                    debug!(
                        "Container removal in progress, retrying ({}/{})",
                        attempt + 1,
                        max_retries
                    );
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(DockerError::DockerResponseServerError {
                    status_code: 404, ..
                }) => {
                    break;
                }
                Err(e) => {
                    info!("Failed to remove container {}: {}", container_id, e);
                    return Err(e);
                }
            }
        }

        // --- Step 2: Ensure container is actually gone ---
        let mut gone = false;
        for _ in 0..5 {
            match self
                .docker
                .inspect_container(container_id, None::<InspectContainerOptions>)
                .await
            {
                Ok(_) => {
                    debug!("Container {} still exists, waiting...", container_id);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(DockerError::DockerResponseServerError {
                    status_code: 404, ..
                }) => {
                    gone = true;
                    break;
                }
                Err(e) => {
                    error!("Failed to inspect container {}: {}", container_id, e);
                    break;
                }
            }
        }

        if !gone {
            error!("Container {} still exists after waiting", container_id);
        }

        // --- Step 3: Remove volume with retries ---
        if let Some(container) = container {
            let trimmed_name = container.names.first().unwrap().trim_start_matches('/');
            let volume_name = format!("{}_data", trimmed_name);

            for attempt in 0..max_retries {
                match self.docker.remove_volume(&volume_name, None).await {
                    Ok(_) => {
                        info!("Volume {} removed successfully", volume_name);
                        break;
                    }
                    Err(DockerError::DockerResponseServerError {
                        status_code: 404, ..
                    }) => {
                        debug!("Volume {} already removed", volume_name);
                        break;
                    }
                    Err(DockerError::DockerResponseServerError {
                        status_code: 409, ..
                    }) => {
                        debug!(
                            "Volume {} is still in use, retrying ({}/{})",
                            volume_name,
                            attempt + 1,
                            max_retries
                        );
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                    Err(e) => {
                        error!("Failed to remove volume {}: {}", volume_name, e);
                        break;
                    }
                }
            }

            // --- Step 4: Remove directory with retries ---
            if let Some(path) = &self.storage_path {
                let dir_path = format!("{}/{}", path, trimmed_name);

                // Check if directory exists before attempting to remove it
                if std::path::Path::new(&dir_path).exists() {
                    let mut success = false;

                    for attempt in 0..max_retries {
                        match std::fs::remove_dir_all(&dir_path) {
                            Ok(_) => {
                                info!("Directory {} removed successfully", dir_path);
                                success = true;
                                break;
                            }
                            Err(e) => {
                                debug!(
                                    "Attempt {}/{} failed to remove dir {}: {}",
                                    attempt + 1,
                                    max_retries,
                                    dir_path,
                                    e
                                );
                                tokio::time::sleep(Duration::from_secs(1)).await;
                            }
                        }
                    }

                    if !success {
                        error!(
                            "Failed to remove directory {} after {} attempts — trying fallback",
                            dir_path, max_retries
                        );

                        // Try `rm -rf` as fallback
                        match std::process::Command::new("rm")
                            .arg("-rf")
                            .arg(&dir_path)
                            .status()
                        {
                            Ok(status) if status.success() => {
                                info!("Fallback removal of {} succeeded", dir_path);
                            }
                            Ok(status) => {
                                error!("Fallback rm -rf failed with status {}", status);
                            }
                            Err(e) => {
                                error!("Failed to execute fallback rm -rf: {}", e);
                            }
                        }
                    }
                } else {
                    debug!("Directory {} does not exist, skipping removal", dir_path);
                }
            }
        }

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

        debug!("Retrieved details for container {}", container_id);
        Ok(info)
    }

    pub async fn restart_container(&self, container_id: &str) -> Result<(), DockerError> {
        debug!("Restarting container: {}", container_id);
        self.docker.restart_container(container_id, None).await?;
        debug!("Container {} restarted successfully", container_id);
        Ok(())
    }

    pub async fn get_container_logs(
        &self,
        container_id: &str,
        tail: Option<i64>,
    ) -> Result<String, DockerError> {
        debug!("Fetching logs for container: {}", container_id);
        let tail_value = tail.unwrap_or(Self::DEFAULT_LOG_TAIL).to_string();
        let options = LogsOptions::<String> {
            stdout: true,
            stderr: true,
            tail: tail_value,
            timestamps: false,
            follow: false,
            ..Default::default()
        };

        let mut logs_stream = self.docker.logs(container_id, Some(options));
        let mut all_logs = Vec::new();
        // Buffer to accumulate a line that might be updated via carriage returns
        let mut current_line = String::new();

        while let Some(log_result) = logs_stream.next().await {
            match log_result {
                Ok(log_output) => {
                    let message_bytes = match log_output {
                        LogOutput::StdOut { message } | LogOutput::StdErr { message } => message,
                        LogOutput::Console { message } => message,
                        LogOutput::StdIn { message } => message,
                    };

                    // Strip ANSI escape sequences, skipping on error.
                    let cleaned: Vec<u8> = strip(&message_bytes);

                    // Convert to string without immediately replacing '\r'
                    let cleaned_str = String::from_utf8_lossy(&cleaned);

                    if cleaned_str.contains('\r') {
                        // For messages with carriage returns, treat it as an update to the current line.
                        let parts: Vec<&str> = cleaned_str.split('\r').collect();
                        if let Some(last_segment) = parts.last() {
                            // Update our current line buffer with the latest segment.
                            current_line = last_segment.to_string();
                        }
                    } else {
                        // Flush any buffered progress update if present.
                        if !current_line.is_empty() {
                            all_logs.push(current_line.clone());
                            current_line.clear();
                        }
                        // Process the message normally.
                        for line in cleaned_str.lines() {
                            let trimmed = line.trim();
                            if !trimmed.is_empty() {
                                all_logs.push(trimmed.to_string());
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Error getting logs: {}", e);
                    return Err(e);
                }
            }
        }
        // Push any leftover buffered line.
        if !current_line.is_empty() {
            all_logs.push(current_line);
        }
        let logs = all_logs.join("\n");
        debug!("Successfully retrieved logs for container {}", container_id);
        Ok(logs)
    }
}
