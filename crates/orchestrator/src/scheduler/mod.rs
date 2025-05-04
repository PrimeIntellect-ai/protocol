use alloy::primitives::Address;
use shared::models::task::Task;
use std::sync::Arc;

use crate::store::core::StoreContext;
use anyhow::Result;

pub struct Scheduler {
    store_context: Arc<StoreContext>,
}

impl Scheduler {
    pub fn new(store_context: Arc<StoreContext>) -> Self {
        Self { store_context }
    }

    pub fn get_task_for_node(&self, _node_address: Address) -> Result<Option<Task>> {
        let current_task = self.store_context.task_store.get_current_task();
        Ok(current_task)
    }
}
