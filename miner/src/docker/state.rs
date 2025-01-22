use shared::models::task::{Task, TaskState};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;
pub struct DockerState {
    current_task: Arc<Mutex<Option<Task>>>,
}

impl DockerState {
    pub fn new() -> Self {
        Self {
            current_task: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn set_current_task(&self, task: Option<Task>) {
        let mut current_task = self.current_task.lock().await;
        *current_task = task;
        println!("Current task set in docker state: {:?}", current_task);
    }

    pub async fn update_task_state(&self, task_id: Uuid, state: TaskState) {
        let mut current_task = self.current_task.lock().await;
        if let Some(task) = current_task.as_mut() {
            if task.id == task_id {
                task.state = state;
            }
        }
    }

    pub async fn get_current_task(&self) -> Option<Task> {
        let current_task = self.current_task.lock().await;
        current_task.clone()
    }
}
