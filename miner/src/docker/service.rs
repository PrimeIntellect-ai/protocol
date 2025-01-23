use super::docker_manager::ContainerInfo;
use super::DockerManager;
use super::DockerState;
use crate::console::Console;
use bollard::models::ContainerStateStatusEnum;
use shared::models::task::Task;
use shared::models::task::TaskState;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{interval, Duration};
use tokio_util::sync::CancellationToken; // Importing the Console for logging

pub struct DockerService {
    docker_manager: Arc<DockerManager>,
    cancellation_token: CancellationToken,
    pub state: Arc<DockerState>,
}

const TASK_PREFIX: &str = "prime-task-";

impl DockerService {
    pub fn new(cancellation_token: CancellationToken) -> Self {
        let docker_manager = Arc::new(DockerManager::new().unwrap());
        Self {
            docker_manager,
            cancellation_token,
            state: Arc::new(DockerState::new()),
        }
    }

    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut interval = interval(Duration::from_secs(5));
        let manager = self.docker_manager.clone();
        let cancellation_token = self.cancellation_token.clone();
        let state = self.state.clone();

        let starting_container_tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>> =
            Arc::new(Mutex::new(Vec::new()));
        let terminating_container_tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>> =
            Arc::new(Mutex::new(Vec::new()));

        pub fn generate_task_id(task: &Option<Task>) -> Option<String> {
            task.as_ref()
                .map(|task| format!("{}-{}", task.id, TASK_PREFIX))
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

                    let current_task = state.get_current_task().await;
                    let task_id = generate_task_id(&current_task);
                    let task_clone = current_task.clone();

                    let running_containers = manager.list_running_containers().await.unwrap();

                    let old_tasks: Vec<ContainerInfo> = running_containers
                    .iter()
                    .filter(|c| {
                        c.names.iter().any(|name| name.contains(TASK_PREFIX))
                            && task_id
                                .as_ref()
                                .map_or(true, |id| !c.names.iter().any(|name| name.contains(id)))
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
                        let task_id = format!("{}-{}", current_task.unwrap().id, TASK_PREFIX);
                        let container_match = running_containers.iter().find(|c| c.names.contains(&format!("/{}", task_id)));
                        if container_match.is_none() {
                            let running_tasks = starting_container_tasks.lock().await;
                            let has_running_tasks = running_tasks.iter().any(|h| !h.is_finished());
                            drop(running_tasks);

                            if has_running_tasks {
                                Console::info("DockerService", "Container is still starting ...");
                            } else {
                                Console::info("DockerService", "Starting new container ...");
                                let task_clone = task_clone.clone();
                                let manager_clone = manager_clone.clone();

                                let handle = tokio::spawn(async move {
                                    let payload = task_clone.unwrap();
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
                                    let container_id = manager_clone.start_container(&payload.image, &task_id, payload.env_vars, Some(cmd)).await.unwrap();

                                    Console::info("DockerService", &format!("Container started with id: {}", container_id));
                                });
                                starting_container_tasks.lock().await.push(handle);
                            }
                        } else {
                            let container_status = container_match.unwrap();
                            let status = manager.get_container_details(&container_status.id).await.unwrap();

                            let task_state = match status.status {
                                Some(ContainerStateStatusEnum::RUNNING) => TaskState::RUNNING,
                                Some(ContainerStateStatusEnum::CREATED) => TaskState::PENDING,
                                Some(ContainerStateStatusEnum::EXITED) => TaskState::COMPLETED,
                                Some(ContainerStateStatusEnum::DEAD) => TaskState::FAILED,
                                Some(ContainerStateStatusEnum::PAUSED) => TaskState::PAUSED,
                                Some(ContainerStateStatusEnum::RESTARTING) => TaskState::RESTARTING,
                                Some(ContainerStateStatusEnum::REMOVING) => TaskState::UNKNOWN,
                                _ => TaskState::UNKNOWN,
                            };
                            Console::info("DockerService", &format!("Task state: {:?}", task_state));
                            state.update_task_state(task_clone.unwrap().id, task_state).await;
                        }
                    }
                },
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::models::task::Task;
    use shared::models::task::TaskState;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_docker_service() {
        let cancellation_token = CancellationToken::new();
        let docker_service = DockerService::new(cancellation_token.clone());
        let task = Task {
            image: "ubuntu:latest".to_string(),
            name: "test".to_string(),
            id: Uuid::new_v4(),
            env_vars: None,
            command: None,
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
        tokio::time::sleep(Duration::from_secs(10)).await;
        state_clone.set_current_task(None).await;
        tokio::time::sleep(Duration::from_secs(10)).await;
        Console::info("DockerService", "Cancelling cancellation token");
        cancellation_token.cancel();
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
