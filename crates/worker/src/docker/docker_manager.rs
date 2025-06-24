use crate::docker::task_container::TaskContainer;
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
use std::path::{Path, PathBuf};
use std::str::FromStr;
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
    pub status_code: Option<i64>,
    #[allow(unused)]
    pub names: Vec<String>,
    #[allow(unused)]
    pub created: i64,
}

pub struct DockerManager {
    docker: Docker,
    storage_path: String,
    /// Controls whether to use host network mode for containers.
    ///
    /// Currently defaults to host mode (when false) to work around performance issues
    /// with Docker bridge networking on certain cloud providers. This is a trade-off
    /// between security isolation and performance.
    ///
    /// TODO: Investigate root cause of bridge network performance degradation and
    /// implement a more optimal solution that maintains security isolation.
    disable_host_network_mode: bool,
}

impl DockerManager {
    const DEFAULT_LOG_TAIL: i64 = 300;

    /// Sanitize a path component to prevent directory traversal attacks
    fn sanitize_path_component(component: &str) -> Result<String, DockerError> {
        // Remove any path separators and potentially dangerous characters
        let sanitized = component
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
            .collect::<String>();

        // Prevent empty strings and dot-only strings
        if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
            return Err(DockerError::DockerResponseServerError {
                status_code: 400,
                message: format!("Invalid path component: {component}"),
            });
        }

        // Prevent path components that are too long
        if sanitized.len() > 255 {
            return Err(DockerError::DockerResponseServerError {
                status_code: 400,
                message: "Path component too long".to_string(),
            });
        }

