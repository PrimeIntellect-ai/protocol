use super::{Plugin, SchedulerPlugin};
use crate::store::core::{RedisStore, StoreContext};
use anyhow::Error;
use anyhow::Result;
use log::warn;
use redis::{Commands, Script};
use serde::{Deserialize, Serialize};
use shared::models::node::ComputeRequirements;
use shared::models::task::Task;
use std::{collections::BTreeSet, sync::Arc};
use std::{collections::HashSet, str::FromStr};

pub mod scheduler_impl;
pub mod status_update_impl;
#[cfg(test)]
mod tests;

const GROUP_KEY_PREFIX: &str = "node_group:";
const NODE_GROUP_MAP_KEY: &str = "node_to_group";
const GROUP_TASK_KEY_PREFIX: &str = "group_task:";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NodeGroupConfiguration {
    name: String,
    min_group_size: usize,
    max_group_size: usize,
    #[serde(deserialize_with = "deserialize_compute_requirements")]
    compute_requirements: Option<ComputeRequirements>,
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
    configurations: Vec<NodeGroupConfiguration>,
    store: Arc<RedisStore>,
    store_context: Arc<StoreContext>,
}

impl NodeGroupsPlugin {
    pub fn new(
        configurations: Vec<NodeGroupConfiguration>,
        store: Arc<RedisStore>,
        store_context: Arc<StoreContext>,
    ) -> Self {
        let mut sorted_configs = configurations;

        // Check for duplicate configuration names
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

        Self {
            configurations: sorted_configs,
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

            let _: () = script.key(&task_key).arg(task_id).invoke(&mut conn)?;
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
