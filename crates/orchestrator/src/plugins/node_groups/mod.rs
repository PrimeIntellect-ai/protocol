use super::webhook::WebhookPlugin;
use crate::models::node::{NodeStatus, OrchestratorNode};
use crate::store::core::{RedisStore, StoreContext};
use crate::utils::loop_heartbeats::LoopHeartbeats;
use anyhow::Error;
use anyhow::Result;
use log::{debug, error, info, warn};
use rand::seq::IteratorRandom;
use redis::{AsyncCommands, Script};
use serde::{Deserialize, Serialize};
use shared::models::node::{ComputeRequirements, NodeLocation};
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
const GROUPS_INDEX_KEY: &str = "orchestrator:groups_index";

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

#[derive(Debug, Clone, PartialEq)]
pub struct TaskSwitchingPolicy {
    /// Whether to enable task switching at all
    pub enabled: bool,
    /// Prefer forming larger groups even if it means switching tasks
    pub prefer_larger_groups: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProximityOptimizationPolicy {
    /// Optimize node groups based on proximity?
    pub enabled: bool,
}

impl Default for ProximityOptimizationPolicy {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl Default for TaskSwitchingPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            prefer_larger_groups: true,
        }
    }
}

pub struct NodeGroupsPlugin {
    configuration_templates: Vec<NodeGroupConfiguration>,
    store: Arc<RedisStore>,
    store_context: Arc<StoreContext>,
    node_groups_heartbeats: Option<Arc<LoopHeartbeats>>,
    webhook_plugins: Option<Vec<WebhookPlugin>>,
    task_switching_policy: TaskSwitchingPolicy,
    proximity_optimization_policy: ProximityOptimizationPolicy,
}

impl NodeGroupsPlugin {
    pub fn new(
        configuration_templates: Vec<NodeGroupConfiguration>,
        store: Arc<RedisStore>,
        store_context: Arc<StoreContext>,
        node_groups_heartbeats: Option<Arc<LoopHeartbeats>>,
        webhook_plugins: Option<Vec<WebhookPlugin>>,
    ) -> Self {
        Self::new_with_policy(
            configuration_templates,
            store,
            store_context,
            node_groups_heartbeats,
            webhook_plugins,
            TaskSwitchingPolicy::default(),
            ProximityOptimizationPolicy::default(),
        )
    }

    pub fn new_with_policy(
        configuration_templates: Vec<NodeGroupConfiguration>,
        store: Arc<RedisStore>,
        store_context: Arc<StoreContext>,
        node_groups_heartbeats: Option<Arc<LoopHeartbeats>>,
        webhook_plugins: Option<Vec<WebhookPlugin>>,
        task_switching_policy: TaskSwitchingPolicy,
        proximity_optimization_policy: ProximityOptimizationPolicy,
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

        sorted_configs.sort_by(|a, b| {
            // First priority: min_group_size (descending)
            let size_cmp = b.min_group_size.cmp(&a.min_group_size);
            if size_cmp != std::cmp::Ordering::Equal {
                return size_cmp;
            }

            // Second priority: configurations with requirements come before those without
            // This ensures specific configs (GPU, etc.) get processed before general configs
            match (&a.compute_requirements, &b.compute_requirements) {
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            }
        });

        Self {
            configuration_templates: sorted_configs,
            store,
            store_context,
            node_groups_heartbeats,
            webhook_plugins,
            task_switching_policy,
            proximity_optimization_policy,
        }
    }

    // TODO: this should consume self; refactor this to separate running logic
    // and other components. it appears quite a lot of different logic is
    // combined into this one type
    pub async fn run_group_management_loop(&self, duration: u64) -> Result<(), Error> {
        let mut interval = tokio::time::interval(Duration::from_secs(duration));

        loop {
            let start = std::time::Instant::now();
            interval.tick().await;

            // First, form new groups with optimal sizing
            if let Err(e) = self.try_form_new_groups().await {
                error!("Error in group formation: {}", e);
            }

            if let Err(e) = self.try_merge_solo_groups().await {
                error!("Error in group merging: {}", e);
            }

            if let Some(heartbeats) = &self.node_groups_heartbeats {
                heartbeats.update_node_groups();
            }

            let elapsed = start.elapsed();
            log::info!("Group management loop completed in {:?}", elapsed);
        }
    }

    /// Check if a node is compatible with a configuration's compute requirements
    fn is_node_compatible_with_config(
        config: &NodeGroupConfiguration,
        node: &OrchestratorNode,
    ) -> bool {
        match (&config.compute_requirements, &node.compute_specs) {
            (Some(reqs), Some(specs)) => specs.meets(reqs),
            (None, _) => true,
            _ => false,
        }
    }

