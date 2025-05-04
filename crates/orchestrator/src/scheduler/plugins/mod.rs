use shared::models::task::Task;
pub mod newest_task;

pub trait Plugin {}

pub trait SchedulerPlugin: Plugin + Send + Sync {
    fn filter_tasks(&self, tasks: Vec<Task>) -> Vec<Task>;
}
