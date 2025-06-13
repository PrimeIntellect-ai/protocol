use super::webhook::WebhookPlugin;
use super::{Plugin, SchedulerPlugin};
use crate::events::TaskObserver;
use crate::models::node::{NodeStatus, OrchestratorNode};
use crate::store::core::{RedisStore, StoreContext};
use crate::utils::loop_heartbeats::LoopHeartbeats;
use anyhow::Error;
use anyhow::Result;
use log::{debug, error, info, warn};
use redis::{AsyncCommands, Script};
use serde::{Deserialize, Serialize};
use shared::models::node::ComputeRequirements;
use shared::models::task::Task;
use std::time::Duration;
use std::{collections::BTreeSet, sync::Arc};
use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
};

pub mod scheduler_impl;
pub mod status_update_impl;
#[cfg(test)]
mod tests;

const GROUP_KEY_PREFIX: &str = "node_group:";
const NODE_GROUP_MAP_KEY: &str = "node_to_group";
const GROUP_TASK_KEY_PREFIX: &str = "group_task:";

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct NodeGroupConfiguration {
    pub name: String,
    pub min_group_size: usize,
    pub max_group_size: usize,
    #[serde(deserialize_with = "deserialize_compute_requirements")]
    pub compute_requirements: Option<ComputeRequirements>,
}

fn deserialize_compute_requirements<'de, D>(
    deserializer: D,
) -> Result<Option<ComputeRequirements>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: Option<String> = Option::deserialize(deserializer)?;
    match s {
        Some(s) => ComputeRequirements::from_str(&s)
            .map(Some)
            .map_err(serde::de::Error::custom),
        None => Ok(None),
    }
}

