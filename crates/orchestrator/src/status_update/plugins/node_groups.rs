use alloy::primitives::Address;
use anyhow::Error;
use log::{error, info, warn};
use rand::seq::IndexedRandom;
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
const GROUP_TASK_KEY_PREFIX: &str = "group_task:";

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
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

    fn try_form_new_group(&self, node_addr: Option<&str>) -> Result<Option<NodeGroup>, Error> {
        let mut conn = self.store.client.get_connection()?;

        // Check if node is already in a group (if a specific node was provided)
        if let Some(addr) = node_addr {
            let existing_group: Option<String> = conn.hget(NODE_GROUP_MAP_KEY, addr)?;
            if existing_group.is_some() {
                return Ok(None);
            }
        }

        let nodes = self.store_context.node_store.get_nodes();

        // Get all node->group mappings to check which nodes are already in groups
        let assigned_nodes: std::collections::HashMap<String, String> =
            conn.hgetall(NODE_GROUP_MAP_KEY)?;

        let mut healthy_nodes = nodes
            .iter()
            .filter(|node| node.status == NodeStatus::Healthy)
            .filter(|node| node.p2p_id.is_some())
            .filter(|node| !assigned_nodes.contains_key(&node.address.to_string()))
            .collect::<Vec<&OrchestratorNode>>();

        // If a specific node was provided, make sure it's included and not counted twice
        if let Some(addr) = node_addr {
            healthy_nodes.retain(|node| node.address.to_string() != addr);
        }

        info!(
            "Found {} healthy nodes for potential group formation",
            healthy_nodes.len()
        );

        // Calculate total available nodes (healthy nodes + the provided node if any)
        let total_available = healthy_nodes.len() + if node_addr.is_some() { 1 } else { 0 };

        if total_available < self.min_group_size {
            info!(
                "Not enough healthy nodes to form a group (need {}, have {})",
                self.min_group_size, total_available
            );
            return Ok(None);
        }

        let mut available_nodes = BTreeSet::new();

        // Add the provided node first if any
        if let Some(addr) = node_addr {
            available_nodes.insert(addr.to_string());
        }

        for node in healthy_nodes {
            available_nodes.insert(node.address.to_string());
            if available_nodes.len() >= self.max_group_size {
                break;
            }
        }

        // Not enough nodes to form a group
        if available_nodes.len() < self.min_group_size {
            info!(
                "Not enough available nodes to form a group (have {}, need {})",
                available_nodes.len(),
                self.min_group_size
            );
            return Ok(None);
        }

        // Create new group
        let group_id = Self::generate_group_id();
        let group = NodeGroup {
            id: group_id.clone(),
            nodes: available_nodes.clone(),
            created_at: chrono::Utc::now(),
        };

        // Store group data
        let group_key = Self::get_group_key(&group_id);
        let group_data = serde_json::to_string(&group)?;
        conn.set::<_, _, ()>(&group_key, group_data)?;

        // Map nodes to group
        for node in &available_nodes {
            conn.hset::<_, _, _, ()>(NODE_GROUP_MAP_KEY, node, &group_id)?;
        }

        info!(
            "Created new group {} with {} nodes{}",
            group_id,
            available_nodes.len(),
            if available_nodes.len() == self.max_group_size {
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

    pub fn get_node_group(&self, node_addr: &str) -> Result<Option<NodeGroup>, Error> {
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
    fn get_current_group_task(&self, group_id: &str) -> Result<Option<Task>, Error> {
        let mut conn = self.store.client.get_connection()?;
        let task_key = format!("{}{}", GROUP_TASK_KEY_PREFIX, group_id);
        let task_id: Option<String> = conn.get(&task_key)?;

        if let Some(task_id) = task_id {
            if let Some(task) = self.store_context.task_store.get_task(&task_id) {
                return Ok(Some(task));
            }
            warn!("Task id set but task not found");
        }
        Ok(None)
    }

    fn assign_task_to_group(&self, group_id: &str, task_id: &str) -> Result<bool, Error> {
        let mut conn = self.store.client.get_connection()?;
        let task_key = format!("{}{}", GROUP_TASK_KEY_PREFIX, group_id);
        let result: bool = conn.set_nx::<_, _, bool>(&task_key, task_id)?;
        Ok(result)
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
                if let Some(group) = self.try_form_new_group(Some(&node_addr))? {
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
                    self.try_form_new_group(None)?;
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

            let mut current_task: Option<Task> = None;
            match self.get_current_group_task(&group.id) {
                Ok(Some(task)) => {
                    current_task = Some(task);
                }
                Ok(None) => {
                    if tasks.is_empty() {
                        return vec![];
                    }
                    if let Some(new_task) = tasks.choose(&mut rand::rng()) {
                        let task_id = new_task.id.to_string();
                        match self.assign_task_to_group(&group.id, &task_id) {
                            Ok(true) => {
                                // Successfully assigned the task
                                current_task = Some(new_task.clone());
                            }
                            Ok(false) => {
                                // Another node already assigned a task, try to get it
                                if let Ok(Some(task)) = self.get_current_group_task(&group.id) {
                                    current_task = Some(task);
                                }
                            }
                            Err(e) => {
                                error!("Failed to assign task to group: {}", e);
                            }
                        }
                    }
                }
                _ => {}
            }

            if let Some(t) = current_task {
                let mut task_clone = t.clone();

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
                        .replace("${NEXT_P2P_ADDRESS}", &next_p2p_id)
                        .replace("${GROUP_ID}", &group.id);

                    *value = new_value;
                }
                task_clone.env_vars = Some(env_vars);
                task_clone.args = task_clone.args.map(|args| {
                    args.into_iter()
                        .map(|arg| {
                            arg.replace("${GROUP_INDEX}", &node_group_index.to_string())
                                .replace("${GROUP_SIZE}", &group.nodes.len().to_string())
                                .replace("${NEXT_P2P_ADDRESS}", &next_p2p_id)
                                .replace("${GROUP_ID}", &group.id)
                        })
                        .collect::<Vec<String>>()
                });
                return vec![task_clone];
            }
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
        // Generate a deterministic IP address from the Ethereum address
        let addr_bytes = Address::from_str(addr).unwrap().to_vec();
        let ip_address = format!(
            "{}.{}.{}.{}",
            addr_bytes[0], addr_bytes[1], addr_bytes[2], addr_bytes[3]
        );

        OrchestratorNode {
            address: Address::from_str(addr).unwrap(),
            ip_address,
            port: 8080,
            status,
            task_id: None,
            task_state: None,
            version: None,
            last_status_change: None,
            p2p_id: Some("test_p2p_id".to_string()),
            compute_specs: None,
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
        plugin
            .store_context
            .node_store
            .update_node_status(&node1_dead.address, NodeStatus::Dead);
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
        let store: Arc<RedisStore> = Arc::new(RedisStore::new_test());
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
        env_vars.insert("GROUP_ID".to_string(), "${GROUP_ID}".to_string());

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
                "--group-id".to_string(),
                "${GROUP_ID}".to_string(),
            ]),
            state: TaskState::PENDING,
            created_at: 0,
            updated_at: None,
        };
        plugin.store_context.task_store.add_task(task1.clone());

        let mut task2 = task1.clone();
        task2.id = Uuid::new_v4();
        plugin.store_context.task_store.add_task(task2.clone());

        let mut task3 = task1.clone();
        task3.id = Uuid::new_v4();
        plugin.store_context.task_store.add_task(task3.clone());

        let tasks = vec![task1, task2, task3];

        let filtered_tasks = plugin.filter_tasks(&tasks, &node1.address);
        assert_eq!(filtered_tasks.len(), 0);

        let _ = plugin
            .handle_status_change(&node1, &NodeStatus::Healthy)
            .await;
        let mut tasks_clone = tasks.clone();
        tasks_clone.reverse();
        assert_ne!(tasks_clone[0].id, tasks[0].id);

        let (filtered_tasks_1, filtered_tasks_2) = tokio::join!(
            async { plugin.filter_tasks(&tasks, &node1.address) },
            async { plugin.filter_tasks(&tasks_clone, &node2.address) }
        );

        // Check both nodes get assigned valid and different indexes
        // Also ensure both nodes get the same task
        assert_eq!(filtered_tasks_1.len(), 1);
        let task_node_1 = &filtered_tasks_1[0];
        let env_vars_1 = task_node_1.env_vars.as_ref().unwrap();
        assert_eq!(env_vars_1.get("GROUP_INDEX").unwrap(), "0");
        assert_eq!(env_vars_1.get("RANK").unwrap(), "0");
        assert_eq!(env_vars_1.get("WORLD_SIZE").unwrap(), "2");
        assert_eq!(task_node_1.args.as_ref().unwrap()[3], "model/Qwen3-14B-0.2");
        assert_ne!(env_vars_1.get("GROUP_ID").unwrap(), "${GROUP_ID}");

        assert_eq!(filtered_tasks_2.len(), 1);
        let task_node_2 = &filtered_tasks_2[0];
        let env_vars_2 = task_node_2.env_vars.as_ref().unwrap();
        assert_eq!(env_vars_2.get("GROUP_INDEX").unwrap(), "1");
        assert_eq!(env_vars_2.get("RANK").unwrap(), "1");
        assert_eq!(env_vars_2.get("WORLD_SIZE").unwrap(), "2");
        assert_eq!(task_node_2.args.as_ref().unwrap()[3], "model/Qwen3-14B-1.2");
        assert_ne!(env_vars_2.get("GROUP_ID").unwrap(), "${GROUP_ID}");

        assert_eq!(task_node_1.id, task_node_2.id);
    }

    #[tokio::test]
    async fn test_group_scheduling_without_tasks() {
        let store: Arc<RedisStore> = Arc::new(RedisStore::new_test());
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
        let tasks = vec![];

        let filtered_tasks = plugin.filter_tasks(&tasks, &node1.address);
        assert_eq!(filtered_tasks.len(), 0);

        let _ = plugin
            .handle_status_change(&node1, &NodeStatus::Healthy)
            .await;

        let filtered_tasks = plugin.filter_tasks(&tasks, &node1.address);
        assert_eq!(filtered_tasks.len(), 0);

        let filtered_tasks = plugin.filter_tasks(&tasks, &node2.address);
        assert_eq!(filtered_tasks.len(), 0);
    }

    #[tokio::test]
    async fn test_group_formation_with_max_size() {
        let store = Arc::new(RedisStore::new_test());
        let context_store = store.clone();
        let store_context = Arc::new(StoreContext::new(context_store));

        // Set max group size to 2
        let plugin = NodeGroupsPlugin::new(2, 2, store.clone(), store_context);

        // Create three healthy nodes
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

        let node3 = create_test_node(
            "0x3234567890123456789012345678901234567890",
            NodeStatus::Healthy,
        );
        plugin.store_context.node_store.add_node(node3.clone());

        // Handle status changes to trigger group formation
        let _ = plugin
            .handle_status_change(&node1, &NodeStatus::Healthy)
            .await;
        let _ = plugin
            .handle_status_change(&node2, &NodeStatus::Healthy)
            .await;
        let _ = plugin
            .handle_status_change(&node3, &NodeStatus::Healthy)
            .await;

        // Create a test task
        let mut env_vars = HashMap::new();
        env_vars.insert("RANK".to_string(), "${GROUP_INDEX}".to_string());
        env_vars.insert("WORLD_SIZE".to_string(), "${GROUP_SIZE}".to_string());

        let task = Task {
            id: Uuid::new_v4(),
            image: "test-image".to_string(),
            name: "test-task".to_string(),
            env_vars: Some(env_vars),
            command: Some("run".to_string()),
            args: Some(vec!["--index".to_string(), "${GROUP_INDEX}".to_string()]),
            state: TaskState::PENDING,
            created_at: 0,
            updated_at: None,
        };
        plugin.store_context.task_store.add_task(task.clone());

        let tasks = vec![task];

        // Check if node1 and node2 are in a group
        let group1 = plugin.get_node_group(&node1.address.to_string()).unwrap();
        let group2 = plugin.get_node_group(&node2.address.to_string()).unwrap();

        // Check if node3 is not in a group
        let group3 = plugin.get_node_group(&node3.address.to_string()).unwrap();

        // Either node1 and node2 are in a group, or node2 and node3 are in a group
        // But all three cannot be in the same group due to max_group_size=2
        assert!(
            (group1.is_some()
                && group2.is_some()
                && group1.as_ref() == group2.as_ref()
                && group3.is_none())
                || (group2.is_some()
                    && group3.is_some()
                    && group2.as_ref() == group3.as_ref()
                    && group1.is_none())
                || (group1.is_some()
                    && group3.is_some()
                    && group1.as_ref() == group3.as_ref()
                    && group2.is_none())
        );

        // Verify that tasks are only assigned to nodes in a group
        for node in [&node1, &node2, &node3] {
            let filtered_tasks = plugin.filter_tasks(&tasks, &node.address);
            let group = plugin.get_node_group(&node.address.to_string()).unwrap();

            if group.is_some() {
                assert_eq!(
                    filtered_tasks.len(),
                    1,
                    "Node in group should receive tasks"
                );
            } else {
                assert_eq!(
                    filtered_tasks.len(),
                    0,
                    "Node not in group should not receive tasks"
                );
            }
        }
    }

    #[tokio::test]
    async fn test_node_cannot_be_in_multiple_groups() {
        let store = Arc::new(RedisStore::new_test());
        let context_store = store.clone();
        let store_context = Arc::new(StoreContext::new(context_store));
        // Set max_group_size to 2, so groups can only have 2 nodes
        let plugin = NodeGroupsPlugin::new(2, 2, store.clone(), store_context);

        let all_nodes = plugin.store_context.node_store.get_nodes();
        assert_eq!(all_nodes.len(), 0, "No nodes should be in the store");

        // Create three nodes
        let node1 = create_test_node(
            "0x1234567890123456789012345678901234567890",
            NodeStatus::Healthy,
        );
        let node2 = create_test_node(
            "0x2234567890123456789012345678901234567890",
            NodeStatus::Healthy,
        );
        let node3 = create_test_node(
            "0x3234567890123456789012345678901234567890",
            NodeStatus::Healthy,
        );

        // Add nodes to the store
        plugin.store_context.node_store.add_node(node1.clone());
        plugin.store_context.node_store.add_node(node2.clone());
        plugin.store_context.node_store.add_node(node3.clone());

        // Add nodes to groups through the normal flow
        let _ = plugin
            .handle_status_change(&node1, &NodeStatus::Healthy)
            .await;
        let _ = plugin
            .handle_status_change(&node2, &NodeStatus::Healthy)
            .await;
        let _ = plugin
            .handle_status_change(&node3, &NodeStatus::Healthy)
            .await;

        // Get connection to check Redis state
        let mut conn = plugin.store.client.get_connection().unwrap();

        // Verify each node's group assignment
        let node1_group_id: Option<String> = conn
            .hget(NODE_GROUP_MAP_KEY, node1.address.to_string())
            .unwrap();
        let node2_group_id: Option<String> = conn
            .hget(NODE_GROUP_MAP_KEY, node2.address.to_string())
            .unwrap();
        let node3_group_id: Option<String> = conn
            .hget(NODE_GROUP_MAP_KEY, node3.address.to_string())
            .unwrap();

        // With 3 nodes and max_group_size=2, one node MUST NOT be in a group
        let nodes_without_group = [&node1_group_id, &node2_group_id, &node3_group_id]
            .iter()
            .filter(|id| id.is_none())
            .count();

        assert_eq!(
            nodes_without_group, 1,
            "With 3 nodes and max_group_size=2, exactly one node must not be in a group"
        );

        // The other two nodes must be in the same group
        let group_ids: Vec<_> = [node1_group_id, node2_group_id, node3_group_id]
            .iter()
            .filter_map(|x| x.clone())
            .collect();
        assert_eq!(group_ids.len(), 2, "Exactly 2 nodes should have group IDs");
        assert_eq!(
            group_ids[0], group_ids[1],
            "The 2 nodes in groups should be in the same group"
        );

        // Get all group keys
        let group_keys: Vec<String> = conn.keys(format!("{}*", GROUP_KEY_PREFIX)).unwrap();
        let group_copy = group_keys.clone();

        // There should be exactly one group

        // Count how many groups each node appears in
        let mut node1_group_count = 0;
        let mut node2_group_count = 0;
        let mut node3_group_count = 0;

        for key in group_keys {
            let group_data: String = conn.get(&key).unwrap();
            let group: NodeGroup = serde_json::from_str(&group_data).unwrap();

            // Verify the group has exactly 2 nodes
            assert_eq!(group.nodes.len(), 2, "Group should have exactly 2 nodes");

            if group.nodes.contains(&node1.address.to_string()) {
                node1_group_count += 1;
            }
            if group.nodes.contains(&node2.address.to_string()) {
                node2_group_count += 1;
            }
            if group.nodes.contains(&node3.address.to_string()) {
                node3_group_count += 1;
            }
        }

        assert_eq!(group_copy.len(), 1, "There should be exactly one group");

        // Total group count should be 2 (exactly 2 nodes in groups)
        assert_eq!(
            node1_group_count + node2_group_count + node3_group_count,
            2,
            "Exactly 2 nodes should be in groups"
        );

        // Each node should appear in at most one group
        assert!(
            node1_group_count <= 1,
            "Node1 should be in at most one group"
        );
        assert!(
            node2_group_count <= 1,
            "Node2 should be in at most one group"
        );
        assert!(
            node3_group_count <= 1,
            "Node3 should be in at most one group"
        );

        // Add a fourth node and make it healthy
        let node4 = create_test_node(
            "0x4234567890123456789012345678901234567890",
            NodeStatus::Healthy,
        );
        plugin.store_context.node_store.add_node(node4.clone());
        let _ = plugin
            .handle_status_change(&node4, &NodeStatus::Healthy)
            .await;

        // Get updated group keys
        let group_keys: Vec<String> = conn.keys(format!("{}*", GROUP_KEY_PREFIX)).unwrap();

        // There should now be exactly two groups
        assert_eq!(
            group_keys.len(),
            2,
            "There should be exactly two groups after adding node4"
        );

        // Verify each node's updated group assignment
        let node1_group_id: Option<String> = conn
            .hget(NODE_GROUP_MAP_KEY, node1.address.to_string())
            .unwrap();
        let node2_group_id: Option<String> = conn
            .hget(NODE_GROUP_MAP_KEY, node2.address.to_string())
            .unwrap();
        let node3_group_id: Option<String> = conn
            .hget(NODE_GROUP_MAP_KEY, node3.address.to_string())
            .unwrap();
        let node4_group_id: Option<String> = conn
            .hget(NODE_GROUP_MAP_KEY, node4.address.to_string())
            .unwrap();

        // All nodes should now be in a group
        assert!(node1_group_id.is_some(), "Node1 should be in a group");
        assert!(node2_group_id.is_some(), "Node2 should be in a group");
        assert!(node3_group_id.is_some(), "Node3 should be in a group");
        assert!(node4_group_id.is_some(), "Node4 should be in a group");

        // Verify that we have exactly two distinct group IDs
        let all_group_ids = [
            node1_group_id.unwrap(),
            node2_group_id.unwrap(),
            node3_group_id.unwrap(),
            node4_group_id.unwrap(),
        ];
        let unique_group_ids: std::collections::HashSet<_> = all_group_ids.iter().collect();
        assert_eq!(
            unique_group_ids.len(),
            2,
            "There should be exactly two distinct group IDs"
        );
    }

    #[tokio::test]
    async fn test_reformation_on_death() {
        let store = Arc::new(RedisStore::new_test());
        let context_store = store.clone();
        let store_context = Arc::new(StoreContext::new(context_store));
        // Set max_group_size to 2, so groups can only have 2 nodes
        let plugin = NodeGroupsPlugin::new(2, 2, store.clone(), store_context);

        let all_nodes = plugin.store_context.node_store.get_nodes();
        assert_eq!(all_nodes.len(), 0, "No nodes should be in the store");

        // Create three nodes
        let node1 = create_test_node(
            "0x9234567890123456789012345678901234567890",
            NodeStatus::Healthy,
        );
        let mut node2 = create_test_node(
            "0x8234567890123456789012345678901234567890",
            NodeStatus::Healthy,
        );

        // Add nodes to the store
        plugin.store_context.node_store.add_node(node1.clone());
        plugin.store_context.node_store.add_node(node2.clone());

        // Add nodes to groups through the normal flow
        let _ = plugin
            .handle_status_change(&node1, &NodeStatus::Healthy)
            .await;
        let _ = plugin
            .handle_status_change(&node2, &NodeStatus::Healthy)
            .await;

        // Get connection to check Redis state
        let mut conn = plugin.store.client.get_connection().unwrap();

        // Verify each node's group assignment
        let node1_group_id: Option<String> = conn
            .hget(NODE_GROUP_MAP_KEY, node1.address.to_string())
            .unwrap();
        let node2_group_id: Option<String> = conn
            .hget(NODE_GROUP_MAP_KEY, node2.address.to_string())
            .unwrap();

        assert!(node1_group_id.is_some(), "Node1 should be in a group");
        assert!(node2_group_id.is_some(), "Node2 should be in a group");

        let node_3 = create_test_node(
            "0x3234567890123456789012345678901234567890",
            NodeStatus::Healthy,
        );
        plugin.store_context.node_store.add_node(node_3.clone());

        let node_3_group_id: Option<String> = conn
            .hget(NODE_GROUP_MAP_KEY, node_3.address.to_string())
            .unwrap();

        assert!(node_3_group_id.is_none(), "Node3 should not be in a group");

        node2.status = NodeStatus::Dead;
        plugin
            .store_context
            .node_store
            .update_node_status(&node2.address, NodeStatus::Dead);
        let _ = plugin
            .handle_status_change(&node2, &NodeStatus::Healthy)
            .await;

        let node_2_group_id: Option<String> = conn
            .hget(NODE_GROUP_MAP_KEY, node2.address.to_string())
            .unwrap();
        let node_1_group_id: Option<String> = conn
            .hget(NODE_GROUP_MAP_KEY, node1.address.to_string())
            .unwrap();
        let node_3_group_id: Option<String> = conn
            .hget(NODE_GROUP_MAP_KEY, node_3.address.to_string())
            .unwrap();

        assert!(node_2_group_id.is_none(), "Node2 should not be in a group");
        assert!(node_1_group_id.is_some(), "Node1 should be in a group");
        assert!(node_3_group_id.is_some(), "Node3 should be in a group");
    }
}
