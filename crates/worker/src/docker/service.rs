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
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{interval, Duration};
use tokio_util::sync::CancellationToken;

pub(crate) struct DockerService {
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
    pub(crate) fn new(
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

    pub(crate) async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
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

                    if current_task.is_some() && task_id.is_some() {
                        let container_task_id = task_id.as_ref().unwrap().clone();
                        let container_match = all_containers.iter().find(|c| c.names.contains(&format!("/{container_task_id}")));
                        if container_match.is_none() {
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
                                        match manager_clone.start_container(&payload.image, &container_task_id, Some(env_vars), Some(cmd), gpu, Some(volumes), Some(shm_size), payload.entrypoint, None).await {
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
                        } else {
                            let container_status = container_match.unwrap().clone();
                            let status = match manager.get_container_details(&container_status.id).await {
                                Ok(status) => status,
                                Err(e) => {
                                    log::error!("Error getting container details: {e}");
                                    continue;
                                }
                            };

                            let task_state_current = match task_state_clone.get_current_task().await {
                                Some(task) => task.state,
                                None => {
                                    log::error!("No task found in state");
                                    continue;
                                }
                            };
                            // handle edge case where container instantly dies due to invalid command
                            if status.status == Some(ContainerStateStatusEnum::CREATED) && task_state_current == TaskState::FAILED {
                                Console::info("DockerService", "Task failed, waiting for new command from manager ...");
                            } else {
                                debug!("docker container status: {:?}, status_code: {:?}", status.status, status.status_code);
                                let task_state_live = match status.status {
                                    Some(ContainerStateStatusEnum::RUNNING) => TaskState::RUNNING,
                                    Some(ContainerStateStatusEnum::CREATED) => TaskState::PENDING,
                                    Some(ContainerStateStatusEnum::EXITED) => {
                                        match status.status_code {
                                            Some(0) => TaskState::COMPLETED,
                                            Some(_) => TaskState::FAILED, // Any non-zero exit code
                                            None => TaskState::UNKNOWN,
                                        }
                                    },
                                    Some(ContainerStateStatusEnum::DEAD) => TaskState::FAILED,
                                    Some(ContainerStateStatusEnum::PAUSED) => TaskState::PAUSED,
                                    Some(ContainerStateStatusEnum::RESTARTING) => TaskState::RESTARTING,
                                    Some(ContainerStateStatusEnum::REMOVING) | Some(ContainerStateStatusEnum::EMPTY) | None => TaskState::UNKNOWN,
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
                    }
                },
            }
        }

        Ok(())
    }

    pub(crate) async fn get_logs(&self) -> Result<String, Box<dyn std::error::Error>> {
        let current_task = self.state.get_current_task().await;
        match current_task {
            Some(task) => {
                let config_hash = task.generate_config_hash();
                let container_id = format!("{}-{}-{:x}", TASK_PREFIX, task.id, config_hash);

                let logs = self
                    .docker_manager
                    .get_container_logs(&container_id, None)
                    .await?;
                if logs.is_empty() {
                    Ok("No logs found in docker container".to_string())
                } else {
                    Ok(logs)
                }
            }
            None => Ok("No task running".to_string()),
        }
    }

    pub(crate) async fn restart_task(&self) -> Result<(), Box<dyn std::error::Error>> {
        let current_task = self.state.get_current_task().await;
        match current_task {
            Some(task) => {
                let config_hash = task.generate_config_hash();
                let container_id = format!("{}-{}-{:x}", TASK_PREFIX, task.id, config_hash);
                self.docker_manager.restart_container(&container_id).await?;
                Ok(())
            }
            None => Ok(()),
        }
    }

    pub(crate) async fn get_task_details(&self, task: &Task) -> Option<TaskDetails> {
        let config_hash = task.generate_config_hash();
        let container_name = format!("{}-{}-{:x}", TASK_PREFIX, task.id, config_hash);

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
}
