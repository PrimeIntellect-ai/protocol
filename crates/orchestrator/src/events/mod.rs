use anyhow::Result;
use shared::models::task::Task;

pub trait TaskObserver: Send + Sync {
    fn on_task_created(&self, task: &Task) -> Result<()>;
    fn on_task_deleted(&self, task: Option<Task>) -> Result<()>;
}
