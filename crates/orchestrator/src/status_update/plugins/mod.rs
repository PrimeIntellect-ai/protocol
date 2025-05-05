pub mod webhook;
use crate::models::node::NodeStatus;
use crate::models::node::OrchestratorNode;
use crate::prelude::Plugin;
use anyhow::Error;
use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait StatusUpdatePlugin: Plugin + Send + Sync {
    async fn handle_status_change(
        &self,
        node: &OrchestratorNode,
        status: &NodeStatus,
    ) -> Result<(), Error>;
}
