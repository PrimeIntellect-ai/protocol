use alloy::primitives::Address;
use anyhow::Error;
use log::{debug, info};
use redis::Commands;
use serde::{Deserialize, Serialize};
use shared::models::task::Task;
use std::{collections::HashSet, sync::Arc};

use crate::{
    models::node::{NodeStatus, OrchestratorNode},
    prelude::Plugin,
    scheduler::plugins::SchedulerPlugin,
    store::core::{RedisStore, StoreContext},
};

use super::StatusUpdatePlugin;

const GROUP_KEY_PREFIX: &str = "node_group:";
const NODE_GROUP_MAP_KEY: &str = "node_to_group";

#[derive(Debug, Serialize, Deserialize)]
pub struct NodeGroup {
    pub id: String,
    pub nodes: HashSet<String>, // Node addresses
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone)]
pub struct NodeGroupsPlugin {
    // Configuration
    min_group_size: usize,
    max_group_size: usize,
    store: Arc<RedisStore>,
    store_context: Arc<StoreContext>,
}

impl NodeGroupsPlugin {
    pub fn new(
        min_group_size: usize,
        max_group_size: usize,
        store: Arc<RedisStore>,
        store_context: Arc<StoreContext>,
    ) -> Self {
        Self {
            min_group_size,
            max_group_size,
            store,
            store_context,
        }
    }

    fn generate_group_id() -> String {
        use rand::Rng;
        let mut rng = rand::rng();
        format!("group_{}", rng.random::<u64>())
    }

    fn get_group_key(group_id: &str) -> String {
        format!("{}{}", GROUP_KEY_PREFIX, group_id)
    }

    fn try_form_new_group(&self, node_addr: &str) -> Result<Option<NodeGroup>, Error> {
        let mut conn = self.store.client.get_connection()?;

        // Check if node is already in a group
        let existing_group: Option<String> = conn.hget(NODE_GROUP_MAP_KEY, node_addr)?;
        if existing_group.is_some() {
            return Ok(None);
        }

        let nodes = self.store_context.node_store.get_nodes();
        let healthy_nodes = nodes
            .iter()
            .filter(|node| node.status == NodeStatus::Healthy)
            .filter(|node| node.address.to_string() != node_addr)
            .collect::<Vec<&OrchestratorNode>>();
        println!("Healthy nodes: {:?}", healthy_nodes.len());
        println!("Min group size: {:?}", self.min_group_size);

        if (healthy_nodes.len() + 1) < self.min_group_size {
            println!("Not enough healthy nodes to form a group");
            return Ok(None);
        }

        let mut available_nodes = HashSet::new();
        available_nodes.insert(node_addr.to_string());

        for node in healthy_nodes {
            available_nodes.insert(node.address.to_string());
        }

        // Get all node->group mappings
        let assigned_nodes: std::collections::HashMap<String, String> =
            conn.hgetall(NODE_GROUP_MAP_KEY)?;

        // Scan through all node groups to find healthy unassigned nodes
        let keys: Vec<String> = conn.keys(format!("{}*", GROUP_KEY_PREFIX))?;
        for key in keys {
            let group_data: String = conn.get(&key)?;
            let group: NodeGroup = serde_json::from_str(&group_data)?;

            for node in group.nodes {
                if !assigned_nodes.contains_key(&node) {
                    available_nodes.insert(node);
                    if available_nodes.len() >= self.min_group_size {
                        break;
                    }
                }
            }
        }

        // Not enough nodes to form a group
        if available_nodes.len() < self.min_group_size {
            return Ok(None);
        }

        // Limit nodes if needed
        let nodes = if available_nodes.len() > self.max_group_size {
            available_nodes
                .into_iter()
                .take(self.max_group_size)
                .collect()
        } else {
            available_nodes
        };

        // Create new group
        let group_id = Self::generate_group_id();
        let group = NodeGroup {
            id: group_id.clone(),
            nodes: nodes.clone(),
            created_at: chrono::Utc::now(),
        };

        // Store group data
        let group_key = Self::get_group_key(&group_id);
        let group_data = serde_json::to_string(&group)?;
        conn.set::<_, _, ()>(&group_key, group_data)?;

        // Map nodes to group
        for node in &nodes {
            conn.hset::<_, _, _, ()>(NODE_GROUP_MAP_KEY, node, &group_id)?;
        }

        debug!(
            "Created new group {} with {} nodes{}",
            group_id,
            nodes.len(),
            if nodes.len() == self.max_group_size {
                " (limited by max size)"
            } else {
                ""
            }
        );
        Ok(Some(group))
    }