        Ok(sanitized)
    }

    /// Safely construct a path within the storage directory
    fn safe_storage_path(&self, components: &[&str]) -> Result<PathBuf, DockerError> {
        let base_path = PathBuf::from(&self.storage_path);

        let mut result = base_path.clone();
        for component in components {
            let sanitized = Self::sanitize_path_component(component)?;
            result = result.join(sanitized);
        }

        // Ensure the final path is still within the base storage path
        if !result.starts_with(&base_path) {
            return Err(DockerError::DockerResponseServerError {
                status_code: 400,
                message: "Path traversal attempt detected".to_string(),
            });
        }

        Ok(result)
    }

    /// Create a directory with secure permissions
    fn create_secure_directory(path: &Path) -> Result<(), DockerError> {
        std::fs::create_dir_all(path).map_err(|e| DockerError::DockerResponseServerError {
            status_code: 500,
            message: format!("Failed to create directory: {e}"),
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(path)
                .map_err(|e| DockerError::DockerResponseServerError {
                    status_code: 500,
                    message: format!("Failed to get directory metadata: {e}"),
                })?
                .permissions();
            perms.set_mode(0o777);
            std::fs::set_permissions(path, perms).map_err(|e| {
                DockerError::DockerResponseServerError {
                    status_code: 500,
                    message: format!("Failed to set directory permissions: {e}"),
                }
            })?;
        }

        Ok(())
    }

    /// Create a new DockerManager instance
    pub fn new(storage_path: String, disable_host_network_mode: bool) -> Result<Self, DockerError> {
        let docker = match Docker::connect_with_unix_defaults() {
            Ok(docker) => docker,
            Err(e) => {
                error!("Failed to connect to Docker daemon: {e}");
                return Err(e);
            }
        };

        // Validate and create storage directory
        let storage_path_buf = PathBuf::from(&storage_path);
        if !storage_path_buf.exists() {
            info!("Creating storage directory: {storage_path}");
            Self::create_secure_directory(&storage_path_buf)?;
        } else {
            // Verify it's a directory and writable
            if !storage_path_buf.is_dir() {
                return Err(DockerError::DockerResponseServerError {
                    status_code: 400,
                    message: format!("Storage path is not a directory: {storage_path}"),
                });
            }
        }

        info!("DockerManager initialized with storage path: {storage_path}");
        Ok(Self {
            docker,
            storage_path,
            disable_host_network_mode,
        })
    }

    /// Pull a Docker image if it doesn't exist locally
    pub async fn pull_image(&self, image: &str) -> Result<(), DockerError> {
        debug!("Checking if image needs to be pulled: {image}");

        // Check if the image uses :latest or :main tag
        let should_always_pull = image.ends_with(":latest") || image.ends_with(":main");

        // Only skip pulling if image exists locally AND it's not a :latest or :main tag
        if !should_always_pull && self.docker.inspect_image(image).await.is_ok() {
            debug!("Image {image} already exists locally");
            return Ok(());
        }

        if should_always_pull {
            info!(
                "Image {image} uses :latest or :main tag, pulling to ensure we have the newest version"
            );
        } else {
            info!("Image {image} not found locally, pulling...");
        }

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
                    debug!("Pull progress: {create_info:?}");
                }
                Err(e) => return Err(e),
            }
        }

        info!("Successfully pulled image {image}");
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
        // Simple Vec of (host_path, container_path, read_only, task_volume)
        volumes: Option<Vec<(String, String, bool, bool)>>,
        shm_size: Option<u64>,
        entrypoint: Option<Vec<String>>,
        restart_policy_max_retries: Option<i64>,
    ) -> Result<String, DockerError> {
        info!("Starting to pull image: {image}");

        let mut final_volumes = Vec::new();
        let volume_name = format!("{name}_data");

        let data_dir_name = match TaskContainer::from_str(name) {
            Ok(task_container) => task_container.data_dir_name(),
            Err(_) => {
                // Fallback to using full container name if extraction fails
                name.trim_start_matches('/').to_string()
            }
        };

        let task_data_path = self.safe_storage_path(&[&data_dir_name, "data"])?;
        Self::create_secure_directory(&task_data_path)?;

        self.docker
            .create_volume(CreateVolumeOptions {
                name: volume_name.clone(),
                driver: "local".to_string(),
                driver_opts: HashMap::from([
                    ("type".to_string(), "none".to_string()),
                    ("o".to_string(), "bind".to_string()),
                    (
                        "device".to_string(),
                        task_data_path.to_string_lossy().to_string(),
                    ),
                ]),
                labels: HashMap::new(),
            })
            .await?;

        final_volumes.push((volume_name, "/data".to_string(), false));

        // Create shared volume if it doesn't exist (idempotent)
        let shared_path = self.safe_storage_path(&["shared"])?;
        Self::create_secure_directory(&shared_path)?;

        // Try to create shared volume, ignore if it already exists
        match self
            .docker
            .create_volume(CreateVolumeOptions {
                name: "shared_data".to_string(),
                driver: "local".to_string(),
                driver_opts: HashMap::from([
                    ("type".to_string(), "none".to_string()),
                    ("o".to_string(), "bind".to_string()),
                    (
                        "device".to_string(),
                        shared_path.to_string_lossy().to_string(),
                    ),
                ]),
                labels: HashMap::new(),
            })
            .await
        {
            Ok(_) => {
                debug!("Shared volume 'shared_data' created successfully");
            }
            Err(DockerError::DockerResponseServerError {
                status_code: 409, ..
            }) => {
                debug!("Shared volume 'shared_data' already exists, reusing");
            }
            Err(e) => {
                error!("Failed to create shared volume: {e}");
                return Err(e);
            }
        }

        final_volumes.push(("shared_data".to_string(), "/shared".to_string(), false));

        self.pull_image(image).await?;

        let env = env_vars.map(|vars| {
            vars.iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<String>>()
        });
        let volume_binds = {
            let mut binds = final_volumes
                .iter()
                .map(|(vol, container, read_only)| {
                    if *read_only {
                        format!("{vol}:{container}:ro")
                    } else {
                        format!("{vol}:{container}")
                    }
                })
                .collect::<Vec<String>>();

            if let Some(vols) = volumes {
                let processed_volumes: Vec<(String, String, bool)> = vols
                    .into_iter()
                    .map(|(host_path, container_path, read_only, task_volume)| {
                        if task_volume {
                            // Create volume mount directory within the task's storage area
                            // Remove leading slash and sanitize the path
                            let sanitized_host_path =
                                host_path.trim_start_matches('/').replace('/', "_");

                            let mount_dir_name = match TaskContainer::from_str(name) {
                                Ok(task_container) => task_container.data_dir_name(),
                                Err(_) => {
                                    // Fallback to using full container name if extraction fails
                                    name.trim_start_matches('/').to_string()
                                }
                            };

                            match self.safe_storage_path(&[
                                &mount_dir_name,
                                "mounts",
                                &sanitized_host_path,
                            ]) {
                                Ok(volume_mount_dir) => {
                                    // Create the directory
                                    if let Err(e) = Self::create_secure_directory(&volume_mount_dir)
                                    {
                                        error!(
                                            "Failed to create volume mount directory {}: {}",
                                            volume_mount_dir.display(),
                                            e
                                        );
                                    }
                                    (
                                        volume_mount_dir.to_string_lossy().to_string(),
                                        container_path,
                                        read_only,
                                    )
                                }
                                Err(e) => {
                                    error!("Failed to create secure path for volume mount: {e}");
                                    // Fallback to original host path for non-task volumes
                                    (host_path, container_path, read_only)
                                }
                            }
                        } else {
                            // Use the original host path for non-task volumes
                            (host_path, container_path, read_only)
                        }
                    })
                    .collect();

                binds.extend(
                    processed_volumes
                        .into_iter()
                        .map(|(host, container, read_only)| {
                            if read_only {
                                format!("{host}:{container}:ro")
                            } else {
                                format!("{host}:{container}")
                            }
                        }),
                );
            }

            Some(binds)
        };

        let network_mode = if self.disable_host_network_mode {
            "bridge".to_string()
        } else {
            "host".to_string()
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
                network_mode: Some(network_mode),
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
                restart_policy: Some(bollard::models::RestartPolicy {
                    name: Some(bollard::models::RestartPolicyNameEnum::ON_FAILURE),
                    maximum_retry_count: restart_policy_max_retries,
                }),
                ..Default::default()
            })
        } else {
            Some(HostConfig {
                network_mode: Some(network_mode),
                extra_hosts: Some(vec!["host.docker.internal:host-gateway".into()]),
                binds: volume_binds,
                restart_policy: Some(bollard::models::RestartPolicy {
                    name: Some(bollard::models::RestartPolicyNameEnum::ON_FAILURE),
                    maximum_retry_count: restart_policy_max_retries,
                }),
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
            entrypoint: entrypoint
                .as_ref()
                .map(|e| e.iter().map(String::as_str).collect()),
            host_config,
            ..Default::default()
        };

        info!("Creating container with name: {name}");
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
                error!("Failed to create container: {e}");
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

    /// Remove container, volumes, and directories
    pub async fn remove_container(&self, container_id: &str) -> Result<(), DockerError> {
        let container = (self.get_container_details(container_id).await).ok();

        if container.is_some() {
            if let Err(e) = self.docker.stop_container(container_id, None).await {
                error!("Failed to stop container: {e}");
            }
        }

        let max_retries = 10;

        // --- Step 1: Remove container with retries ---
        for attempt in 0..max_retries {
            match self.docker.remove_container(container_id, None).await {
                Ok(_) => {
                    info!("Container {container_id} removed successfully");
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
                    info!("Failed to remove container {container_id}: {e}");
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
                    debug!("Container {container_id} still exists, waiting...");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Err(DockerError::DockerResponseServerError {
                    status_code: 404, ..
                }) => {
                    gone = true;
                    break;
                }
                Err(e) => {
                    error!("Failed to inspect container {container_id}: {e}");
                    break;
                }
            }
        }

        if !gone {
            error!("Container {container_id} still exists after waiting");
        }

        // --- Step 3: Remove volume with retries ---
        if let Some(container) = container {
            let trimmed_name = container.names.first().unwrap().trim_start_matches('/');
            let volume_name = format!("{trimmed_name}_data");

            for attempt in 0..max_retries {
                match self.docker.remove_volume(&volume_name, None).await {
                    Ok(_) => {
                        info!("Volume {volume_name} removed successfully");
                        break;
                    }
                    Err(DockerError::DockerResponseServerError {
                        status_code: 404, ..
                    }) => {
                        debug!("Volume {volume_name} already removed");
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
                        error!("Failed to remove volume {volume_name}: {e}");
                        break;
                    }
                }
            }

            // --- Step 4: Check if other containers with same task ID exist before removing directory ---
            let should_remove_directory = if let Ok(task_container) =
                TaskContainer::from_str(trimmed_name)
            {
                // Check if there are other containers with the same task ID
                match self.list_containers(true).await {
                    Ok(containers) => {
                        let other_containers_with_same_task = containers.iter().any(|c| {
                            c.names.iter().any(|name| {
                                let clean_name = name.trim_start_matches('/');
                                if let Ok(other_task_container) = TaskContainer::from_str(name) {
                                    // Same task ID but different container (not the one being removed)
                                    other_task_container.task_id == task_container.task_id
                                        && clean_name != trimmed_name
                                } else {
                                    false
                                }
                            })
                        });

                        if other_containers_with_same_task {
                            info!(
                                "Other containers with task ID {} exist, keeping shared directory",
                                task_container.task_id
                            );
                            false
                        } else {
                            info!("No other containers with task ID {} found, safe to remove directory", task_container.task_id);
                            true
                        }
                    }
                    Err(e) => {
                        error!("Failed to list containers for cleanup check: {e}");
                        // Err on the side of caution - don't remove directory if we can't check
                        false
                    }
                }
            } else {
                // If we can't extract task ID, use original behavior
                true
            };

            if should_remove_directory {
                let dir_name = if let Ok(task_container) = TaskContainer::from_str(trimmed_name) {
                    task_container.data_dir_name()
                } else {
                    trimmed_name.to_string()
                };

                match self.safe_storage_path(&[&dir_name]) {
                    Ok(dir_path) => {
                        // Check if directory exists before attempting to remove it
                        if dir_path.exists() {
                            let mut success = false;

                            for attempt in 0..max_retries {
                                match std::fs::remove_dir_all(&dir_path) {
                                    Ok(_) => {
                                        info!(
                                            "Directory {} removed successfully",
                                            dir_path.display()
                                        );
                                        success = true;
                                        break;
                                    }
                                    Err(e) => {
                                        debug!(
                                            "Attempt {}/{} failed to remove dir {}: {}",
                                            attempt + 1,
                                            max_retries,
                                            dir_path.display(),
                                            e
                                        );
                                        tokio::time::sleep(Duration::from_secs(1)).await;
                                    }
                                }
                            }

                            if !success {
                                error!(
                                    "Failed to remove directory {} after {} attempts — trying fallback",
                                    dir_path.display(), max_retries
                                );

                                // Try `rm -rf` as fallback
                                match std::process::Command::new("rm")
                                    .arg("-rf")
                                    .arg(&dir_path)
                                    .status()
                                {
                                    Ok(status) if status.success() => {
                                        info!(
                                            "Fallback removal of {} succeeded",
                                            dir_path.display()
                                        );
                                    }
                                    Ok(status) => {
                                        error!("Fallback rm -rf failed with status {status}");
                                    }
                                    Err(e) => {
                                        error!("Failed to execute fallback rm -rf: {e}");
                                    }
                                }
                            }
                        } else {
                            debug!(
                                "Directory {} does not exist, skipping removal",
                                dir_path.display()
                            );
                        }
                    }
                    Err(e) => {
                        error!("Failed to create secure path for directory removal: {e}");
                    }
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
        debug!("Getting details for container: {container_id}");
        let container = self.docker.inspect_container(container_id, None).await?;
        let state = container.state.clone();

        let info = ContainerDetails {
            id: container.id.unwrap_or_default(),
            image: container.image.unwrap_or_default(),
            status: state.as_ref().and_then(|s| s.status),
            status_code: state.as_ref().and_then(|s| s.exit_code),
            names: vec![container.name.unwrap_or_default()],
            created: container
                .created
                .and_then(|c| c.parse::<i64>().ok())
                .unwrap_or_default(),
        };

        debug!("Retrieved details for container {container_id}");
        Ok(info)
    }

    pub async fn restart_container(&self, container_id: &str) -> Result<(), DockerError> {
        debug!("Restarting container: {container_id}");
        self.docker.restart_container(container_id, None).await?;
        debug!("Container {container_id} restarted successfully");
        Ok(())
    }

    pub async fn get_container_logs(
        &self,
        container_id: &str,
        tail: Option<i64>,
    ) -> Result<String, DockerError> {
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
                    error!("Error getting logs: {e}");
                    return Err(e);
                }
            }
        }
        // Push any leftover buffered line.
        if !current_line.is_empty() {
            all_logs.push(current_line);
        }
        let logs = all_logs.join("\n");
        debug!("Successfully retrieved logs for container {container_id}");
        Ok(logs)
    }

    pub async fn inspect_container(
        &self,
        container_id: &str,
    ) -> Result<bollard::models::ContainerInspectResponse, DockerError> {
        self.docker
            .inspect_container(container_id, Some(InspectContainerOptions { size: false }))
            .await
    }
}
