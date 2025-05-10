use alloy::primitives::Address;
use anyhow::Error;
use log::{debug, info};
use redis::Commands;
use serde::{Deserialize, Serialize};
use shared::models::task::Task;
use std::str::FromStr;
use std::{collections::BTreeSet, sync::Arc};

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
    pub nodes: BTreeSet<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone)]
pub struct NodeGroupsPlugin {
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
            .filter(|node| node.p2p_id.is_some())
            .collect::<Vec<&OrchestratorNode>>();
        info!(
            "Found {} healthy nodes for potential group formation",
            healthy_nodes.len()
        );
        if (healthy_nodes.len() + 1) < self.min_group_size {
            info!(
                "Not enough healthy nodes to form a group (need {}, have {})",
                self.min_group_size,
                healthy_nodes.len() + 1
            );
            return Ok(None);
        }

        let mut available_nodes = BTreeSet::new();
        available_nodes.insert(node_addr.to_string());

        for node in healthy_nodes {
            available_nodes.insert(node.address.to_string());
        }

        // Get all node->group mappings
        let assigned_nodes: std::collections::HashMap<String, String> =
            conn.hgetall(NODE_GROUP_MAP_KEY)?;

        // Scan through all node groups to find healthy unassigned nodes
        let mut keys: Vec<String> = conn.keys(format!("{}*", GROUP_KEY_PREFIX))?;
        keys.sort();
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
            info!("After scanning existing groups, still not enough available nodes (have {}, need {})",
                 available_nodes.len(), self.min_group_size);
            return Ok(None);
        }

        // Take nodes up to max size while preserving order
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

            info!(
                "Dissolved group {} with {} nodes",
                group_id,
                group.nodes.len()
            );
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

        info!(
            "Handling node status change in group plugin: node {} status is now {:?}",
            node_addr, node.status
        );

        match node.status {
            NodeStatus::Healthy => {
                // Try to form new group with healthy nodes
                info!(
                    "Node {} is healthy, attempting to form new group",
                    node_addr
                );
                if let Some(group) = self.try_form_new_group(&node_addr)? {
                    info!(
                        "Successfully formed new group {} with {} nodes",
                        group.id,
                        group.nodes.len()
                    );
                }
            }
            NodeStatus::Dead => {
                // Dissolve entire group if node becomes unhealthy
                if let Some(group) = self.get_node_group(&node_addr)? {
                    info!(
                        "Node {} became {}, dissolving entire group {} with {} nodes",
                        node_addr,
                        node.status,
                        group.id,
                        group.nodes.len()
                    );
                    self.dissolve_group(&group.id)?;
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

impl SchedulerPlugin for NodeGroupsPlugin {
    fn filter_tasks(&self, tasks: &[Task], node_address: &Address) -> Vec<Task> {
        // Pretty dumb first version - we return a task when the node is in a group
        // otherwise we do not return a task

        if let Ok(Some(group)) = self.get_node_group(&node_address.to_string()) {
            info!(
                "Node {} is in group {} with {} nodes",
                node_address,
                group.id,
                group.nodes.len()
            );

            let node_group_index = group
                .nodes
                .iter()
                .position(|n| n == &node_address.to_string())
                .unwrap();

            let mut final_tasks: Vec<Task> = Vec::new();
            for task in tasks {
                let mut task_clone = task.clone();

                let next_node_idx = (node_group_index + 1) % group.nodes.len();
                let next_node_addr = group.nodes.iter().nth(next_node_idx).unwrap();

                // Get p2p_id for next node from node store
                let next_p2p_id = if let Some(next_node) = self
                    .store_context
                    .node_store
                    .get_node(&Address::from_str(next_node_addr).unwrap())
                {
                    next_node.p2p_id.unwrap_or_default()
                } else {
                    String::new()
                };

                let mut env_vars = task_clone.env_vars.unwrap_or_default();
                env_vars.insert("GROUP_INDEX".to_string(), node_group_index.to_string());
                for (_, value) in env_vars.iter_mut() {
                    let new_value = value
                        .replace("${GROUP_INDEX}", &node_group_index.to_string())
                        .replace("${GROUP_SIZE}", &group.nodes.len().to_string())
                        .replace("${NEXT_P2P_ADDRESS}", &next_p2p_id);

                    *value = new_value;
                }
                task_clone.env_vars = Some(env_vars);
                task_clone.args = task_clone.args.map(|args| {
                    args.into_iter()
                        .map(|arg| {
                            arg.replace("${GROUP_INDEX}", &node_group_index.to_string())
                                .replace("${GROUP_SIZE}", &group.nodes.len().to_string())
                                .replace("${NEXT_P2P_ADDRESS}", &next_p2p_id)
                        })
                        .collect::<Vec<String>>()
                });

                final_tasks.push(task_clone);
            }

            info!(
                "Returning {} tasks for node {} in group {}",
                final_tasks.len(),
                node_address,
                group.id
            );
            return final_tasks;
        }
        info!(
            "Node {} is not in a group, skipping all tasks",
            node_address
        );
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use crate::store::core::StoreContext;

    use super::*;
    use alloy::primitives::Address;
    use shared::models::task::TaskState;
    use std::{collections::HashMap, str::FromStr, sync::Arc};
    use uuid::Uuid;

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
            p2p_id: Some("test_p2p_id".to_string()),
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

    #[tokio::test]
    async fn test_group_scheduling() {
        let store = Arc::new(RedisStore::new_test());
        let context_store = store.clone();
        let store_context = Arc::new(StoreContext::new(context_store));

        let plugin = NodeGroupsPlugin::new(2, 5, store.clone(), store_context);
        let node1 = create_test_node(
            "0x1234567890123456789012345678901234567890",
            NodeStatus::Healthy,
        );
        plugin.store_context.node_store.add_node(node1.clone());
        let node2 = create_test_node(
            "0x2234567890123456789012345678901234567890",
            NodeStatus::Healthy,
        );
        plugin.store_context.node_store.add_node(node2.clone());

        let mut env_vars = HashMap::new();
        env_vars.insert("LOCAL_RANK".to_string(), "0".to_string());
        env_vars.insert("RANK".to_string(), "${GROUP_INDEX}".to_string());
        env_vars.insert("WORLD_SIZE".to_string(), "${GROUP_SIZE}".to_string());

        let task1 = Task {
            id: Uuid::new_v4(),
            image: "prime-vllm".to_string(),
            name: "test-task".to_string(),
            env_vars: Some(env_vars),
            command: Some("uv".to_string()),
            args: Some(vec![
                "run".to_string(),
                "generate.py".to_string(),
                "--model".to_string(),
                "model/Qwen3-14B-${GROUP_INDEX}.${GROUP_SIZE}".to_string(),
                "--top-p".to_string(),
                "0.95".to_string(),
            ]),
            state: TaskState::PENDING,
            created_at: 0,
            updated_at: None,
        };

        let tasks = vec![task1];

        let filtered_tasks = plugin.filter_tasks(&tasks, &node1.address);
        assert_eq!(filtered_tasks.len(), 0);

        let _ = plugin
            .handle_status_change(&node1, &NodeStatus::Healthy)
            .await;

        let filtered_tasks = plugin.filter_tasks(&tasks, &node1.address);

        // Check both nodes get assigned valid and different indexes
        assert_eq!(filtered_tasks.len(), 1);
        let task = &filtered_tasks[0];
        let env_vars = task.env_vars.as_ref().unwrap();
        println!("task: {:?}", task);
        assert_eq!(env_vars.get("GROUP_INDEX").unwrap(), "0");
        assert_eq!(env_vars.get("RANK").unwrap(), "0");
        assert_eq!(env_vars.get("WORLD_SIZE").unwrap(), "2");
        assert_eq!(task.args.as_ref().unwrap()[3], "model/Qwen3-14B-0.2");

        let filtered_tasks = plugin.filter_tasks(&tasks, &node2.address);
        assert_eq!(filtered_tasks.len(), 1);
        let task = &filtered_tasks[0];
        let env_vars = task.env_vars.as_ref().unwrap();
        assert_eq!(env_vars.get("GROUP_INDEX").unwrap(), "1");
        assert_eq!(env_vars.get("RANK").unwrap(), "1");
        assert_eq!(env_vars.get("WORLD_SIZE").unwrap(), "2");
        assert_eq!(task.args.as_ref().unwrap()[3], "model/Qwen3-14B-1.2");
    }
}
