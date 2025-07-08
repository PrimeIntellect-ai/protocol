use anyhow::Result;
use shared::models::task::Task;

#[derive(Clone)]
pub struct NewestTaskPlugin;

impl NewestTaskPlugin {
    pub(crate) fn filter_tasks(&self, tasks: &[Task]) -> Result<Vec<Task>> {
        if tasks.is_empty() {
            return Ok(vec![]);
        }

        // Find newest task based on created_at timestamp
        Ok(tasks
            .iter()
            .max_by_key(|task| task.created_at)
            .map(|task| vec![task.clone()])
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use shared::models::task::TaskState;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn test_filter_tasks() {
        let plugin = NewestTaskPlugin;
        let tasks = vec![
            Task {
                id: Uuid::new_v4(),
                image: "image".to_string(),
                name: "name".to_string(),
                state: TaskState::PENDING,
                created_at: 1,
                ..Default::default()
            },
            Task {
                id: Uuid::new_v4(),
                image: "image".to_string(),
                name: "name".to_string(),
                state: TaskState::PENDING,
                created_at: 2,
                ..Default::default()
            },
        ];

        let filtered_tasks = plugin.filter_tasks(&tasks).unwrap();
        assert_eq!(filtered_tasks.len(), 1);
        assert_eq!(filtered_tasks[0].id, tasks[1].id);
    }
}
