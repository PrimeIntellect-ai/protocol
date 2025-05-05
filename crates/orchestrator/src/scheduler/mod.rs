mod plugins;

use alloy::primitives::Address;
use plugins::{newest_task::NewestTaskPlugin, SchedulerPlugin};
use shared::models::task::Task;
use std::sync::Arc;

use crate::store::core::StoreContext;
use anyhow::Result;

pub struct Scheduler {
    store_context: Arc<StoreContext>,
    plugins: Vec<Box<dyn SchedulerPlugin>>,
}

impl Scheduler {
    pub fn new(store_context: Arc<StoreContext>) -> Self {
        Self {
            store_context,
            plugins: vec![Box::new(NewestTaskPlugin)],
        }
    }

    pub fn get_task_for_node(&self, _node_address: Address) -> Result<Option<Task>> {
        let mut all_tasks = self.store_context.task_store.get_all_tasks();

        for plugin in self.plugins.iter() {
            let filtered_tasks = plugin.filter_tasks(&all_tasks);
            all_tasks = filtered_tasks;
        }

        if !all_tasks.is_empty() {
            return Ok(Some(all_tasks[0].clone()));
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use shared::models::task::TaskState;
    use uuid::Uuid;

    use crate::api::tests::helper::create_test_app_state;

    use super::*;

    #[tokio::test]
    async fn test_get_task_for_node() {
        let state = create_test_app_state().await;
        let scheduler = Scheduler::new(state.store_context.clone());

        let task = Task {
            id: Uuid::new_v4(),
            image: "image".to_string(),
            name: "name".to_string(),
            env_vars: None,
            command: None,
            args: None,
            state: TaskState::PENDING,
            created_at: 1,
            updated_at: None,
        };

        state.store_context.task_store.add_task(task.clone());

        let task_for_node = scheduler.get_task_for_node(Address::ZERO).unwrap();
        assert_eq!(task_for_node, Some(task));
    }
}