impl NodeGroupConfiguration {
    pub fn is_valid(&self) -> bool {
        if self.max_group_size < self.min_group_size {
            return false;
        }
        true
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct NodeGroup {
    pub id: String,
    pub nodes: BTreeSet<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub configuration_name: String,
}

#[derive(Clone)]
pub struct NodeGroupsPlugin {
    configuration_templates: Vec<NodeGroupConfiguration>,
    store: Arc<RedisStore>,
    store_context: Arc<StoreContext>,
    node_groups_heartbeats: Option<Arc<LoopHeartbeats>>,
    webhook_plugins: Option<Vec<WebhookPlugin>>,
}

impl NodeGroupsPlugin {
    pub fn new(
        configuration_templates: Vec<NodeGroupConfiguration>,
        store: Arc<RedisStore>,
        store_context: Arc<StoreContext>,
        node_groups_heartbeats: Option<Arc<LoopHeartbeats>>,
        webhook_plugins: Option<Vec<WebhookPlugin>>,
    ) -> Self {
        let mut sorted_configs = configuration_templates;

        let mut seen_names = HashSet::new();
        for config in &sorted_configs {
            if !seen_names.insert(config.name.clone()) {
                panic!("Configuration names must be unique");
            }
            if !config.is_valid() {
                panic!("Plugin configuration is invalid");
            }
        }

        sorted_configs.sort_by(|a, b| b.min_group_size.cmp(&a.min_group_size));

        let plugin = Self {
            configuration_templates: sorted_configs,
            store,
            store_context,
            node_groups_heartbeats,
            webhook_plugins,
        };

        plugin
            .store_context
            .task_store
            .add_observer(Arc::new(plugin.clone()));

        plugin
    }

    fn generate_group_id() -> String {
        use rand::Rng;
        let mut rng = rand::rng();
        format!("{:x}", rng.random::<u64>())
    }

    fn get_group_key(group_id: &str) -> String {
        format!("{}{}", GROUP_KEY_PREFIX, group_id)
    }

    pub async fn get_node_group(&self, node_addr: &str) -> Result<Option<NodeGroup>, Error> {
        let mut conn = self.store.client.get_multiplexed_async_connection().await?;

        let group_id: Option<String> = conn.hget(NODE_GROUP_MAP_KEY, node_addr).await?;
        if let Some(group_id) = group_id {
            let group_key = Self::get_group_key(&group_id);
            let group_data: Option<String> = conn.get(&group_key).await?;
            if let Some(group_data) = group_data {
                return Ok(Some(serde_json::from_str(&group_data)?));
            }
        }

        Ok(None)
    }

    pub async fn get_available_configurations(&self) -> Vec<NodeGroupConfiguration> {
        let mut conn = match self.store.client.get_multiplexed_async_connection().await {
            Ok(conn) => conn,
            Err(_) => return vec![],
        };

        let available_configs: HashSet<String> = conn
            .smembers("available_node_group_configs")
            .await
            .unwrap_or_default();

        let mut configs: Vec<NodeGroupConfiguration> = self
            .configuration_templates
            .iter()
            .filter(|config| available_configs.contains(&config.name))
            .cloned()
            .collect();

        configs.sort_by(|a, b| b.min_group_size.cmp(&a.min_group_size));
        configs
    }

    pub fn get_all_configuration_templates(&self) -> Vec<NodeGroupConfiguration> {
        self.configuration_templates.clone()
    }

    pub async fn enable_configuration(&self, configuration_name: &str) -> Result<(), Error> {
        let mut conn = self.store.client.get_multiplexed_async_connection().await?;
        conn.sadd::<_, _, ()>("available_node_group_configs", configuration_name)
            .await?;
        Ok(())
    }

    pub async fn disable_configuration(&self, configuration_name: &str) -> Result<(), Error> {
        let mut conn = self.store.client.get_multiplexed_async_connection().await?;
        conn.srem::<_, _, ()>("available_node_group_configs", configuration_name)
            .await?;
        Ok(())
    }

    pub fn get_idx_in_group(
        &self,
        node_group: &NodeGroup,
        node_addr: &str,
    ) -> Result<usize, Error> {
        node_group
            .nodes
            .iter()
            .position(|n| n == &node_addr.to_string())
            .ok_or_else(|| anyhow::anyhow!("Node {} not found in group", node_addr))
    }

    async fn get_current_group_task(&self, group_id: &str) -> Result<Option<Task>, Error> {
        let mut conn = self.store.client.get_multiplexed_async_connection().await?;
        let task_key = format!("{}{}", GROUP_TASK_KEY_PREFIX, group_id);
        let task_id: Option<String> = conn.get(&task_key).await?;

        if let Some(task_id) = task_id {
            if let Some(task) = self.store_context.task_store.get_task(&task_id).await? {
                return Ok(Some(task));
            }

            warn!("Task id set but task not found");
            let script = Script::new(
                r#"
            local task_key = KEYS[1]
            local expected_task_id = ARGV[1]
            
            local current_task_id = redis.call('GET', task_key)
            if current_task_id == expected_task_id then
                redis.call('DEL', task_key)
                return 1
            else
                return 0
            end
        "#,
            );

            let _: () = script
                .key(&task_key)
                .arg(task_id)
                .invoke_async(&mut conn)
                .await?;
        }
        Ok(None)
    }

    pub async fn assign_task_to_group(&self, group_id: &str, task_id: &str) -> Result<bool, Error> {
        let mut conn = self.store.client.get_multiplexed_async_connection().await?;
        let task_key = format!("{}{}", GROUP_TASK_KEY_PREFIX, group_id);
        let result: bool = conn.set_nx::<_, _, bool>(&task_key, task_id).await?;
        Ok(result)
    }

    async fn try_form_new_groups(&self) -> Result<Vec<NodeGroup>, Error> {
        let mut conn = self.store.client.get_multiplexed_async_connection().await?;

        let mut formed_groups = Vec::new();

        let available_configurations = self.get_available_configurations().await;
        debug!("Available configurations: {:?}", available_configurations);

        let nodes = self.store_context.node_store.get_nodes().await?;
        let assigned_nodes: std::collections::HashMap<String, String> =
            conn.hgetall(NODE_GROUP_MAP_KEY).await?;

        debug!("Assigned nodes: {:?}", assigned_nodes);

        let mut healthy_nodes = nodes
            .iter()
            .filter(|node| node.status == NodeStatus::Healthy)
            .filter(|node| node.p2p_id.is_some())
            .filter(|node| !assigned_nodes.contains_key(&node.address.to_string()))
            .collect::<Vec<&OrchestratorNode>>();
        println!(
            "Found {} healthy nodes for potential group formation",
            healthy_nodes.len()
        );
        info!(
            "Found {} healthy nodes for potential group formation",
            healthy_nodes.len()
        );

        let mut total_available = healthy_nodes.len();

        for config in &available_configurations {
            debug!("Checking configuration: {:?}", config);
            while total_available >= config.min_group_size {
                let initial_available = total_available;

                let mut available_nodes = BTreeSet::new();
                let mut nodes_to_remove = Vec::new();

                for node in &healthy_nodes {
                    let should_add_node = match (&config.compute_requirements, &node.compute_specs)
                    {
                        (Some(reqs), Some(specs)) => specs.meets(reqs),
                        (None, _) => true,
                        _ => false,
                    };

                    if should_add_node {
                        available_nodes.insert(node.address.to_string());
                        nodes_to_remove.push(node.address.to_string());
                        if available_nodes.len() >= config.max_group_size {
                            break;
                        }
                    }
                }

                println!("Available nodes: {:?}", available_nodes);
                // Not enough nodes to form a group
                if available_nodes.len() < config.min_group_size {
                    break;
                }

                // Create new group
                let group_id = Self::generate_group_id();
                debug!("Generating new group with ID: {}", group_id);

                let group = NodeGroup {
                    id: group_id.clone(),
                    nodes: available_nodes.clone(),
                    created_at: chrono::Utc::now(),
                    configuration_name: config.name.clone(),
                };
                debug!("Created new group structure: {:?}", group);

                // Store group data
                let group_key = Self::get_group_key(&group_id);
                let group_data = serde_json::to_string(&group)?;
                debug!("Storing group data at key: {}", group_key);
                conn.set::<_, _, ()>(&group_key, group_data).await?;

                // Map nodes to group
                debug!(
                    "Mapping {} nodes to group {}",
                    available_nodes.len(),
                    group_id
                );
                for node in &available_nodes {
                    conn.hset::<_, _, _, ()>(NODE_GROUP_MAP_KEY, node, &group_id)
                        .await?;
                }

                // Remove used nodes from healthy_nodes
                let prev_healthy_count = healthy_nodes.len();
                healthy_nodes.retain(|node| !nodes_to_remove.contains(&node.address.to_string()));
                total_available = healthy_nodes.len();
                debug!(
                    "Removed {} nodes from healthy pool ({} remaining)",
                    prev_healthy_count - healthy_nodes.len(),
                    healthy_nodes.len()
                );

                info!(
                    "Created new group {} with {} nodes for configuration {}{}",
                    group_id,
                    available_nodes.len(),
                    config.name,
                    if available_nodes.len() == config.max_group_size {
                        " (limited by max size)"
                    } else {
                        ""
                    }
                );
                debug!("Group details: {:?}", group);
                formed_groups.push(group);
                if total_available == initial_available {
                    break; // No progress made, exit this config
                }
            }
        }
        let webhook_groups = formed_groups.clone();
        for group in webhook_groups {
            if let Some(plugins) = &self.webhook_plugins {
                for plugin in plugins.iter() {
                    let group_clone = group.clone();
                    let plugin_clone = plugin.clone();
                    tokio::spawn(async move {
                        if let Err(e) = plugin_clone
                            .send_group_created(
                                group_clone.id.to_string(),
                                group_clone.configuration_name.to_string(),
                                group_clone.nodes.iter().cloned().collect(),
                            )
                            .await
                        {
                            error!("Failed to send group created webhook: {}", e);
                        }
                    });
                }
            }
        }

        Ok(formed_groups)
    }

    #[cfg(test)]
    pub async fn test_try_form_new_groups(&self) -> Result<Vec<NodeGroup>, Error> {
        self.try_form_new_groups().await
    }

    pub async fn run_group_management_loop(&self, duration: u64) -> Result<(), Error> {
        let mut interval = tokio::time::interval(Duration::from_secs(duration));

        loop {
            let start = std::time::Instant::now();
            interval.tick().await;

            if let Err(e) = self.try_form_new_groups().await {
                error!("Error in group management: {}", e);
            }
            if let Some(heartbeats) = &self.node_groups_heartbeats {
                heartbeats.update_node_groups();
            }

            let elapsed = start.elapsed();
            log::info!("Group management loop completed in {:?}", elapsed);
        }
    }

    pub async fn dissolve_group(&self, group_id: &str) -> Result<(), Error> {
        debug!("Attempting to dissolve group: {}", group_id);
        let mut conn = self.store.client.get_multiplexed_async_connection().await?;

        let group_key = Self::get_group_key(group_id);
        let group_data: Option<String> = conn.get(&group_key).await?;

        if let Some(group_data) = group_data {
            let group: NodeGroup = serde_json::from_str(&group_data)?;
            debug!("Found group to dissolve: {:?}", group);

            // Use a Redis transaction to atomically dissolve the group
            let mut pipe = redis::pipe();
            pipe.atomic();

            // Remove all nodes from the group mapping
            debug!("Removing {} nodes from group mapping", group.nodes.len());
            for node in &group.nodes {
                pipe.hdel(NODE_GROUP_MAP_KEY, node);
            }

            // Delete group task assignment
            let task_key = format!("{}{}", GROUP_TASK_KEY_PREFIX, group_id);
            debug!("Deleting group task assignment from key: {}", task_key);
            pipe.del(&task_key);

            // Delete group
            debug!("Deleting group data from key: {}", group_key);
            pipe.del(&group_key);

            // Execute all operations atomically
            pipe.query_async::<()>(&mut conn).await?;

            info!(
                "Dissolved group {} with {} nodes",
                group_id,
                group.nodes.len()
            );
            if let Some(plugins) = &self.webhook_plugins {
                for plugin in plugins.iter() {
                    let plugin_clone = plugin.clone();
                    let group_clone = group.clone();
                    tokio::spawn(async move {
                        if let Err(e) = plugin_clone
                            .send_group_destroyed(
                                group_clone.id.to_string(),
                                group_clone.configuration_name.to_string(),
                                group_clone.nodes.iter().cloned().collect(),
                            )
                            .await
                        {
                            error!("Failed to send group dissolved webhook: {}", e);
                        }
                    });
                }
            }
        } else {
            debug!("No group found with ID: {}", group_id);
        }

        Ok(())
    }

    pub fn get_task_topologies(&self, task: &Task) -> Result<Vec<String>, Error> {
        debug!("Getting topologies for task: {:?}", task);
        if let Some(config) = &task.scheduling_config {
            if let Some(plugins) = &config.plugins {
                if let Some(node_groups) = plugins.get("node_groups") {
                    if let Some(allowed_topologies) = node_groups.get("allowed_topologies") {
                        debug!("Found allowed topologies: {:?}", allowed_topologies);
                        return Ok(allowed_topologies.iter().map(|t| t.to_string()).collect());
                    }
                }
            }
        }
        debug!("No topologies found for task");
        Ok(vec![])
    }

    pub async fn get_all_tasks_for_topology(&self, topology: &str) -> Result<Vec<Task>, Error> {
        debug!("Getting all tasks for topology: {}", topology);
        let all_tasks = self.store_context.task_store.get_all_tasks().await?;
        debug!("Found {} total tasks to check", all_tasks.len());

        let mut tasks = Vec::new();
        for task in all_tasks {
            let topologies = self.get_task_topologies(&task)?;
            if topologies.contains(&topology.to_string()) {
                tasks.push(task);
            }
        }
        debug!("Found {} tasks for topology {}", tasks.len(), topology);
        Ok(tasks)
    }

    pub async fn get_all_groups(&self) -> Result<Vec<NodeGroup>, Error> {
        debug!("Getting all groups");
        let mut conn = self.store.client.get_multiplexed_async_connection().await?;

        // Get all node-to-group mappings
        let node_mappings: HashMap<String, String> = conn.hgetall(NODE_GROUP_MAP_KEY).await?;

        if node_mappings.is_empty() {
            debug!("No node mappings found");
            return Ok(Vec::new());
        }

        // Collect unique group IDs
        let group_ids: HashSet<String> = node_mappings.values().cloned().collect();
        debug!("Found {} unique group IDs", group_ids.len());

        // Fetch each group's data
        let mut groups = Vec::new();
        for group_id in group_ids {
            let group_key = Self::get_group_key(&group_id);
            if let Some(group_data) = conn.get::<_, Option<String>>(&group_key).await? {
                match serde_json::from_str::<NodeGroup>(&group_data) {
                    Ok(group) => groups.push(group),
                    Err(e) => {
                        error!("Failed to parse group {} data: {}", group_id, e);
                    }
                }
            } else {
                warn!("Group {} exists in mapping but has no data", group_id);
            }
        }

        debug!("Successfully loaded {} groups", groups.len());
        Ok(groups)
    }

    pub async fn get_group_by_id(&self, group_id: &str) -> Result<Option<NodeGroup>, Error> {
        let mut conn = self.store.client.get_multiplexed_async_connection().await?;
        let group_key = Self::get_group_key(group_id);

        if let Some(group_data) = conn.get::<_, Option<String>>(&group_key).await? {
            Ok(Some(serde_json::from_str(&group_data)?))
        } else {
            Ok(None)
        }
    }

    pub async fn get_all_node_group_mappings(&self) -> Result<HashMap<String, String>, Error> {
        let mut conn = self.store.client.get_multiplexed_async_connection().await?;

        let mappings: HashMap<String, String> = conn.hgetall(NODE_GROUP_MAP_KEY).await?;
        Ok(mappings)
    }

    /// Get all groups assigned to a specific task
    /// Returns a list of group IDs that are currently working on the given task
    pub async fn get_groups_for_task(&self, task_id: &str) -> Result<Vec<String>, Error> {
        debug!("Getting all groups for task: {}", task_id);
        let mut conn = self.store.client.get_multiplexed_async_connection().await?;

        // First, collect all group_task keys
        let pattern = format!("{}*", GROUP_TASK_KEY_PREFIX);
        let mut iter: redis::AsyncIter<String> = conn.scan_match(&pattern).await?;
        let mut all_keys = Vec::new();

        while let Some(key) = iter.next_item().await {
            all_keys.push(key);
        }

        // Drop the iterator to release the borrow on conn
        drop(iter);

        // Now check which keys point to our task_id
        let mut group_ids = Vec::new();
        for key in all_keys {
            if let Some(stored_task_id) = conn.get::<_, Option<String>>(&key).await? {
                if stored_task_id == task_id {
                    // Extract group_id from the key (remove the prefix)
                    if let Some(group_id) = key.strip_prefix(GROUP_TASK_KEY_PREFIX) {
                        group_ids.push(group_id.to_string());
                    }
                }
            }
        }

        debug!(
            "Found {} groups for task {}: {:?}",
            group_ids.len(),
            task_id,
            group_ids
        );
        Ok(group_ids)
    }
}

impl Plugin for NodeGroupsPlugin {}

impl TaskObserver for NodeGroupsPlugin {
    fn on_task_created(&self, task: &Task) -> Result<()> {
        debug!("Task created event received: {:?}", task);
        let topologies = self.get_task_topologies(task)?;
        debug!("Found {} topologies for new task", topologies.len());

        for topology in topologies {
            debug!("Enabling configuration for topology: {}", topology);
            tokio::spawn({
                let plugin = self.clone();
                let topology = topology.clone();
                async move {
                    if let Err(e) = plugin.enable_configuration(&topology).await {
                        error!(
                            "Failed to enable configuration for topology {}: {}",
                            topology, e
                        );
                    }
                }
            });
        }

        Ok(())
    }

    fn on_task_deleted(&self, task: Option<Task>) -> Result<()> {
        if let Some(task) = task {
            debug!("Task deleted event received: {:?}", task);
            let task_id = task.id.to_string();
            let topologies = self.get_task_topologies(&task)?;
            debug!("Found {} topologies for task cleanup", topologies.len());

            tokio::spawn({
                let plugin = self.clone();
                let task_id = task_id.clone();
                let topologies = topologies.clone();
                async move {
                    // Immediately dissolve all groups assigned to this specific task
                    debug!("Dissolving groups for deleted task: {}", task_id);
                    let groups_for_task = match plugin.get_groups_for_task(&task_id).await {
                        Ok(groups) => groups,
                        Err(e) => {
                            error!("Failed to get groups for task {}: {}", task_id, e);
                            return;
                        }
                    };

                    if !groups_for_task.is_empty() {
                        info!(
                            "Dissolving {} groups for deleted task {}",
                            groups_for_task.len(),
                            task_id
                        );

                        for group_id in &groups_for_task {
                            debug!("Dissolving group {} for deleted task {}", group_id, task_id);
                            if let Err(e) = plugin.dissolve_group(group_id).await {
                                error!(
                                    "Failed to dissolve group {} for task {}: {}",
                                    group_id, task_id, e
                                );
                            } else {
                                info!(
                                    "Successfully dissolved group {} for deleted task {}",
                                    group_id, task_id
                                );
                            }
                        }
                    } else {
                        debug!("No groups found for deleted task {}", task_id);
                    }

                    // Also check if we need to disable configurations when no tasks remain for a topology
                    // This is secondary to the immediate group dissolution above
                    for topology in topologies {
                        debug!("Checking topology {} for configuration cleanup", topology);
                        let remaining_tasks =
                            match plugin.get_all_tasks_for_topology(&topology).await {
                                Ok(tasks) => tasks,
                                Err(e) => {
                                    error!("Failed to get tasks for topology {}: {}", topology, e);
                                    continue;
                                }
                            };

                        if remaining_tasks.is_empty() {
                            debug!(
                                "No tasks remaining for topology {}, disabling configuration",
                                topology
                            );
                            if let Err(e) = plugin.disable_configuration(&topology).await {
                                error!(
                                    "Failed to disable configuration for topology {}: {}",
                                    topology, e
                                );
                            }
                        }
                    }
                }
            });
        }
        Ok(())
    }
}
