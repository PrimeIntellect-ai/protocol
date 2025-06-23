use crate::models::node::{NodeStatus, OrchestratorNode};
use crate::plugins::node_groups::NodeGroupsPlugin;
use crate::plugins::StatusUpdatePlugin;
use anyhow::Error;
use anyhow::Result;
use log::info;

#[async_trait::async_trait]
impl StatusUpdatePlugin for NodeGroupsPlugin {
    async fn handle_status_change(
        &self,
        node: &OrchestratorNode,
        _old_status: &NodeStatus,
    ) -> Result<(), Error> {
        let node_addr = node.address.to_string();

        info!(
            "Handling node status change in group plugin: node {} status is now {:?}",
            node_addr, node.status
        );

        match node.status {
            NodeStatus::Dead | NodeStatus::LowBalance => {
                // Dissolve entire group if node becomes unhealthy
                if let Some(group) = self.get_node_group(&node_addr).await? {
                    info!(
                        "Node {} became {}, dissolving entire group {} with {} nodes",
                        node_addr,
                        node.status,
                        group.id,
                        group.nodes.len()
                    );
                    self.dissolve_group(&group.id).await?;
                }
            }
            _ => {
                info!(
                    "No group action needed for node {} with status {:?}",
                    node_addr, node.status
                );
            }
        }

        Ok(())
    }
}
