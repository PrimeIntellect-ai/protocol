use super::docker_manager::ContainerInfo;
use super::DockerManager;
use super::DockerState;
use crate::console::Console;
use bollard::models::ContainerStateStatusEnum;
use chrono::{DateTime, Utc};
use shared::models::task::Task;
use shared::models::task::TaskState;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{interval, Duration};
use tokio_util::sync::CancellationToken;

pub struct DockerService {
    docker_manager: Arc<DockerManager>,
    cancellation_token: CancellationToken,
    pub state: Arc<DockerState>,
    has_gpu: bool,
    system_memory_mb: Option<u32>,
    task_bridge_socket_path: String,
}

const TASK_PREFIX: &str = "prime-task";

impl DockerService {
    pub fn new(
        cancellation_token: CancellationToken,
        has_gpu: bool,
        system_memory_mb: Option<u32>,
        task_bridge_socket_path: String,
        storage_path: Option<String>,
    ) -> Self {
        let docker_manager = Arc::new(DockerManager::new(storage_path).unwrap());
        Self {
            docker_manager,
            cancellation_token,
            state: Arc::new(DockerState::new()),
            has_gpu,
            system_memory_mb,
            task_bridge_socket_path,
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

        pub fn generate_task_id(task: &Option<Task>) -> Option<String> {
            task.as_ref()
                .map(|task| format!("{}-{}", TASK_PREFIX, task.id))
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

                    let all_containers = manager.list_containers(true).await.unwrap();

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
                                    Err(e) => Console::error(&format!("Error terminating container: {}", e)),
                                }
                            });
                            terminating_container_tasks.lock().await.push(handle);
                        }
                    }

                    if current_task.is_some() && task_id.is_some() {
                        let container_task_id = format!("{}-{}", TASK_PREFIX, current_task.unwrap().id);
                        let container_match = all_containers.iter().find(|c| c.names.contains(&format!("/{}", container_task_id)));
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
                                // wait 15 seconds before starting a new container
                                if elapsed < 15 {
                                    Console::info("DockerService", "Waiting before starting new container ...");
                                } else {
                                    Console::info("DockerService", "Starting new container ...");
                                    let manager_clone = manager_clone.clone();
                                    let state_clone = task_state_clone.clone();
                                    let has_gpu = self.has_gpu;
                                    let system_memory_mb = self.system_memory_mb;
                                    let task_bridge_socket_path = self.task_bridge_socket_path.clone();
                                    let handle = tokio::spawn(async move {
                                        let payload = state_clone.get_current_task().await.unwrap();
                                        let cmd_full = (payload.command, payload.args);
                                        let cmd = match cmd_full {
                                            (Some(c), Some(a)) => {
                                                let mut cmd = vec![c];
                                                cmd.extend(a);
                                                cmd
                                            }
                                            (Some(c), None) => vec![c],
                                            _ => vec!["sleep".to_string(), "infinity".to_string()],
                                        };

                                        let mut env_vars: HashMap<String, String> = HashMap::new();
                                        if let Some(env) = &payload.env_vars {
                                            env_vars.extend(env.clone());
                                        }

                                        env_vars.insert("PRIME_TASK_BRIDGE_SOCKET".to_string(), task_bridge_socket_path.to_string());
                                        env_vars.insert("PRIME_TASK_ID".to_string(), payload.id.to_string());
                                        let volumes = vec![
                                            (
                                                Path::new(&task_bridge_socket_path).parent().unwrap().to_path_buf().to_string_lossy().to_string(),
                                                Path::new(&task_bridge_socket_path).parent().unwrap().to_path_buf().to_string_lossy().to_string(),
                                                false,
                                            )
                                        ];
                                        let shm_size = match system_memory_mb {
                                            Some(mem_mb) => (mem_mb as u64) * 1024 * 1024 / 2, // Convert MB to bytes and divide by 2
                                            None => {
                                                Console::warning("System memory not available, using default shm size");
                                                67108864 // Default to 64MB in bytes
                                            }
                                        };
                                        match manager_clone.start_container(&payload.image, &container_task_id, Some(env_vars), Some(cmd), has_gpu, Some(volumes), Some(shm_size)).await {
                                            Ok(container_id) => {
                                                Console::info("DockerService", &format!("Container started with id: {}", container_id));
                                            },
                                            Err(e) => {
                                                Console::error(&format!("Error starting container: {}", e));
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
                            let status = manager.get_container_details(&container_status.id).await.unwrap();

                            let task_state_current = task_state_clone.get_current_task().await.unwrap().state;
                            // handle edge case where container instantly dies due to invalid command
                            if status.status == Some(ContainerStateStatusEnum::CREATED) && task_state_current == TaskState::FAILED {
                                Console::info("DockerService", "Task failed, waiting for new command from manager ...");
                            } else {
                                let task_state_live = match status.status {
                                    Some(ContainerStateStatusEnum::RUNNING) => TaskState::RUNNING,
                                    Some(ContainerStateStatusEnum::CREATED) => TaskState::PENDING,
                                    Some(ContainerStateStatusEnum::EXITED) => TaskState::COMPLETED,
                                    Some(ContainerStateStatusEnum::DEAD) => TaskState::FAILED,
                                    Some(ContainerStateStatusEnum::PAUSED) => TaskState::PAUSED,
                                    Some(ContainerStateStatusEnum::RESTARTING) => TaskState::RESTARTING,
                                    Some(ContainerStateStatusEnum::REMOVING) => TaskState::UNKNOWN,
                                    _ => TaskState::UNKNOWN,
                                };

                                // Only log if state changed
                                if task_state_live != task_state_current {
                                    Console::info("DockerService", &format!("Task state changed from {:?} to {:?}", task_state_current, task_state_live));

                                    if task_state_live == TaskState::FAILED {
                                        Console::info("DockerService", "Task failed, waiting for new command from manager and restarting container");
                                        let terminate_manager_clone = terminate_manager.clone();
                                        let handle = tokio::spawn(async move {
                                            let termination = terminate_manager_clone.remove_container(&container_status.id).await;
                                            match termination {
                                                Ok(_) => Console::info("DockerService", "Container terminated successfully"),
                                                Err(e) => Console::error(&format!("Error terminating container: {}", e)),
                                            }
                                        });
                                        terminating_container_tasks.lock().await.push(handle);
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

    pub async fn get_logs(&self) -> Result<String, Box<dyn std::error::Error>> {
        let current_task = self.state.get_current_task().await;
        match current_task {
            Some(task) => {
                let container_id = format!("{}-{}", TASK_PREFIX, task.id);
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

    pub async fn restart_task(&self) -> Result<(), Box<dyn std::error::Error>> {
        let current_task = self.state.get_current_task().await;
        match current_task {
            Some(task) => {
                let container_id = format!("{}-{}", TASK_PREFIX, task.id);
                self.docker_manager.restart_container(&container_id).await?;
                Ok(())
            }
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::models::task::Task;
    use shared::models::task::TaskState;
    use uuid::Uuid;

    #[tokio::test]
    #[serial_test::serial]
    async fn test_docker_service() {
        let cancellation_token = CancellationToken::new();
        let docker_service = DockerService::new(
            cancellation_token.clone(),
            false,
            Some(1024),
            "/tmp/com.prime.worker/metrics.sock".to_string(),
            None,
        );
        let task = Task {
            image: "ubuntu:latest".to_string(),
            name: "test".to_string(),
            id: Uuid::new_v4(),
            env_vars: None,
            command: Some("sleep".to_string()),
            args: Some(vec!["100".to_string()]),
            state: TaskState::PENDING,
        };
        let task_clone = task.clone();
        let state_clone = docker_service.state.clone();
        docker_service
            .state
            .set_current_task(Some(task_clone))
            .await;
        let task_name = task.name.to_string();
        assert_eq!(
            docker_service.state.get_current_task().await.unwrap().name,
            task_name
        );

        tokio::spawn(async move {
            docker_service.run().await.unwrap();
        });
        tokio::time::sleep(Duration::from_secs(10)).await;
        state_clone.set_current_task(None).await;
        tokio::time::sleep(Duration::from_secs(10)).await;
        Console::info("DockerService", "Cancelling cancellation token");
        cancellation_token.cancel();
        tokio::time::sleep(Duration::from_secs(10)).await;
        Console::info("DockerService", "Cancelling done");
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn test_docker_service_idle_on_failure() {
        let cancellation_token = CancellationToken::new();
        let docker_service = DockerService::new(
            cancellation_token.clone(),
            false,
            Some(1024),
            "/tmp/com.prime.worker/metrics.sock".to_string(),
            None,
        );
        let state = docker_service.state.clone();

        // Create task that will fail
        let task = Task {
            image: "ubuntu:latest".to_string(),
            name: "test-restart".to_string(),
            id: Uuid::new_v4(),
            env_vars: None,
            command: Some("invalid_command".to_string()),
            args: None,
            state: TaskState::PENDING,
        };

        let task_clone = task.clone();
        let state_clone = docker_service.state.clone();
        docker_service
            .state
            .set_current_task(Some(task_clone))
            .await;
        let task_name = task.name.to_string();
        assert_eq!(
            docker_service.state.get_current_task().await.unwrap().name,
            task_name
        );
        tokio::spawn(async move {
            docker_service.run().await.unwrap();
        });

        // Wait for initial container start
        tokio::time::sleep(Duration::from_secs(5)).await;

        // Wait for container to fail and timeout period
        tokio::time::sleep(Duration::from_secs(20)).await;

        // Get current task state
        let current_task = state.get_current_task().await.unwrap();
        assert_eq!(current_task.state, TaskState::FAILED);

        // Verify new container was created after timeout
        let last_started = state.get_last_started().await;
        assert!(last_started.is_some());

        // Cleanup
        state_clone.set_current_task(None).await;
        tokio::time::sleep(Duration::from_secs(10)).await;
        cancellation_token.cancel();
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
