use super::{NodeGroupsPlugin, SchedulerPlugin};
use alloy::primitives::Address;
use log::{error, info};
use rand::seq::IndexedRandom;
use shared::models::task::Task;
use std::str::FromStr;

impl SchedulerPlugin for NodeGroupsPlugin {
    fn filter_tasks(&self, tasks: &[Task], node_address: &Address) -> Vec<Task> {
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

                    let applicable_tasks: Vec<Task> = tasks
                        .iter()
                        .filter(|&task| match &task.scheduling_config {
                            None => true,
                            Some(config) => {
                                match config.plugins.as_ref().and_then(|p| p.get("node_groups")) {
                                    None => true,
                                    Some(node_config) => {
                                        match node_config.get("allowed_topologies") {
                                            None => true,
                                            Some(topologies) => {
                                                topologies.contains(&group.configuration_name)
                                            }
                                        }
                                    }
                                }
                            }
                        })
                        .cloned()
                        .collect();
                    if applicable_tasks.is_empty() {
                        return vec![];
                    }

                    if let Some(new_task) = applicable_tasks.choose(&mut rand::rng()) {
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
