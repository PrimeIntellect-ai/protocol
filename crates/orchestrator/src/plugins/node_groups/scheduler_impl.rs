use super::{NodeGroupsPlugin, SchedulerPlugin};
use alloy::primitives::Address;
use anyhow::Result;
use async_trait::async_trait;
use log::{error, info};
use rand::seq::IteratorRandom;
use redis::AsyncCommands;
use shared::models::task::Task;
use std::str::FromStr;

#[async_trait]
impl SchedulerPlugin for NodeGroupsPlugin {
    async fn filter_tasks(&self, tasks: &[Task], node_address: &Address) -> Result<Vec<Task>> {
        if let Ok(Some(group)) = self.get_node_group(&node_address.to_string()).await {
            info!(
                "Node {} is in group {} with {} nodes",
                node_address,
                group.id,
                group.nodes.len()
            );

            let idx = match self.get_idx_in_group(&group, &node_address.to_string()) {
                Ok(idx) => idx,
                Err(e) => {
                    error!("Failed to get index in group: {}", e);
                    return Ok(vec![]);
                }
            };

            let mut current_task: Option<Task> = None;
            match self.get_current_group_task(&group.id).await {
                Ok(Some(task)) => {
                    current_task = Some(task);
                }
                Ok(None) => {
                    if tasks.is_empty() {
                        return Ok(vec![]);
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
                        return Ok(vec![]);
                    }

                    // Select a random task before any await points
                    let selected_task = {
                        let mut rng = rand::rng();
                        applicable_tasks.into_iter().choose(&mut rng)
                    };

                    if let Some(new_task) = selected_task {
                        let task_id = new_task.id.to_string();
                        match self.assign_task_to_group(&group.id, &task_id).await {
                            Ok(true) => {
                                // Successfully assigned the task
                                current_task = Some(new_task.clone());
                            }
                            Ok(false) => {
                                // Another node already assigned a task, try to get it
                                if let Ok(Some(task)) = self.get_current_group_task(&group.id).await
                                {
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

                let next_node_idx = (idx + 1) % group.nodes.len();
                let next_node_addr = group.nodes.iter().nth(next_node_idx).unwrap();

                // Get p2p_id for next node from node store
                let next_p2p_id = if let Ok(Some(next_node)) = self
                    .store_context
                    .node_store
                    .get_node(&Address::from_str(next_node_addr).unwrap())
                    .await
                {
                    next_node.p2p_id.unwrap_or_default()
                } else {
                    String::new()
                };

                // Temporary hack to get the upload count
                let pattern = format!("upload:{}:{}:*", node_address, group.id);
                let total_upload_count =
                    match self.store.client.get_multiplexed_async_connection().await {
                        Ok(mut conn) => {
                            let mut keys: Vec<String> = Vec::new();
                            match conn.scan_match(&pattern).await {
                                Ok(mut iter) => {
                                    while let Some(key) = iter.next_item().await {
                                        keys.push(key);
                                    }
                                    keys.len().to_string()
                                }
                                Err(e) => {
                                    error!("Failed to scan upload keys: {}", e);
                                    "0".to_string()
                                }
                            }
                        }
                        Err(e) => {
                            error!("Failed to get Redis connection: {}", e);
                            "0".to_string()
                        }
                    };
                // File number starts with 0 for the first file, while filecount is at 1
                let last_file_idx = total_upload_count
                    .parse::<u32>()
                    .unwrap_or(0)
                    .saturating_sub(1);

                let mut env_vars = task_clone.env_vars.unwrap_or_default();
                env_vars.insert("GROUP_INDEX".to_string(), idx.to_string());
                for (_, value) in env_vars.iter_mut() {
                    let new_value = value
                        .replace("${GROUP_INDEX}", &idx.to_string())
                        .replace("${GROUP_SIZE}", &group.nodes.len().to_string())
                        .replace("${NEXT_P2P_ADDRESS}", &next_p2p_id)
                        .replace("${GROUP_ID}", &group.id)
                        .replace("${TOTAL_UPLOAD_COUNT}", &total_upload_count.to_string())
                        .replace("${LAST_FILE_IDX}", &last_file_idx.to_string());

                    *value = new_value;
                }
                task_clone.env_vars = Some(env_vars);
                task_clone.args = task_clone.args.map(|args| {
                    args.into_iter()
                        .map(|arg| {
                            arg.replace("${GROUP_INDEX}", &idx.to_string())
                                .replace("${GROUP_SIZE}", &group.nodes.len().to_string())
                                .replace("${NEXT_P2P_ADDRESS}", &next_p2p_id)
                                .replace("${GROUP_ID}", &group.id)
                                .replace("${TOTAL_UPLOAD_COUNT}", &total_upload_count.to_string())
                                .replace("${LAST_FILE_IDX}", &last_file_idx.to_string())
                        })
                        .collect::<Vec<String>>()
                });
                // Replace group variables in volume mounts if they exist
                if let Some(volume_mounts) = task_clone.volume_mounts {
                    task_clone.volume_mounts = Some(
                        volume_mounts
                            .into_iter()
                            .map(|mut volume_mount| {
                                volume_mount.host_path =
                                    volume_mount.host_path.replace("${GROUP_ID}", &group.id);

                                volume_mount.container_path = volume_mount
                                    .container_path
                                    .replace("${GROUP_ID}", &group.id);

                                volume_mount
                            })
                            .collect(),
                    );
                }

                return Ok(vec![task_clone]);
            }
        }
        info!(
            "Node {} is not in a group, skipping all tasks",
            node_address
        );
        Ok(vec![])
    }
}
