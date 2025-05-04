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
        let all_tasks = self.store_context.task_store.get_all_tasks();

        for plugin in self.plugins.iter() {
            let filtered_tasks = plugin.filter_tasks(all_tasks.clone());
            if !filtered_tasks.is_empty() {
                return Ok(Some(filtered_tasks[0].clone()));
            }
        }
        Ok(None)
    }
}
