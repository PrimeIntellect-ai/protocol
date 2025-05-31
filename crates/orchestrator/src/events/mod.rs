use anyhow::Result;
use shared::models::task::Task;

// TODO: Shouldnt this be handled in a tokio trhead?
pub trait TaskObserver: Send + Sync {
    fn on_task_created(&self, task: &Task) -> Result<()>;
    fn on_task_deleted(&self, task: Option<Task>) -> Result<()>;
}