    /// Calculate the distance between two geographic locations using the Haversine formula
    fn calculate_distance(loc1: &NodeLocation, loc2: &NodeLocation) -> f64 {
        const EARTH_RADIUS_KM: f64 = 6371.0;

        let lat1_rad = loc1.latitude.to_radians();
        let lat2_rad = loc2.latitude.to_radians();
        let delta_lat = (loc2.latitude - loc1.latitude).to_radians();
        let delta_lon = (loc2.longitude - loc1.longitude).to_radians();

        let a = (delta_lat / 2.0).sin().powi(2)
            + lat1_rad.cos() * lat2_rad.cos() * (delta_lon / 2.0).sin().powi(2);
        let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());

        EARTH_RADIUS_KM * c
    }

    /// Sort nodes by proximity to a reference node
    fn sort_nodes_by_proximity(
        reference_node: &OrchestratorNode,
        nodes: &mut Vec<&OrchestratorNode>,
    ) {
        if let Some(ref_location) = &reference_node.location {
            nodes.sort_by(|a, b| {
                let dist_a = a
                    .location
                    .as_ref()
                    .map(|loc| Self::calculate_distance(ref_location, loc))
                    .unwrap_or(f64::MAX);
                let dist_b = b
                    .location
                    .as_ref()
                    .map(|loc| Self::calculate_distance(ref_location, loc))
                    .unwrap_or(f64::MAX);
                dist_a
                    .partial_cmp(&dist_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
    }
    /// Determine if task switching is beneficial for group formation
    async fn should_switch_tasks(
        &self,
        current_groups: &[NodeGroup],
        potential_merged_size: usize,
    ) -> Result<bool, Error> {
        // Check if task switching is enabled
        if !self.task_switching_policy.enabled {
            return Ok(false);
        }

        // Must be solo groups to consider switching
        if !current_groups.iter().all(|g| g.nodes.len() == 1) {
            return Ok(false);
        }

        if potential_merged_size < 2 {
            return Ok(false);
        }

        // If prefer_larger_groups is disabled and all groups have tasks, don't switch
        if !self.task_switching_policy.prefer_larger_groups {
            for group in current_groups {
                if self.get_current_group_task(&group.id).await?.is_some() {
                    debug!(
                        "Group {} has task and prefer_larger_groups is disabled",
                        group.id
                    );
                    return Ok(false);
                }
            }
        }

        debug!(
            "Task switching beneficial: merging {} solo groups into group of size {} (policy: prefer_larger_groups={})",
            current_groups.len(),
            potential_merged_size,
            self.task_switching_policy.prefer_larger_groups
        );
        Ok(true)
    }

    /// Atomically create a new group with Redis operations
    async fn create_group_atomically(
        &self,
        group: &NodeGroup,
        conn: &mut redis::aio::MultiplexedConnection,
    ) -> Result<(), Error> {
        let mut pipe = redis::pipe();
        pipe.atomic();

        // Store group data
        let group_key = get_group_key(&group.id);
        let group_data = serde_json::to_string(group)?;
        pipe.set(&group_key, group_data);

        // Add group ID to groups index
        pipe.sadd(GROUPS_INDEX_KEY, &group.id);

        // Map nodes to group
        for node in &group.nodes {
            pipe.hset(NODE_GROUP_MAP_KEY, node, &group.id);
        }

        pipe.query_async::<()>(conn).await?;
        Ok(())
    }

    pub async fn get_node_group(&self, node_addr: &str) -> Result<Option<NodeGroup>, Error> {
        let mut conn = self.store.client.get_multiplexed_async_connection().await?;

        let group_id: Option<String> = conn.hget(NODE_GROUP_MAP_KEY, node_addr).await?;
        if let Some(group_id) = group_id {
            let group_key = get_group_key(&group_id);
            let group_data: Option<String> = conn.get(&group_key).await?;
            if let Some(group_data) = group_data {
                return Ok(Some(serde_json::from_str(&group_data)?));
            }
        }

        Ok(None)
    }

    pub async fn get_node_groups_batch(
        &self,
        node_addresses: &[String],
    ) -> Result<HashMap<String, Option<NodeGroup>>, Error> {
        let mut conn = self.store.client.get_multiplexed_async_connection().await?;
        let mut result = HashMap::new();

        if node_addresses.is_empty() {
            return Ok(result);
        }

        let mut pipe = redis::pipe();
        for node_addr in node_addresses {
            pipe.hget(NODE_GROUP_MAP_KEY, node_addr);
        }
        let group_ids: Vec<Option<String>> = pipe.query_async(&mut conn).await?;

        let unique_group_ids: HashSet<String> = group_ids
            .iter()
            .filter_map(|opt| opt.as_ref())
            .cloned()
            .collect();

        // Step 3: Batch fetch all group data
        let group_data: HashMap<String, NodeGroup> = if !unique_group_ids.is_empty() {
            let group_keys: Vec<String> = unique_group_ids
                .iter()
                .map(|id| get_group_key(id))
                .collect();

            let group_values: Vec<Option<String>> = conn.mget(&group_keys).await?;

            unique_group_ids
                .into_iter()
                .zip(group_values.into_iter())
                .filter_map(|(group_id, group_json)| {
                    group_json.and_then(|json| {
                        serde_json::from_str::<NodeGroup>(&json)
                            .map_err(|e| {
                                error!("Failed to parse group {group_id} data: {e}");
                                e
                            })
                            .ok()
                            .map(|group| (group_id, group))
                    })
                })
                .collect()
        } else {
            HashMap::new()
        };

        // Step 4: Build result mapping node addresses to their groups
        for (node_addr, group_id) in node_addresses.iter().zip(group_ids.iter()) {
            let group = group_id.as_ref().and_then(|id| group_data.get(id)).cloned();
            result.insert(node_addr.clone(), group);
        }

        Ok(result)
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
        let task_key = format!("{GROUP_TASK_KEY_PREFIX}{group_id}");
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
        let task_key = format!("{GROUP_TASK_KEY_PREFIX}{group_id}");
        let result: bool = conn.set_nx::<_, _, bool>(&task_key, task_id).await?;
        Ok(result)
    }

    async fn try_form_new_groups(&self) -> Result<Vec<NodeGroup>, Error> {
        let mut conn = self.store.client.get_multiplexed_async_connection().await?;

        let mut formed_groups = Vec::new();

        let available_configurations = self.get_available_configurations().await;
        debug!("Available configurations: {available_configurations:?}");

        let nodes = self.store_context.node_store.get_nodes().await?;
        let assigned_nodes: std::collections::HashMap<String, String> =
            conn.hgetall(NODE_GROUP_MAP_KEY).await?;

        debug!("Assigned nodes: {assigned_nodes:?}");

        let mut healthy_nodes = nodes
            .iter()
            .filter(|node| node.status == NodeStatus::Healthy)
            .filter(|node| node.p2p_id.is_some())
            .filter(|node| !assigned_nodes.contains_key(&node.address.to_string()))
            .collect::<Vec<&OrchestratorNode>>();
        info!(
            "Found {} healthy nodes for potential group formation",
            healthy_nodes.len()
        );

        let mut total_available = healthy_nodes.len();

        for config in &available_configurations {
            debug!("Checking configuration: {config:?}");
            while total_available >= config.min_group_size {
                let initial_available = total_available;

                // Find compatible nodes for this configuration
                let compatible_nodes: Vec<&OrchestratorNode> = healthy_nodes
                    .iter()
                    .filter(|node| Self::is_node_compatible_with_config(config, node))
                    .cloned()
                    .collect();

                if compatible_nodes.len() < config.min_group_size {
                    break;
                }

                let mut available_nodes = BTreeSet::new();
                let mut nodes_to_remove = Vec::new();

                if self.proximity_optimization_policy.enabled {
                    // Start with a seed node (prefer nodes with location data)
                    let seed_node = compatible_nodes
                        .iter()
                        .find(|n| n.location.is_some())
                        .or(compatible_nodes.first())
                        .copied();

                    if let Some(seed) = seed_node {
                        // Add the seed node
                        available_nodes.insert(seed.address.to_string());
                        nodes_to_remove.push(seed.address.to_string());

                        // Sort remaining nodes by proximity to seed
                        let mut remaining_compatible: Vec<&OrchestratorNode> = compatible_nodes
                            .into_iter()
                            .filter(|n| n.address != seed.address)
                            .collect();
                        Self::sort_nodes_by_proximity(seed, &mut remaining_compatible);

                        // Add closest nodes until we reach the desired size
                        for node in remaining_compatible {
                            if available_nodes.len() >= config.max_group_size {
                                break;
                            }
                            available_nodes.insert(node.address.to_string());
                            nodes_to_remove.push(node.address.to_string());
                        }
                    }
                } else {
                    for node in compatible_nodes {
                        if available_nodes.len() >= config.max_group_size {
                            break;
                        }
                        available_nodes.insert(node.address.to_string());
                        nodes_to_remove.push(node.address.to_string());
                    }
                }

                // Not enough nodes to form a group
                if available_nodes.len() < config.min_group_size {
                    break;
                }

                // Create new group
                let group_id = generate_group_id();
                debug!("Generating new group with ID: {group_id}");

                let group = NodeGroup {
                    id: group_id.clone(),
                    nodes: available_nodes.clone(),
                    created_at: chrono::Utc::now(),
                    configuration_name: config.name.clone(),
                };
                debug!("Created new group structure: {group:?}");

                // Use atomic group creation helper
                self.create_group_atomically(&group, &mut conn).await?;

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
                debug!("Group details: {group:?}");
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
                    if let Err(e) = plugin.send_group_created(
                        group.id.clone(),
                        group.configuration_name.clone(),
                        group.nodes.iter().cloned().collect(),
                    ) {
                        error!("Failed to send group created webhook: {}", e);
                    }
                }
            }
        }

        Ok(formed_groups)
    }

    /// Try to merge single-node groups when possible, including task switching
    async fn try_merge_solo_groups(&self) -> Result<Vec<NodeGroup>, Error> {
        debug!("Starting solo group merging process with task switching support");
        let mut conn = self.store.client.get_multiplexed_async_connection().await?;
        let mut merged_groups = Vec::new();

        // Get all existing groups and available configurations
        let all_groups = self.get_all_groups().await?;

        // Quick optimization: check if there are any solo groups before proceeding
        let solo_groups_count = all_groups.iter().filter(|g| g.nodes.len() == 1).count();
        if solo_groups_count < 2 {
            debug!("Found {solo_groups_count} solo groups, merging not beneficial");
            return Ok(merged_groups);
        }

        let available_configurations = self.get_available_configurations().await;

        debug!(
            "Found {} total groups ({} solo) for potential merging",
            all_groups.len(),
            solo_groups_count
        );

        for config in &available_configurations {
            // up-to-date list of groups
            let current_groups = self.get_all_groups().await?;
            let config_merged_groups = self
                .try_merge_groups_for_config(&current_groups, config, &mut conn)
                .await?;
            merged_groups.extend(config_merged_groups);
        }

        if !merged_groups.is_empty() {
            info!(
                "Group merging completed: created {} new merged groups",
                merged_groups.len()
            );
        } else {
            debug!("No groups were merged");
        }

        Ok(merged_groups)
    }

    /// Try to merge groups for a specific configuration
    async fn try_merge_groups_for_config(
        &self,
        all_groups: &[NodeGroup],
        config: &NodeGroupConfiguration,
        conn: &mut redis::aio::MultiplexedConnection,
    ) -> Result<Vec<NodeGroup>, Error> {
        let mut merged_groups = Vec::new();

        // Find compatible solo groups (including those with tasks)
        let compatible_groups = self.find_compatible_solo_groups(all_groups, config).await?;

        if compatible_groups.len() < config.min_group_size {
            debug!("Not enough compatible groups to merge");
            return Ok(merged_groups);
        }

        // Group merging attempts
        let mut remaining_groups = compatible_groups;
        while remaining_groups.len() >= config.min_group_size {
            let merge_result = self
                .attempt_group_merge(&remaining_groups, config, conn)
                .await?;

            match merge_result {
                Some((merged_group, used_group_ids)) => {
                    merged_groups.push(merged_group);
                    remaining_groups.retain(|g| !used_group_ids.contains(&g.id));
                }
                None => break, // No more merges possible
            }
        }

        Ok(merged_groups)
    }

    /// Find solo groups compatible with a configuration
    async fn find_compatible_solo_groups(
        &self,
        all_groups: &[NodeGroup],
        config: &NodeGroupConfiguration,
    ) -> Result<Vec<NodeGroup>, Error> {
        let nodes = self.store_context.node_store.get_nodes().await?;
        let node_specs: HashMap<String, &OrchestratorNode> = nodes
            .iter()
            .map(|node| (node.address.to_string(), node))
            .collect();

        let mut compatible_groups = Vec::new();

        for group in all_groups {
            if group.nodes.len() == 1
                && self.is_group_compatible_with_config(group, config, &node_specs)
            {
                compatible_groups.push(group.clone());
            }
        }

        Ok(compatible_groups)
    }

    /// Check if a group is compatible with a configuration
    fn is_group_compatible_with_config(
        &self,
        group: &NodeGroup,
        config: &NodeGroupConfiguration,
        node_specs: &HashMap<String, &OrchestratorNode>,
    ) -> bool {
        group.nodes.iter().all(|node_addr| {
            node_specs
                .get(node_addr)
                .map(|node| Self::is_node_compatible_with_config(config, node))
                .unwrap_or(false)
        })
    }

    /// Attempt to merge a group of compatible groups
    async fn attempt_group_merge(
        &self,
        compatible_groups: &[NodeGroup],
        config: &NodeGroupConfiguration,
        conn: &mut redis::aio::MultiplexedConnection,
    ) -> Result<Option<(NodeGroup, Vec<String>)>, Error> {
        let mut merge_batch = Vec::new();
        let mut total_nodes = BTreeSet::new();
        let mut groups_to_dissolve = Vec::new();

        // If proximity optimization is enabled, try to use location-based selection
        if self.proximity_optimization_policy.enabled {
            // Get node information for location data
            let nodes = self.store_context.node_store.get_nodes().await?;
            let node_map: HashMap<String, &OrchestratorNode> = nodes
                .iter()
                .map(|node| (node.address.to_string(), node))
                .collect();

            // Try to find a seed group with location data
            let seed_group = compatible_groups.iter().find(|group| {
                group
                    .nodes
                    .iter()
                    .next()
                    .and_then(|addr| node_map.get(addr))
                    .and_then(|node| node.location.as_ref())
                    .is_some()
            });

            if let Some(seed) = seed_group {
                // Found a seed with location, use proximity-based selection
                let seed_node = node_map.get(seed.nodes.iter().next().unwrap()).unwrap();

                merge_batch.push(seed.clone());
                total_nodes.extend(seed.nodes.iter().cloned());
                groups_to_dissolve.push(seed.id.clone());

                // Create a sorted list of remaining groups by proximity
                let mut remaining_with_distance: Vec<(f64, &NodeGroup)> = compatible_groups
                    .iter()
                    .filter(|g| g.id != seed.id)
                    .filter_map(|group| {
                        let node_addr = group.nodes.iter().next()?;
                        let node = node_map.get(node_addr)?;
                        let node_loc = node.location.as_ref()?;
                        let seed_loc = seed_node.location.as_ref()?;
                        let distance = Self::calculate_distance(seed_loc, node_loc);
                        Some((distance, group))
                    })
                    .collect();

                remaining_with_distance
                    .sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

                // Add closest groups first
                for (_, group) in remaining_with_distance {
                    if total_nodes.len() + group.nodes.len() <= config.max_group_size {
                        merge_batch.push(group.clone());
                        total_nodes.extend(group.nodes.iter().cloned());
                        groups_to_dissolve.push(group.id.clone());

                        if total_nodes.len() >= config.max_group_size {
                            break;
                        }
                    }
                }
            }
        }

        // If no proximity-based selection happened or we still need more groups, use original logic
        if merge_batch.is_empty()
            || (total_nodes.len() < config.max_group_size
                && total_nodes.len() < config.min_group_size)
        {
            // Reset if we didn't get enough nodes
            if total_nodes.len() < config.min_group_size {
                merge_batch.clear();
                total_nodes.clear();
                groups_to_dissolve.clear();
            }

            // Original selection logic
            for group in compatible_groups {
                if !groups_to_dissolve.contains(&group.id)
                    && total_nodes.len() + group.nodes.len() <= config.max_group_size
                {
                    merge_batch.push(group.clone());
                    total_nodes.extend(group.nodes.iter().cloned());
                    groups_to_dissolve.push(group.id.clone());

                    if total_nodes.len() >= config.max_group_size {
                        break;
                    }
                }
            }
        }
        // Validate merge conditions
        if !self
            .is_merge_beneficial(&merge_batch, total_nodes.len())
            .await?
        {
            return Ok(None);
        }

        // Perform the merge
        self.execute_group_merge(&merge_batch, config, &total_nodes, conn)
            .await
    }

    /// Check if merging these groups would be beneficial
    async fn is_merge_beneficial(
        &self,
        groups: &[NodeGroup],
        new_size: usize,
    ) -> Result<bool, Error> {
        if groups.len() < 2 || new_size < 2 {
            return Ok(false);
        }
        // Check if task switching is beneficial
        self.should_switch_tasks(groups, new_size).await
    }

    /// Execute the actual group merge with Redis operations
    async fn execute_group_merge(
        &self,
        groups_to_merge: &[NodeGroup],
        config: &NodeGroupConfiguration,
        merged_nodes: &BTreeSet<String>,
        conn: &mut redis::aio::MultiplexedConnection,
    ) -> Result<Option<(NodeGroup, Vec<String>)>, Error> {
        let group_ids_to_dissolve: Vec<String> =
            groups_to_merge.iter().map(|g| g.id.clone()).collect();

        // Create new merged group
        let new_group_id = generate_group_id();
        let merged_group = NodeGroup {
            id: new_group_id.clone(),
            nodes: merged_nodes.clone(),
            created_at: chrono::Utc::now(),
            configuration_name: config.name.clone(),
        };

        // Find the best task for the new group before starting transaction
        let selected_task = self.find_best_task_for_group(&merged_group).await?;

        // Begin atomic Redis transaction
        let mut pipe = redis::pipe();
        pipe.atomic();

        // Dissolve old groups
        for group_id in &group_ids_to_dissolve {
            // Get group data for webhook notifications (done before deletion)
            let group_key = get_group_key(group_id);

            // Remove nodes from group mapping
            if let Some(old_group) = groups_to_merge.iter().find(|g| &g.id == group_id) {
                for node in &old_group.nodes {
                    pipe.hdel(NODE_GROUP_MAP_KEY, node);
                }
            }

            // Remove group ID from groups index
            pipe.srem(GROUPS_INDEX_KEY, group_id);

            // Delete group task assignment
            let task_key = format!("{GROUP_TASK_KEY_PREFIX}{group_id}");
            pipe.del(&task_key);

            // Delete group
            pipe.del(&group_key);
        }

        // Create new merged group
        let group_key = get_group_key(&new_group_id);
        let group_data = serde_json::to_string(&merged_group)?;
        pipe.set(&group_key, group_data);

        // Add new group ID to groups index
        pipe.sadd(GROUPS_INDEX_KEY, &new_group_id);

        // Map nodes to new group
        for node in merged_nodes {
            pipe.hset(NODE_GROUP_MAP_KEY, node, &new_group_id);
        }

        // Assign task to new group if one was found
        if let Some(task) = &selected_task {
            let task_key = format!("{GROUP_TASK_KEY_PREFIX}{new_group_id}");
            pipe.set_nx(&task_key, task.id.to_string());
        }

        // Execute all operations atomically
        pipe.query_async::<()>(conn).await?;

        // Log results
        if let Some(task) = &selected_task {
            info!(
                "Successfully merged {} solo groups into group {} with {} nodes for configuration {} and assigned task {}",
                groups_to_merge.len(),
                new_group_id,
                merged_nodes.len(),
                config.name,
                task.id
            );
        } else {
            warn!(
                "Successfully merged {} solo groups into group {} with {} nodes for configuration {} but NO TASK ASSIGNED - group will be idle",
                groups_to_merge.len(),
                new_group_id,
                merged_nodes.len(),
                config.name
            );
        }

        // Send webhook notifications
        self.send_merge_webhooks(groups_to_merge, &merged_group);

        Ok(Some((merged_group, group_ids_to_dissolve)))
    }

    /// Send webhook notifications for group merging
    fn send_merge_webhooks(&self, dissolved_groups: &[NodeGroup], merged_group: &NodeGroup) {
        if let Some(plugins) = &self.webhook_plugins {
            // Send dissolved notifications
            for group in dissolved_groups {
                for plugin in plugins.iter() {
                    if let Err(e) = plugin.send_group_destroyed(
                        group.id.clone(),
                        group.configuration_name.clone(),
                        group.nodes.iter().cloned().collect(),
                    ) {
                        error!("Failed to send group dissolved webhook: {}", e);
                    }
                }
            }

            // Send created notification
            for plugin in plugins.iter() {
                if let Err(e) = plugin.send_group_created(
                    merged_group.id.clone(),
                    merged_group.configuration_name.clone(),
                    merged_group.nodes.iter().cloned().collect(),
                ) {
                    error!("Failed to send group created webhook: {}", e);
                }
            }
        }
    }

    pub(crate) async fn dissolve_group(&self, group_id: &str) -> Result<(), Error> {
        dissolve_group(group_id, &self.store, &self.webhook_plugins).await
    }

    pub(crate) async fn get_all_groups(&self) -> Result<Vec<NodeGroup>, Error> {
        debug!("Getting all groups");
        let mut conn = self.store.client.get_multiplexed_async_connection().await?;

        // Use SMEMBERS to get all group IDs from the groups index
        let group_ids: Vec<String> = conn.smembers(GROUPS_INDEX_KEY).await?;

        if group_ids.is_empty() {
            debug!("No groups found in index");
            return Ok(Vec::new());
        }

        debug!("Found {} group IDs in index", group_ids.len());

        // Use MGET to batch fetch all group data
        let group_keys: Vec<String> = group_ids.iter().map(|id| get_group_key(id)).collect();

        let group_values: Vec<Option<String>> = conn.mget(&group_keys).await?;

        // Parse the group data
        let mut groups = Vec::new();
        for (group_id, group_data) in group_ids.iter().zip(group_values.iter()) {
            if let Some(group_data) = group_data {
                match serde_json::from_str::<NodeGroup>(group_data) {
                    Ok(group) => groups.push(group),
                    Err(e) => {
                        error!("Failed to parse group {group_id} data: {e}");
                    }
                }
            } else {
                warn!("Group {group_id} exists in index but has no data");
            }
        }

        groups.sort_by(|a, b| a.id.cmp(&b.id));

        debug!("Successfully loaded {} groups", groups.len());
        Ok(groups)
    }

    pub(crate) async fn get_group_by_id(&self, group_id: &str) -> Result<Option<NodeGroup>, Error> {
        let mut conn = self.store.client.get_multiplexed_async_connection().await?;
        let group_key = get_group_key(group_id);

        if let Some(group_data) = conn.get::<_, Option<String>>(&group_key).await? {
            Ok(Some(serde_json::from_str(&group_data)?))
        } else {
            Ok(None)
        }
    }

    pub(crate) async fn get_all_node_group_mappings(
        &self,
    ) -> Result<HashMap<String, String>, Error> {
        let mut conn = self.store.client.get_multiplexed_async_connection().await?;

        let mappings: HashMap<String, String> = conn.hgetall(NODE_GROUP_MAP_KEY).await?;
        Ok(mappings)
    }

    /// Validate that a group still exists before task assignment (for scheduler integration)
    pub(crate) async fn validate_group_exists(&self, group_id: &str) -> Result<bool, Error> {
        let group = self.get_group_by_id(group_id).await?;
        Ok(group.is_some())
    }

    /// Handle the case where a group was dissolved while processing - for scheduler integration
    pub(crate) async fn handle_group_not_found(
        &self,
        group_id: &str,
        task_id: &str,
    ) -> Result<(), Error> {
        warn!(
            "Group {group_id} not found during task assignment for task {task_id}, attempting recovery"
        );

        // Try to find another suitable group for the task
        let all_groups = self.get_all_groups().await?;

        for group in all_groups {
            // Check if group is idle and compatible
            if self.get_current_group_task(&group.id).await?.is_none() {
                // Try to assign the task to this group
                match self.assign_task_to_group(&group.id, task_id).await {
                    Ok(true) => {
                        info!(
                            "Successfully reassigned task {} from dissolved group {} to group {}",
                            task_id, group_id, group.id
                        );
                        return Ok(());
                    }
                    Ok(false) => {
                        debug!(
                            "Group {} was assigned another task while trying to reassign task {}",
                            group.id, task_id
                        );
                        continue;
                    }
                    Err(e) => {
                        debug!(
                            "Failed to assign task {} to group {}: {}",
                            task_id, group.id, e
                        );
                        continue;
                    }
                }
            }
        }

        warn!(
            "Could not find alternative group for task {task_id} after group {group_id} was dissolved"
        );
        Ok(())
    }

    /// Find the best task for a newly merged group
    async fn find_best_task_for_group(&self, group: &NodeGroup) -> Result<Option<Task>, Error> {
        debug!(
            "Finding best task for merged group {} with configuration {}",
            group.id, group.configuration_name
        );

        // Get all available tasks
        let all_tasks = self.store_context.task_store.get_all_tasks().await?;

        // Filter tasks that are compatible with this group's configuration
        let applicable_tasks: Vec<Task> = all_tasks
            .into_iter()
            .filter(|task| {
                // Task is compatible if:
                // 1. It has no topology restrictions (None) - can run anywhere
                // 2. Its allowed_topologies list includes this group's configuration
                match &task.scheduling_config {
                    None => true, // No restrictions - compatible with any group
                    Some(config) => {
                        match config.plugins.as_ref().and_then(|p| p.get("node_groups")) {
                            None => true, // No node_groups plugin config - compatible
                            Some(node_config) => {
                                match node_config.get("allowed_topologies") {
                                    None => true, // No topology restrictions - compatible
                                    Some(topologies) => {
                                        // Task specifies allowed topologies - check if group's config is allowed
                                        let compatible =
                                            topologies.contains(&group.configuration_name);
                                        debug!(
                                            "Task {} topologies {:?} {} group config '{}'",
                                            task.id,
                                            topologies,
                                            if compatible { "includes" } else { "excludes" },
                                            group.configuration_name
                                        );
                                        compatible
                                    }
                                }
                            }
                        }
                    }
                }
            })
            .collect();

        if applicable_tasks.is_empty() {
            warn!(
                "No applicable tasks found for merged group {} with configuration {}",
                group.id, group.configuration_name
            );
            return Ok(None);
        }

        // Select random task
        let mut rng = rand::rng();
        let selected_task = applicable_tasks.into_iter().choose(&mut rng);

        if let Some(task) = selected_task {
            debug!(
                "Selected task {} for group {} (created_at: {})",
                task.id, group.id, task.created_at
            );
            Ok(Some(task))
        } else {
            warn!("No task could be selected for merged group {}", group.id);
            Ok(None)
        }
    }

    #[cfg(test)]
    pub(crate) async fn test_try_form_new_groups(&self) -> Result<Vec<NodeGroup>, Error> {
        self.try_form_new_groups().await
    }

    #[cfg(test)]
    pub(crate) async fn test_try_merge_solo_groups(&self) -> Result<Vec<NodeGroup>, Error> {
        self.try_merge_solo_groups().await
    }

    #[cfg(test)]
    pub(crate) async fn test_should_switch_tasks(
        &self,
        current_groups: &[NodeGroup],
        potential_merged_size: usize,
    ) -> Result<bool, Error> {
        self.should_switch_tasks(current_groups, potential_merged_size)
            .await
    }

    #[cfg(test)]
    pub(crate) async fn test_find_best_task_for_group(
        &self,
        group: &NodeGroup,
    ) -> Result<Option<Task>, Error> {
        self.find_best_task_for_group(group).await
    }

    #[cfg(test)]
    pub(crate) fn test_get_task_switching_policy(&self) -> &TaskSwitchingPolicy {
        &self.task_switching_policy
    }

    pub(crate) fn on_task_created(&self, task: &Task) -> Result<()> {
        debug!("Task created event received: {:?}", task);
        let topologies = get_task_topologies(task)?;
        debug!("Found {} topologies for new task", topologies.len());

        for topology in topologies {
            debug!("Enabling configuration for topology: {}", topology);
            let store = self.store.clone();
            tokio::spawn({
                let topology = topology.clone();
                async move {
                    if let Err(e) = enable_configuration(&store, &topology).await {
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

    pub(crate) fn on_task_deleted(&self, task: Option<Task>) -> Result<()> {
        if let Some(task) = task {
            debug!("Task deleted event received: {task:?}");
            let task_id = task.id.to_string();
            let topologies = get_task_topologies(&task)?;
            debug!("Found {} topologies for task cleanup", topologies.len());
            let store = self.store.clone();
            let store_context = self.store_context.clone();
            let webhook_plugins = self.webhook_plugins.clone();

            tokio::spawn({
                let task_id = task_id.clone();
                let topologies = topologies.clone();
                async move {
                    // Immediately dissolve all groups assigned to this specific task
                    debug!("Dissolving groups for deleted task: {task_id}");
                    let groups_for_task = match get_groups_for_task(store.clone(), &task_id).await {
                        Ok(groups) => groups,
                        Err(e) => {
                            error!("Failed to get groups for task {task_id}: {e}");
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
                            debug!("Dissolving group {group_id} for deleted task {task_id}");
                            if let Err(e) = dissolve_group(group_id, &store, &webhook_plugins).await
                            {
                                error!(
                                    "Failed to dissolve group {group_id} for task {task_id}: {e}"
                                );
                            } else {
                                info!(
                                    "Successfully dissolved group {group_id} for deleted task {task_id}"
                                );
                            }
                        }
                    } else {
                        debug!("No groups found for deleted task {task_id}");
                    }

                    // Also check if we need to disable configurations when no tasks remain for a topology
                    // This is secondary to the immediate group dissolution above
                    for topology in topologies {
                        debug!("Checking topology {topology} for configuration cleanup");
                        let remaining_tasks = match get_all_tasks_for_topology(
                            store_context.clone(),
                            &topology,
                        )
                        .await
                        {
                            Ok(tasks) => tasks,
                            Err(e) => {
                                error!("Failed to get tasks for topology {}: {}", topology, e);
                                continue;
                            }
                        };

                        if remaining_tasks.is_empty() {
                            debug!(
                                "No tasks remaining for topology {topology}, disabling configuration"
                            );
                            if let Err(e) = disable_configuration(&store, &topology).await {
                                error!(
                                    "Failed to disable configuration for topology {topology}: {e}"
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

pub(crate) async fn enable_configuration(
    store: &Arc<RedisStore>,
    configuration_name: &str,
) -> Result<(), Error> {
    let mut conn = store.client.get_multiplexed_async_connection().await?;
    conn.sadd::<_, _, ()>("available_node_group_configs", configuration_name)
        .await?;
    Ok(())
}

pub(crate) async fn disable_configuration(
    store: &Arc<RedisStore>,
    configuration_name: &str,
) -> Result<(), Error> {
    let mut conn = store.client.get_multiplexed_async_connection().await?;
    conn.srem::<_, _, ()>("available_node_group_configs", configuration_name)
        .await?;
    Ok(())
}

/// Get all groups assigned to a specific task
/// Returns a list of group IDs that are currently working on the given task
async fn get_groups_for_task(store: Arc<RedisStore>, task_id: &str) -> Result<Vec<String>, Error> {
    debug!("Getting all groups for task: {}", task_id);
    let mut conn = store.client.get_multiplexed_async_connection().await?;

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

async fn get_all_tasks_for_topology(
    store_context: Arc<StoreContext>,
    topology: &str,
) -> Result<Vec<Task>, Error> {
    debug!("Getting all tasks for topology: {}", topology);
    let all_tasks = store_context.task_store.get_all_tasks().await?;
    debug!("Found {} total tasks to check", all_tasks.len());

    let mut tasks = Vec::new();
    for task in all_tasks {
        let topologies = get_task_topologies(&task)?;
        if topologies.contains(&topology.to_string()) {
            tasks.push(task);
        }
    }
    debug!("Found {} tasks for topology {}", tasks.len(), topology);
    Ok(tasks)
}

pub(crate) fn get_task_topologies(task: &Task) -> Result<Vec<String>, Error> {
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

async fn dissolve_group(
    group_id: &str,
    store: &Arc<RedisStore>,
    webhook_plugins: &Option<Vec<WebhookPlugin>>,
) -> Result<(), Error> {
    debug!("Attempting to dissolve group: {}", group_id);
    let mut conn = store.client.get_multiplexed_async_connection().await?;

    let group_key = get_group_key(group_id);
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

        // Remove group ID from groups index
        pipe.srem(GROUPS_INDEX_KEY, group_id);

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

        if let Some(plugins) = webhook_plugins {
            for plugin in plugins.iter() {
                let plugin_clone = plugin.clone();
                let group_clone = group.clone();
                if let Err(e) = plugin_clone.send_group_destroyed(
                    group_clone.id.to_string(),
                    group_clone.configuration_name.to_string(),
                    group_clone.nodes.iter().cloned().collect(),
                ) {
                    error!("Failed to send group dissolved webhook: {}", e);
                }
            }
        }
    } else {
        debug!("No group found with ID: {}", group_id);
    }

    Ok(())
}

fn generate_group_id() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    format!("{:x}", rng.random::<u64>())
}

fn get_group_key(group_id: &str) -> String {
    format!("{}{}", GROUP_KEY_PREFIX, group_id)
}
