use chrono::{DateTime, Utc};
use shared::models::task::{Task, TaskState};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;
pub struct DockerState {
    current_task: Arc<Mutex<Option<Task>>>,
    last_started: Arc<Mutex<Option<DateTime<Utc>>>>,
    is_running: Arc<Mutex<bool>>,
}

impl DockerState {
    pub fn new() -> Self {
        Self {
            current_task: Arc::new(Mutex::new(None)),
            last_started: Arc::new(Mutex::new(None)),
            is_running: Arc::new(Mutex::new(false)),
        }
    }

    pub async fn set_current_task(&self, task: Option<Task>) {
        let mut current_task = self.current_task.lock().await;
        *current_task = task;
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

    pub async fn get_last_started(&self) -> Option<DateTime<Utc>> {
        let last_started = self.last_started.lock().await;
        *last_started
    }

    pub async fn set_last_started(&self, last_started: DateTime<Utc>) {
        let mut last_started_guard = self.last_started.lock().await;
        *last_started_guard = Some(last_started);
    }

    pub async fn get_is_running(&self) -> bool {
        let is_running = self.is_running.lock().await;
        *is_running
    }

    pub async fn set_is_running(&self, is_running: bool) {
        let mut is_running_guard = self.is_running.lock().await;
        *is_running_guard = is_running;
    }
}
