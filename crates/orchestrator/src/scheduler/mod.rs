use alloy::primitives::Address;
use shared::models::task::Task;
use std::sync::Arc;

use crate::plugins::{newest_task::NewestTaskPlugin, SchedulerPlugin};
use crate::store::core::StoreContext;
use anyhow::Result;

pub struct Scheduler {
    store_context: Arc<StoreContext>,
    plugins: Vec<Box<dyn SchedulerPlugin>>,
}
impl Scheduler {
    pub fn new(store_context: Arc<StoreContext>, plugins: Vec<Box<dyn SchedulerPlugin>>) -> Self {
        let mut plugins = plugins;
        if plugins.is_empty() {
            plugins.push(Box::new(NewestTaskPlugin));
        }

        Self {
            store_context,
            plugins,
        }
    }

    pub async fn get_task_for_node(&self, node_address: Address) -> Result<Option<Task>> {
        let mut all_tasks = self.store_context.task_store.get_all_tasks().await?;

        for plugin in self.plugins.iter() {
            let filtered_tasks = plugin.filter_tasks(&all_tasks, &node_address).await?;
            all_tasks = filtered_tasks;
        }

        if !all_tasks.is_empty() {
            let mut task = all_tasks[0].clone();

            // Replace variables in env_vars
            if let Some(env_vars) = &mut task.env_vars {
                for (_, value) in env_vars.iter_mut() {
                    let new_value = value
                        .replace("${TASK_ID}", &task.id.to_string())
                        .replace("${NODE_ADDRESS}", &node_address.to_string());
                    *value = new_value;
                }
            }

            // Replace variables in args
            if let Some(args) = &mut task.args {
                for arg in args.iter_mut() {
                    *arg = arg
                        .replace("${TASK_ID}", &task.id.to_string())
                        .replace("${NODE_ADDRESS}", &node_address.to_string());
                }
            }

            // Replace variables in volume mounts
            if let Some(volume_mounts) = &mut task.volume_mounts {
                // Extract group_id from metadata labels if available

                for volume_mount in volume_mounts.iter_mut() {
                    // Use the replace_labels method with all variables
                    let processed = volume_mount
                        .replace_labels(&task.id.to_string(), Some(&node_address.to_string()));

                    // Replace the mount with the processed version
                    *volume_mount = processed;
                }
            }

            return Ok(Some(task));
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use shared::models::task::TaskState;
    use std::collections::HashMap;
    use uuid::Uuid;

    use crate::api::tests::helper::create_test_app_state;

    use super::*;

    #[tokio::test]
    async fn test_get_task_for_node() {
        let state = create_test_app_state().await;
        let scheduler = Scheduler::new(state.store_context.clone(), vec![]);

        let task = Task {
            id: Uuid::new_v4(),
            image: "image".to_string(),
            name: "name".to_string(),
            state: TaskState::PENDING,
            created_at: 1,
            ..Default::default()
        };

        let _ = state.store_context.task_store.add_task(task.clone()).await;

        let task_for_node = scheduler.get_task_for_node(Address::ZERO).await.unwrap();
        assert_eq!(task_for_node, Some(task));
    }

    #[tokio::test]
    async fn test_variable_replacement() {
        let state = create_test_app_state().await;
        let scheduler = Scheduler::new(state.store_context.clone(), vec![]);
        let node_address = Address::from([1u8; 20]);

        let mut env_vars = HashMap::new();
        env_vars.insert("TASK_ID_VAR".to_string(), "task-${TASK_ID}".to_string());
        env_vars.insert("NODE_VAR".to_string(), "node-${NODE_ADDRESS}".to_string());

        let task = Task {
            id: Uuid::new_v4(),
            image: "image".to_string(),
            name: "name".to_string(),
            state: TaskState::PENDING,
            created_at: 1,
            env_vars: Some(env_vars),
            args: Some(vec![
                "--task=${TASK_ID}".to_string(),
                "--node=${NODE_ADDRESS}".to_string(),
            ]),
            ..Default::default()
        };

        let _ = state.store_context.task_store.add_task(task.clone()).await;

        let result = scheduler.get_task_for_node(node_address).await.unwrap();
        assert!(result.is_some());

        let returned_task = result.unwrap();

        // Check env vars replacement
        let env_vars = returned_task.env_vars.unwrap();
        assert_eq!(
            env_vars.get("TASK_ID_VAR").unwrap(),
            &format!("task-{}", task.id)
        );
        assert_eq!(
            env_vars.get("NODE_VAR").unwrap(),
            &format!("node-{}", node_address)
        );

        // Check args replacement
        let args = returned_task.args.unwrap();
        assert_eq!(args[0], format!("--task={}", task.id));
        assert_eq!(args[1], format!("--node={}", node_address));
    }

    #[tokio::test]
    async fn test_volume_mount_variable_replacement() {
        let state = create_test_app_state().await;
        let scheduler = Scheduler::new(state.store_context.clone(), vec![]);
        let node_address = Address::from([1u8; 20]);

        use shared::models::task::{TaskMetadata, VolumeMount};

        let mut labels = HashMap::new();
        labels.insert("GROUP_ID".to_string(), "training-group-001".to_string());

        let task = Task {
            id: Uuid::new_v4(),
            image: "pytorch/pytorch:latest".to_string(),
            name: "ml-task".to_string(),
            state: TaskState::PENDING,
            created_at: 1,
            volume_mounts: Some(vec![
                VolumeMount {
                    host_path: "/host/data/${TASK_ID}-${GROUP_ID}".to_string(),
                    container_path: "/data/${TASK_ID}".to_string(),
                },
                VolumeMount {
                    host_path: "/host/outputs/${TASK_ID}".to_string(),
                    container_path: "/outputs".to_string(),
                },
                VolumeMount {
                    host_path: "/host/logs/${NODE_ADDRESS}-${TIMESTAMP}".to_string(),
                    container_path: "/logs".to_string(),
                },
            ]),
            metadata: Some(TaskMetadata {
                labels: Some(labels),
            }),
            ..Default::default()
        };

        let _ = state.store_context.task_store.add_task(task.clone()).await;

        let result = scheduler.get_task_for_node(node_address).await.unwrap();
        assert!(result.is_some());

        let returned_task = result.unwrap();

        // Check volume mounts replacement
        let volume_mounts = returned_task.volume_mounts.unwrap();

        // First volume mount: should have TASK_ID and GROUP_ID replaced
        assert_eq!(
            volume_mounts[0].host_path,
            format!("/host/data/{}-training-group-001", task.id)
        );
        assert_eq!(
            volume_mounts[0].container_path,
            format!("/data/{}", task.id)
        );

        // Second volume mount: should have TASK_ID replaced
        assert_eq!(
            volume_mounts[1].host_path,
            format!("/host/outputs/{}", task.id)
        );
        assert_eq!(volume_mounts[1].container_path, "/outputs");

        // Third volume mount: should have NODE_ADDRESS and TIMESTAMP replaced
        assert!(volume_mounts[2]
            .host_path
            .starts_with(&format!("/host/logs/{}-", node_address)));
        assert!(volume_mounts[2].host_path.len() > format!("/host/logs/{}-", node_address).len());
        assert_eq!(volume_mounts[2].container_path, "/logs");
    }
}
