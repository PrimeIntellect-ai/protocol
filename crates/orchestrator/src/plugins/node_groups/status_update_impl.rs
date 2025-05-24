use crate::models::node::{NodeStatus, OrchestratorNode};
use crate::plugins::node_groups::{NodeGroupsPlugin, NODE_GROUP_MAP_KEY};
use crate::plugins::StatusUpdatePlugin;
use anyhow::Error;
use anyhow::Result;
use log::info;
use rand::seq::IndexedRandom;
use redis::Commands;
use std::collections::BTreeSet;

use super::NodeGroup;

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
                if let Some(group) = self.try_form_new_group(Some(node))? {
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

impl NodeGroupsPlugin {
    fn try_form_new_group(
        &self,
        new_healthy_node: Option<&OrchestratorNode>,
    ) -> Result<Option<NodeGroup>, Error> {
        let mut conn = self.store.client.get_connection()?;

        // Check if node is already in a group (if a specific node was provided)
        if let Some(node) = new_healthy_node {
            let existing_group: Option<String> =
                conn.hget(NODE_GROUP_MAP_KEY, node.address.to_string())?;
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
        if let Some(new_node) = new_healthy_node {
            healthy_nodes.retain(|node| node.address.to_string() != new_node.address.to_string());
        }

        info!(
            "Found {} healthy nodes for potential group formation",
            healthy_nodes.len()
        );

        // Calculate total available nodes (healthy nodes + the provided node if any)
        let total_available = healthy_nodes.len() + if new_healthy_node.is_some() { 1 } else { 0 };

        // Try each configuration in order
        for config in &self.configurations {
            if total_available < config.min_group_size {
                info!(
                    "Not enough healthy nodes for configuration {} (need {}, have {})",
                    config.name, config.min_group_size, total_available
                );
                continue;
            }

            // If a new node is provided, check if it meets the compute requirements
            // If it does not meet the requirements, we will not form a group with this configuration
            if let Some(compute_reqs) = &config.compute_requirements {
                if let Some(node) = new_healthy_node {
                    if let Some(compute_specs) = &node.compute_specs {
                        if !compute_specs.meets(compute_reqs) {
                            continue;
                        }
                    } else {
                        continue;
                    }
                }
            }

            let mut available_nodes = BTreeSet::new();

            // Add the provided node first if any
            if let Some(node) = new_healthy_node {
                available_nodes.insert(node.address.to_string());
            }

            for node in &healthy_nodes {
                let should_add_node = match (&config.compute_requirements, &node.compute_specs) {
                    (Some(reqs), Some(specs)) => specs.meets(reqs),
                    (None, _) => true,
                    _ => false,
                };

                if should_add_node {
                    available_nodes.insert(node.address.to_string());
                    if available_nodes.len() >= config.max_group_size {
                        break;
                    }
                }
            }

            // Not enough nodes to form a group
            if available_nodes.len() < config.min_group_size {
                info!(
                    "Not enough available nodes for configuration {} (have {}, need {})",
                    config.name,
                    available_nodes.len(),
                    config.min_group_size
                );
                continue;
            }

            // Create new group
            let group_id = Self::generate_group_id();
            let group = NodeGroup {
                id: group_id.clone(),
                nodes: available_nodes.clone(),
                created_at: chrono::Utc::now(),
                configuration_name: config.name.clone(),
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
            return Ok(Some(group));
        }

        info!("No suitable configuration found for group formation");
        Ok(None)
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
}
