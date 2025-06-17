use alloy::primitives::Address;
use anyhow::Error;
use anyhow::Result;
use async_trait::async_trait;
use shared::models::task::Task;

use crate::models::node::{NodeStatus, OrchestratorNode};

pub trait Plugin {}

#[async_trait]
pub trait StatusUpdatePlugin: Plugin + Send + Sync {
    async fn handle_status_change(
        &self,
        node: &OrchestratorNode,
        status: &NodeStatus,
    ) -> Result<(), Error>;
}

#[async_trait]
pub trait SchedulerPlugin: Plugin + Send + Sync {
    async fn filter_tasks(&self, tasks: &[Task], node_address: &Address) -> Result<Vec<Task>>;
}
