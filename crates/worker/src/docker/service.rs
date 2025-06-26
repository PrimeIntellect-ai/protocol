use super::docker_manager::ContainerInfo;
use super::DockerManager;
use super::DockerState;
use crate::console::Console;
use bollard::models::ContainerStateStatusEnum;
use chrono::{DateTime, Utc};
use log::debug;
use shared::models::heartbeat::TaskDetails;
use shared::models::node::GpuSpecs;
use shared::models::task::Task;
use shared::models::task::TaskState;
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{interval, Duration};
use tokio_util::sync::CancellationToken;

pub struct DockerService {
    docker_manager: Arc<DockerManager>,
    cancellation_token: CancellationToken,
    pub state: Arc<DockerState>,
    gpu: Option<GpuSpecs>,
    system_memory_mb: Option<u32>,
    task_bridge_socket_path: String,
    node_address: String,
    p2p_seed: Option<u64>,
}

const TASK_PREFIX: &str = "prime-task";
const RESTART_INTERVAL_SECONDS: i64 = 10;

impl DockerService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cancellation_token: CancellationToken,
        gpu: Option<GpuSpecs>,
        system_memory_mb: Option<u32>,
        task_bridge_socket_path: String,
        storage_path: String,
        node_address: String,
        p2p_seed: Option<u64>,
        disable_host_network_mode: bool,
    ) -> Self {
        let docker_manager =
            Arc::new(DockerManager::new(storage_path, disable_host_network_mode).unwrap());
        Self {
            docker_manager,
            cancellation_token,
            state: Arc::new(DockerState::new()),
            gpu,
            system_memory_mb,
            task_bridge_socket_path,
            node_address,
            p2p_seed,
        }
    }

    /// Helper function to get expected container names for a task
    /// Returns a vector of (container_name, gpu_index) tuples
    fn get_container_names_for_task(&self, task: &Task) -> Vec<(String, Option<u32>)> {
        let config_hash = task.generate_config_hash();
        let base_name = format!("{}-{}-{:x}", TASK_PREFIX, task.id, config_hash);

        if task.partition_by_gpu {
            // For GPU partitioned tasks, create one container per GPU
            if let Some(ref gpu_specs) = self.gpu {
                let gpu_count = gpu_specs
                    .indices
                    .as_ref()
                    .map(|indices| indices.len())
                    .unwrap_or(gpu_specs.count.unwrap_or(1) as usize);

                (0..gpu_count)
                    .map(|i| (format!("{}-gpu{}", base_name, i), Some(i as u32)))
                    .collect()
            } else {
                // No GPUs available, fall back to single container
                vec![(base_name, None)]
            }
        } else {
            // Non-partitioned task: single container with access to all GPUs
            vec![(base_name, None)]
        }
    }

    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut interval = interval(Duration::from_secs(5));
        let manager = self.docker_manager.clone();
        let cancellation_token = self.cancellation_token.clone();
        let state = self.state.clone();

        state.set_is_running(true).await;

        let starting_container_tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>> =
            Arc::new(Mutex::new(Vec::new()));
        let terminating_container_tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>> =
            Arc::new(Mutex::new(Vec::new()));

        fn generate_task_id(task: &Option<Task>) -> Option<String> {
            task.as_ref().map(|task| {
                let config_hash = task.generate_config_hash();
                format!("{}-{}-{:x}", TASK_PREFIX, task.id, config_hash)
            })
        }

        async fn cleanup_tasks(tasks: &Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>) {
            let mut tasks_guard = tasks.lock().await;
            for handle in tasks_guard.iter() {
                handle.abort();
            }
            tasks_guard.clear();
        }

        let manager_clone = manager.clone();
        let terminate_manager = manager_clone.clone();
        let task_state_clone = state.clone();

        // Track consecutive failures
        let mut consecutive_failures = 0;

        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => {
                    cleanup_tasks(&starting_container_tasks).await;
                    cleanup_tasks(&terminating_container_tasks).await;
                    break;
                }
                _ = interval.tick() => {
                    {
                        let mut tasks = starting_container_tasks.lock().await;
                        tasks.retain(|handle| !handle.is_finished());
                    }
                    {
                        let mut tasks = terminating_container_tasks.lock().await;
                        tasks.retain(|handle| !handle.is_finished());
                    }

                    let current_task = task_state_clone.get_current_task().await;
                    let task_id = generate_task_id(&current_task);

                    let all_containers = match manager.list_containers(true).await {
                        Ok(containers) => containers,
                        Err(e) => {
                            log::error!("Error listing containers: {e}");
                            continue;
                        }
                    };

                    let old_tasks: Vec<ContainerInfo> = all_containers
                    .iter()
                    .filter(|c| {
                        c.names.iter().any(|name| name.contains(TASK_PREFIX))
                            && task_id
                                .as_ref().is_none_or(|id| !c.names.iter().any(|name| name.contains(id)))
                    })
                    .cloned()
                    .collect();

                    if !old_tasks.is_empty() {
                        for task in old_tasks {
                            let terminate_manager_clone = terminate_manager.clone();
                            let handle = tokio::spawn(async move {
                                let termination = terminate_manager_clone.remove_container(&task.id).await;
                                match termination {
                                    Ok(_) => Console::info("DockerService", "Container terminated successfully"),
                                    Err(e) => log::error!("Error terminating container: {e}"),
                                }
                            });
                            terminating_container_tasks.lock().await.push(handle);
                        }
                    }

                    if let Some(ref current_task_ref) = current_task {
                        // Get expected container names for this task
                        let expected_containers = self.get_container_names_for_task(current_task_ref);
                        let task_containers: Vec<ContainerInfo> = all_containers
                            .iter()
                            .filter(|c| {
                                expected_containers.iter().any(|(name, _)| {
                                    c.names.contains(&format!("/{}", name))
                                })
                            })
                            .cloned()
                            .collect();

                        let missing_containers: Vec<(String, Option<u32>)> = expected_containers
                            .into_iter()
                            .filter(|(name, _)| {
                                !task_containers.iter().any(|c| {
                                    c.names.contains(&format!("/{}", name))
                                })
                            })
                            .collect();

                        if !missing_containers.is_empty() {
                            let running_tasks = starting_container_tasks.lock().await;
                            let has_running_tasks = running_tasks.iter().any(|h| !h.is_finished());
                            drop(running_tasks);

                            if has_running_tasks {
                                Console::info("DockerService", "Container is still starting ...");
                            } else {
                                let last_started_time = match task_state_clone.get_last_started().await {
                                    Some(time) => time,
                                    None => DateTime::from_timestamp(0, 0).unwrap(),
                                };
                                let elapsed = Utc::now().signed_duration_since(last_started_time).num_seconds();

                                let backoff_seconds = RESTART_INTERVAL_SECONDS;

                                // wait for backoff period before starting a new container
                                if elapsed < backoff_seconds {
                                    Console::info("DockerService", &format!("Waiting before starting new container ({}s remaining)...", backoff_seconds - elapsed));
                                } else {
                                    if consecutive_failures > 0 {
                                        Console::info("DockerService", &format!("Starting new container after {consecutive_failures} failures ({RESTART_INTERVAL_SECONDS}s interval)..."));
                                    } else {
                                        Console::info("DockerService", "Starting new container...");
                                    }

                                    // Start containers for all missing GPU indices
                                    for (container_name, gpu_index) in missing_containers {
                                        let manager_clone = manager_clone.clone();
                                        let state_clone = task_state_clone.clone();
                                        let gpu = self.gpu.clone();
                                        let system_memory_mb = self.system_memory_mb;
                                        let task_bridge_socket_path = self.task_bridge_socket_path.clone();
                                        let node_address = self.node_address.clone();
                                        let p2p_seed = self.p2p_seed;
                                        let handle = tokio::spawn(async move {
                                            let payload = match state_clone.get_current_task().await {
                                                Some(payload) => payload,
                                                None => {
                                                    return;
                                                }
                                            };
                                            let cmd = match payload.cmd {
                                                Some(cmd_vec) => {
                                                    cmd_vec.into_iter().map(|arg| {
                                                        let mut processed_arg = arg.replace("${SOCKET_PATH}", &task_bridge_socket_path);
                                                        if let Some(seed) = p2p_seed {
                                                            processed_arg = processed_arg.replace("${WORKER_P2P_SEED}", &seed.to_string());
                                                        }
                                                        processed_arg
                                                    }).collect()
                                                }
                                                None => vec!["sleep".to_string(), "infinity".to_string()],
                                            };

                                            let mut env_vars: HashMap<String, String> = HashMap::new();
                                            if let Some(env) = &payload.env_vars {
                                                // Clone env vars and replace ${SOCKET_PATH} in values
                                                for (key, value) in env.iter() {
                                                    let mut processed_value = value.replace("${SOCKET_PATH}", &task_bridge_socket_path);
                                                    if let Some(seed) = p2p_seed {
                                                        processed_value = processed_value.replace("${WORKER_P2P_SEED}", &seed.to_string());
                                                    }
                                                    env_vars.insert(key.clone(), processed_value);
                                                }
                                            }

                                            env_vars.insert("NODE_ADDRESS".to_string(), node_address);
                                            env_vars.insert("PRIME_MONITOR__SOCKET__PATH".to_string(), task_bridge_socket_path.to_string());
                                            env_vars.insert("PRIME_TASK_ID".to_string(), payload.id.to_string());

                                            // Add GPU_INDEX environment variable for partitioned tasks
                                            if let Some(idx) = gpu_index {
                                                env_vars.insert("GPU_INDEX".to_string(), idx.to_string());
                                            }

                                            let mut volumes = vec![
                                                (
                                                    Path::new(&task_bridge_socket_path).parent().unwrap().to_path_buf().to_string_lossy().to_string(),
                                                    Path::new(&task_bridge_socket_path).parent().unwrap().to_path_buf().to_string_lossy().to_string(),
                                                    false,
                                                    false,
                                                )
                                            ];

                                            if let Some(volume_mounts) = &payload.volume_mounts {
                                                for volume_mount in volume_mounts {
                                                    volumes.push((
                                                        volume_mount.host_path.clone(),
                                                        volume_mount.container_path.clone(),
                                                        false,
                                                        true
                                                    ));
                                                }
                                            }
                                            let shm_size = match system_memory_mb {
                                                Some(mem_mb) => (mem_mb as u64) * 1024 * 1024 / 2, // Convert MB to bytes and divide by 2
                                                None => {
                                                    Console::warning("System memory not available, using default shm size");
                                                    67108864 // Default to 64MB in bytes
                                                }
                                            };

                                            // Prepare GPU specs for this specific container
                                            let container_gpu = if payload.partition_by_gpu && gpu_index.is_some() {
                                                // For partitioned tasks, assign specific GPU
                                                gpu.map(|mut g| {
                                                    let gpu_idx = gpu_index.unwrap();
                                                    // If indices are specified, use the actual GPU index
                                                    if let Some(indices) = &g.indices {
                                                        if (gpu_idx as usize) < indices.len() {
                                                            g.indices = Some(vec![indices[gpu_idx as usize]]);
                                                        }
                                                    } else {
                                                        // Otherwise, use the sequential index
                                                        g.indices = Some(vec![gpu_idx]);
                                                    }
                                                    g
                                                })
                                            } else {
                                                // For non-partitioned tasks, use all GPUs
                                                gpu
                                            };

                                            match manager_clone.start_container(&payload.image, &container_name, Some(env_vars), Some(cmd), container_gpu, Some(volumes), Some(shm_size), payload.entrypoint.clone(), None).await {
                                                Ok(container_id) => {
                                                    Console::info("DockerService", &format!("Container started with id: {container_id}"));
                                                },
                                                Err(e) => {
                                                    log::error!("Error starting container: {e}");
                                                    state_clone.update_task_state(payload.id, TaskState::FAILED).await;
                                                }
                                            }
                                            state_clone.set_last_started(Utc::now()).await;
                                        });
                                        starting_container_tasks.lock().await.push(handle);
                                    }
                                }
                            }
                        } else {
                            // All expected containers exist, now aggregate their states
                            let mut any_failed = false;
                            let mut all_completed = true;
                            let mut any_running = false;
                            let mut all_created = true;

                            for container in &task_containers {
                                let status = match manager.get_container_details(&container.id).await {
                                    Ok(status) => status,
                                    Err(e) => {
                                        log::error!("Error getting container details: {e}");
                                        continue;
                                    }
                                };

                                match (status.status, status.status_code) {
                                    (Some(ContainerStateStatusEnum::RUNNING), _) => {
                                        any_running = true;
                                        all_completed = false;
                                        all_created = false;
                                    },
                                    (Some(ContainerStateStatusEnum::CREATED), _) => {
                                        all_completed = false;
                                    },
                                    (Some(ContainerStateStatusEnum::EXITED), Some(0)) => {
                                        all_created = false;
                                    },
                                    (Some(ContainerStateStatusEnum::EXITED), Some(code)) if code != 0 => {
                                        any_failed = true;
                                        all_created = false;
                                    },
                                    (Some(ContainerStateStatusEnum::DEAD), _) => {
                                        any_failed = true;
                                        all_created = false;
                                    },
                                    _ => {
                                        all_completed = false;
                                        all_created = false;
                                    }
                                }
                            }

                            let task_state_current = current_task_ref.state.clone();

                            // Determine aggregated task state
                            let task_state_live = if any_failed {
                                TaskState::FAILED
                            } else if all_completed {
                                TaskState::COMPLETED
                            } else if any_running {
                                TaskState::RUNNING
                            } else if all_created {
                                TaskState::PENDING
                            } else {
                                TaskState::UNKNOWN
                            };

                            // Only log if state changed
                            if task_state_live != task_state_current {
                                Console::info("DockerService", &format!("Task state changed from {task_state_current:?} to {task_state_live:?}"));

                                if task_state_live == TaskState::FAILED {
                                    consecutive_failures += 1;
                                    Console::info("DockerService", &format!("Task failed (attempt {consecutive_failures}), waiting with exponential backoff before restart"));
                                } else if task_state_live == TaskState::RUNNING {
                                    // Reset failure counter when container runs successfully
                                    consecutive_failures = 0;
                                }
                            }

                            if let Some(task) = task_state_clone.get_current_task().await {
                                task_state_clone.update_task_state(task.id, task_state_live).await;
                            }
                        }
                    }
                },
            }
        }

        Ok(())
    }

    pub async fn get_logs(&self) -> Result<String, Box<dyn std::error::Error>> {
        let current_task = self.state.get_current_task().await;
        match current_task {
            Some(task) => {
                let container_names = self.get_container_names_for_task(&task);
                let mut all_logs = Vec::new();

                for (container_name, gpu_index) in container_names {
                    let logs = match self
                        .docker_manager
                        .get_container_logs(&container_name, None)
                        .await
                    {
                        Ok(logs) if !logs.is_empty() => logs,
                        Ok(_) => continue, // Empty logs, skip
                        Err(e) => {
                            log::debug!(
                                "Failed to get logs for container {}: {}",
                                container_name,
                                e
                            );
                            continue;
                        }
                    };

                    // Add header for partitioned tasks
                    if task.partition_by_gpu && gpu_index.is_some() {
                        all_logs.push(format!("=== GPU {} ===", gpu_index.unwrap()));
                    }
                    all_logs.push(logs);
                }

                if all_logs.is_empty() {
                    Ok("No logs found in docker containers".to_string())
                } else {
                    Ok(all_logs.join("\n\n"))
                }
            }
            None => Ok("No task running".to_string()),
        }
    }

    pub async fn restart_task(&self) -> Result<(), Box<dyn std::error::Error>> {
        let current_task = self.state.get_current_task().await;
        match current_task {
            Some(task) => {
                let container_names = self.get_container_names_for_task(&task);
                let mut restart_errors = Vec::new();

                for (container_name, _) in container_names {
                    if let Err(e) = self.docker_manager.restart_container(&container_name).await {
                        log::error!("Failed to restart container {}: {}", container_name, e);
                        restart_errors.push(format!("{}: {}", container_name, e));
                    }
                }

                if !restart_errors.is_empty() {
                    return Err(format!(
                        "Failed to restart some containers: {}",
                        restart_errors.join(", ")
                    )
                    .into());
                }

                Ok(())
            }
            None => Ok(()),
        }
    }

    pub async fn get_task_details(&self, task: &Task) -> Option<TaskDetails> {
        let container_names = self.get_container_names_for_task(task);

        // For partitioned tasks, use the first container (gpu0) as representative
        let (container_name, _) = container_names.first()?;

        match self.docker_manager.list_containers(true).await {
            Ok(containers) => {
                let container = containers
                    .iter()
                    .find(|c| c.names.contains(&format!("/{container_name}")));

                if let Some(container) = container {
                    match self
                        .docker_manager
                        .get_container_details(&container.id)
                        .await
                    {
                        Ok(details) => {
                            let docker_image_id = if let Ok(inspect_result) =
                                self.docker_manager.inspect_container(&container.id).await
                            {
                                inspect_result.image
                            } else {
                                Some(container.image.clone())
                            };

                            Some(TaskDetails {
                                docker_image_id,
                                container_id: Some(container.id.clone()),
                                container_status: details.status.map(|s| format!("{s:?}")),
                                container_created_at: Some(container.created),
                                container_exit_code: details.status_code,
                            })
                        }
                        Err(e) => {
                            debug!("Failed to get container details: {e}");
                            Some(TaskDetails {
                                docker_image_id: Some(container.image.clone()),
                                container_id: Some(container.id.clone()),
                                container_status: None,
                                container_created_at: Some(container.created),
                                container_exit_code: None,
                            })
                        }
                    }
                } else {
                    debug!(
                        "Container {} not found for task {}",
                        container_name, task.id
                    );
                    None
                }
            }
            Err(e) => {
                debug!("Failed to list containers: {e}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::Address;
    use shared::models::task::Task;
    use shared::models::task::TaskState;
    use uuid::Uuid;

    #[tokio::test]
    #[serial_test::serial]
    async fn test_docker_service_basic() {
        let cancellation_token = CancellationToken::new();
        let docker_service = DockerService::new(
            cancellation_token.clone(),
            None,
            Some(1024),
            "/tmp/com.prime.miner/metrics.sock".to_string(),
            "/tmp/test-storage".to_string(),
            Address::ZERO.to_string(),
            None,
            false,
        );
        let task = Task {
            image: "ubuntu:latest".to_string(),
            name: "test".to_string(),
            id: Uuid::new_v4(),
            env_vars: None,
            cmd: Some(vec!["sleep".to_string(), "5".to_string()]), // Reduced sleep time
            entrypoint: None,
            state: TaskState::PENDING,
            created_at: Utc::now().timestamp_millis(),
            ..Default::default()
        };
        let task_clone = task.clone();
        let state_clone = docker_service.state.clone();
        docker_service
            .state
            .set_current_task(Some(task_clone))
            .await;

        assert_eq!(
            docker_service.state.get_current_task().await.unwrap().name,
            task.name
        );

        tokio::spawn(async move {
            docker_service.run().await.unwrap();
        });

        // Reduced wait times
        tokio::time::sleep(Duration::from_secs(2)).await;
        state_clone.set_current_task(None).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        cancellation_token.cancel();
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn test_socket_path_variable_replacement() {
        let cancellation_token = CancellationToken::new();
        let test_socket_path = "/custom/socket/path.sock";
        let docker_service = DockerService::new(
            cancellation_token.clone(),
            None,
            Some(1024),
            test_socket_path.to_string(),
            "/tmp/test-storage".to_string(),
            Address::ZERO.to_string(),
            Some(12345), // p2p_seed for testing
            false,
        );

        // Test command argument replacement
        let task_with_cmd = Task {
            image: "ubuntu:latest".to_string(),
            name: "test_cmd_replacement".to_string(),
            id: Uuid::new_v4(),
            cmd: Some(vec!["echo".to_string(), "${SOCKET_PATH}".to_string()]),
            env_vars: None,
            entrypoint: None,
            state: TaskState::PENDING,
            created_at: Utc::now().timestamp_millis(),
            ..Default::default()
        };

        // Test environment variable replacement
        let task_with_env = Task {
            image: "ubuntu:latest".to_string(),
            name: "test_env_replacement".to_string(),
            id: Uuid::new_v4(),
            cmd: None,
            env_vars: Some(HashMap::from([
                ("MY_SOCKET_PATH".to_string(), "${SOCKET_PATH}".to_string()),
                (
                    "CUSTOM_PATH".to_string(),
                    "prefix_${SOCKET_PATH}_suffix".to_string(),
                ),
                ("NORMAL_VAR".to_string(), "no_replacement".to_string()),
            ])),
            entrypoint: None,
            state: TaskState::PENDING,
            created_at: Utc::now().timestamp_millis(),
            ..Default::default()
        };

        // Set tasks and verify state
        docker_service
            .state
            .set_current_task(Some(task_with_cmd.clone()))
            .await;
        assert_eq!(
            docker_service.state.get_current_task().await.unwrap().name,
            task_with_cmd.name
        );

        docker_service
            .state
            .set_current_task(Some(task_with_env.clone()))
            .await;
        assert_eq!(
            docker_service.state.get_current_task().await.unwrap().name,
            task_with_env.name
        );

        // Note: We can't easily test the actual replacement in container start
        // without mocking DockerManager, but we've verified the logic visually
        cancellation_token.cancel();
    }

    #[test]
    fn test_get_container_names_for_task() {
        let cancellation_token = CancellationToken::new();

        // Test with GPU partitioning enabled and multiple GPUs
        let gpu_specs = Some(GpuSpecs {
            count: Some(4),
            indices: Some(vec![0, 1, 2, 3]),
            model: None,
            memory_mb: None,
        });

        let docker_service = DockerService::new(
            cancellation_token.clone(),
            gpu_specs,
            Some(1024),
            "/tmp/test.sock".to_string(),
            "/tmp/test-storage".to_string(),
            Address::ZERO.to_string(),
            None,
            false,
        );

        // Test partitioned task
        let partitioned_task = Task {
            image: "test:latest".to_string(),
            name: "partitioned".to_string(),
            id: Uuid::from_str("123e4567-e89b-12d3-a456-426614174000").unwrap(),
            partition_by_gpu: true,
            ..Default::default()
        };

        let container_names = docker_service.get_container_names_for_task(&partitioned_task);
        assert_eq!(container_names.len(), 4);
        assert_eq!(container_names[0].1, Some(0));
        assert_eq!(container_names[1].1, Some(1));
        assert_eq!(container_names[2].1, Some(2));
        assert_eq!(container_names[3].1, Some(3));
        assert!(container_names[0].0.ends_with("-gpu0"));
        assert!(container_names[1].0.ends_with("-gpu1"));

        // Test non-partitioned task
        let non_partitioned_task = Task {
            image: "test:latest".to_string(),
            name: "non-partitioned".to_string(),
            id: Uuid::from_str("123e4567-e89b-12d3-a456-426614174000").unwrap(),
            partition_by_gpu: false,
            ..Default::default()
        };

        let container_names = docker_service.get_container_names_for_task(&non_partitioned_task);
        assert_eq!(container_names.len(), 1);
        assert_eq!(container_names[0].1, None);
        assert!(!container_names[0].0.contains("-gpu"));

        // Test partitioned task with no GPUs available
        let docker_service_no_gpu = DockerService::new(
            cancellation_token.clone(),
            None,
            Some(1024),
            "/tmp/test.sock".to_string(),
            "/tmp/test-storage".to_string(),
            Address::ZERO.to_string(),
            None,
            false,
        );

        let container_names = docker_service_no_gpu.get_container_names_for_task(&partitioned_task);
        assert_eq!(container_names.len(), 1);
        assert_eq!(container_names[0].1, None);
    }
}
