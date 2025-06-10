use alloy::primitives::Address;
use shared::models::task::Task;
use std::sync::Arc;

use crate::plugins::{newest_task::NewestTaskPlugin, SchedulerPlugin};
use crate::store::core::StoreContext;
use anyhow::Result;

pub struct Scheduler {
    store_context: Arc<StoreContext>,
    plugins: Vec<Box<dyn SchedulerPlugin>>,
}

impl Scheduler {
    pub fn new(store_context: Arc<StoreContext>, plugins: Vec<Box<dyn SchedulerPlugin>>) -> Self {
        let mut plugins = plugins;
        if plugins.is_empty() {
            plugins.push(Box::new(NewestTaskPlugin));
        }

        Self {
            store_context,
            plugins,
        }
    }

    pub async fn get_task_for_node(&self, node_address: Address) -> Result<Option<Task>> {
        let mut all_tasks = self.store_context.task_store.get_all_tasks().await?;

        for plugin in self.plugins.iter() {
            let filtered_tasks = plugin.filter_tasks(&all_tasks, &node_address).await?;
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
        let scheduler = Scheduler::new(state.store_context.clone(), vec![]);

        let task = Task {
            id: Uuid::new_v4(),
            image: "image".to_string(),
            name: "name".to_string(),
            state: TaskState::PENDING,
            created_at: 1,
            ..Default::default()
        };

        let _ = state.store_context.task_store.add_task(task.clone()).await;

        let task_for_node = scheduler.get_task_for_node(Address::ZERO).await.unwrap();
        assert_eq!(task_for_node, Some(task));
    }
}
