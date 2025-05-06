use shared::models::task::Task;
pub mod newest_task;
use crate::prelude::Plugin;

pub trait SchedulerPlugin: Plugin + Send + Sync {
    fn filter_tasks(&self, tasks: &[Task]) -> Vec<Task>;
}
