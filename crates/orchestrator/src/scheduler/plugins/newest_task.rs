use shared::models::task::Task;

use super::{Plugin, SchedulerPlugin};

pub struct NewestTaskPlugin;

impl Plugin for NewestTaskPlugin {
    fn name(&self) -> String {
        "newest_task".to_string()
    }
}

impl SchedulerPlugin for NewestTaskPlugin {
    fn filter_tasks(&self, tasks: Vec<Task>) -> Vec<Task> {
        if tasks.is_empty() {
            return vec![];
        }

        // Find newest task based on created_at timestamp

        tasks
            .into_iter()
            .max_by_key(|task| task.created_at)
            .map(|task| vec![task])
            .unwrap_or_default()
    }
}