    fn dissolve_group(&self, group_id: &str) -> Result<(), Error> {
        let mut conn = self.store.client.get_connection()?;

        let group_key = Self::get_group_key(group_id);
        let group_data: Option<String> = conn.get(&group_key)?;

        if let Some(group_data) = group_data {
            let group: NodeGroup = serde_json::from_str(&group_data)?;

            // Remove all nodes from the group mapping
            for node in &group.nodes {
                conn.hdel::<_, _, ()>(NODE_GROUP_MAP_KEY, node)?;
            }

            // Delete group
            conn.del::<_, ()>(&group_key)?;

            info!("Dissolved group {}", group_id);
        }

        Ok(())
    }

    fn get_node_group(&self, node_addr: &str) -> Result<Option<NodeGroup>, Error> {
        let mut conn = self.store.client.get_connection()?;

        let group_id: Option<String> = conn.hget(NODE_GROUP_MAP_KEY, node_addr)?;
        if let Some(group_id) = group_id {
            let group_key = Self::get_group_key(&group_id);
            let group_data: Option<String> = conn.get(&group_key)?;
            if let Some(group_data) = group_data {
                return Ok(Some(serde_json::from_str(&group_data)?));
            }
        }

        Ok(None)
    }
}

impl Plugin for NodeGroupsPlugin {}

#[async_trait::async_trait]
impl StatusUpdatePlugin for NodeGroupsPlugin {
    async fn handle_status_change(
        &self,
        node: &OrchestratorNode,
        _old_status: &NodeStatus,
    ) -> Result<(), Error> {
        let node_addr = node.address.to_string();

        println!(
            "Handle node status change in group plugin {:?}",
            node.status
        );

        match node.status {
            NodeStatus::Healthy => {
                // Try to form new group with healthy nodes
                println!("Try to form new group with healthy nodes");
                if let Some(group) = self.try_form_new_group(&node_addr)? {
                    info!(
                        "Formed new group {} with {} nodes",
                        group.id,
                        group.nodes.len()
                    );
                }
            }
            NodeStatus::Dead => {
                // Dissolve entire group if node becomes unhealthy
                if let Some(group) = self.get_node_group(&node_addr)? {
                    info!(
                        "Node {} became {}, dissolving entire group {}",
                        node_addr, node.status, group.id
                    );
                    self.dissolve_group(&group.id)?;
                }
            }
            _ => {}
        }

        Ok(())
    }
}

impl SchedulerPlugin for NodeGroupsPlugin {
    fn filter_tasks(&self, tasks: &[Task], node_address: &Address) -> Vec<Task> {
        // Pretty dumb first version - we return a task when the node is in a group
        // otherwise we do not return a task
        if let Ok(Some(_)) = self.get_node_group(&node_address.to_string()) {
            return tasks.to_vec();
        }
        println!("Node {} is not in a group, skipping task", node_address);
        return vec![];
    }
}

#[cfg(test)]
mod tests {
    use crate::store::core::StoreContext;

    use super::*;
    use alloy::primitives::Address;
    use std::{str::FromStr, sync::Arc};

    fn create_test_node(addr: &str, status: NodeStatus) -> OrchestratorNode {
        OrchestratorNode {
            address: Address::from_str(addr).unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status,
            task_id: None,
            task_state: None,
            version: None,
            last_status_change: None,
        }
    }

    #[tokio::test]
    async fn test_group_formation_and_dissolution() {
        let store = Arc::new(RedisStore::new_test());
        let context_store = store.clone();
        let store_context = Arc::new(StoreContext::new(context_store));

        let plugin = NodeGroupsPlugin::new(2, 5, store.clone(), store_context);

        // Add first healthy node
        let node1 = create_test_node(
            "0x1234567890123456789012345678901234567890",
            NodeStatus::Healthy,
        );
        plugin.store_context.node_store.add_node(node1.clone());

        let _ = plugin
            .handle_status_change(&node1, &NodeStatus::Healthy)
            .await;

        // Add second healthy node to form group
        let node2 = create_test_node(
            "0x2234567890123456789012345678901234567890",
            NodeStatus::Healthy,
        );
        plugin.store_context.node_store.add_node(node2.clone());
        let _ = plugin
            .handle_status_change(&node2, &NodeStatus::Healthy)
            .await;

        // Verify group was created
        let mut conn = plugin.store.client.get_connection().unwrap();
        let group_id: Option<String> = conn
            .hget(NODE_GROUP_MAP_KEY, node1.address.to_string())
            .unwrap();
        assert!(group_id.is_some());

        // Make node unhealthy
        let node1_dead = create_test_node(
            "0x1234567890123456789012345678901234567890",
            NodeStatus::Dead,
        );
        let _ = plugin
            .handle_status_change(&node1_dead, &NodeStatus::Healthy)
            .await;

        // Verify group was dissolved
        let group_id: Option<String> = conn
            .hget(NODE_GROUP_MAP_KEY, node1.address.to_string())
            .unwrap();
        assert!(group_id.is_none());
    }
}
