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

            // Replace variables in cmd
            if let Some(cmd) = &mut task.cmd {
                for arg in cmd.iter_mut() {
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
            cmd: Some(vec![
                "--task=${TASK_ID}".to_string(),
                "--node=${NODE_ADDRESS}".to_string(),
            ]),
            entrypoint: None,
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

        // Check cmd replacement
        let cmd = returned_task.cmd.unwrap();
        assert_eq!(cmd[0], format!("--task={}", task.id));
        assert_eq!(cmd[1], format!("--node={}", node_address));
    }
}
