use crate::store::core::RedisStore;
use alloy::primitives::Address;
use anyhow::{anyhow, Result};
use log::error;
use redis::AsyncCommands;
use shared::models::metric::MetricEntry;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const ORCHESTRATOR_METRICS_STORE: &str = "orchestrator:metrics";
const ORCHESTRATOR_NODE_METRICS_STORE: &str = "orchestrator:node_metrics";

pub struct MetricsStore {
    redis: Arc<RedisStore>,
}

impl MetricsStore {
    pub fn new(redis: Arc<RedisStore>) -> Self {
        Self { redis }
    }

    fn clean_label(&self, label: &str) -> String {
        label.replace(':', "")
    }

    pub async fn migrate_node_metrics_if_needed(&self, node_address: Address) -> Result<()> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;
        let new_key = format!("{}:{}", ORCHESTRATOR_NODE_METRICS_STORE, node_address);

        // Check if the new node-centric key already exists
        let exists: bool = con.exists(&new_key).await?;
        if exists {
            // Migration already complete for this node
            return Ok(());
        }

        // Perform the slow SCAN to find all metrics for this node in the old data structure
        let pattern = format!("{}:*:*", ORCHESTRATOR_METRICS_STORE);
        let mut iter: redis::AsyncIter<String> = con.scan_match(&pattern).await?;
        let mut old_keys_to_migrate = Vec::new();

        while let Some(key) = iter.next_item().await {
            old_keys_to_migrate.push(key);
        }
        drop(iter);

        // Collect all metrics for this node from the old structure
        let mut node_metrics = HashMap::new();
        let mut keys_to_clean = Vec::new();

        for old_key in old_keys_to_migrate {
            if let Some(value_str) = con
                .hget::<_, _, Option<String>>(&old_key, node_address.to_string())
                .await?
            {
                if let Ok(val) = value_str.parse::<f64>() {
                    let parts: Vec<&str> = old_key.split(':').collect();
                    if parts.len() >= 4 {
                        let task_id = parts[2];
                        let metric_name = parts[3];
                        let new_metric_key = format!("{}:{}", task_id, metric_name);
                        node_metrics.insert(new_metric_key, val);
                        keys_to_clean.push(old_key);
                    }
                }
            }
        }

        // If we have metrics for this node, perform the atomic migration
        if !node_metrics.is_empty() {
            // Use Redis MULTI/EXEC transaction for atomicity
            let mut pipe = redis::pipe();
            pipe.atomic();

            // Set all metrics in the new node-centric key
            for (metric_key, value) in &node_metrics {
                pipe.hset(&new_key, metric_key, value);
            }

            // Clean up the old data structure by removing this node's fields
            for old_key in &keys_to_clean {
                pipe.hdel(old_key, node_address.to_string());
            }

            pipe.query_async::<()>(&mut con).await?;
        } else {
            // Even if no metrics exist, create an empty key to mark migration as complete
            let _: () = con.hset(&new_key, "_migrated", "true").await?;
            let _: () = con.hdel(&new_key, "_migrated").await?;
        }

        Ok(())
    }

    pub async fn store_metrics(
        &self,
        metrics: Option<Vec<MetricEntry>>,
        sender_address: Address,
    ) -> Result<()> {
        let Some(metrics) = metrics else {
            return Ok(());
        };

        if metrics.is_empty() {
            return Ok(());
        }

        let mut con = self.redis.client.get_multiplexed_async_connection().await?;

        let node_key = if sender_address == Address::ZERO {
            format!("{}:{}", ORCHESTRATOR_NODE_METRICS_STORE, Address::ZERO)
        } else {
            format!("{}:{}", ORCHESTRATOR_NODE_METRICS_STORE, sender_address)
        };

        for entry in metrics {
            let task_id = if entry.key.task_id.is_empty() {
                "manual".to_string()
            } else {
                entry.key.task_id.clone()
            };
            let cleaned_label = self.clean_label(&entry.key.label);
            let metric_key = format!("{}:{}", task_id, cleaned_label);

            // Check for dashboard-progress metrics to maintain max value behavior
            let should_update = if entry.key.label.contains("dashboard-progress") {
                let existing_value: Option<String> = con.hget(&node_key, &metric_key).await?;
                match existing_value {
                    Some(val) => match val.parse::<f64>() {
                        Ok(old_val) => entry.value > old_val,
                        Err(_) => true, // Overwrite if old value is not a valid float
                    },
                    None => true,
                }
            } else {
                true
            };

            if should_update {
                if let Err(err) = con
                    .hset::<_, _, _, ()>(&node_key, &metric_key, entry.value)
                    .await
                {
                    error!("Could not update metric value in redis: {}", err);
                }
            }
        }
        Ok(())
    }

    pub async fn store_manual_metrics(&self, label: String, value: f64) -> Result<()> {
        self.store_metrics(
            Some(vec![MetricEntry {
                key: shared::models::metric::MetricKey {
                    task_id: "".to_string(),
                    label,
                },
                value,
            }]),
            Address::ZERO,
        )
        .await
    }

    pub async fn delete_metric(&self, task_id: &str, label: &str, address: &str) -> Result<bool> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;
        let cleaned_label = self.clean_label(label);

        let node_address = match address.parse::<Address>() {
            Ok(addr) => addr,
            Err(_) => return Ok(false), // Invalid address format
        };

        // Try new node-centric model first
        let node_key = format!("{}:{}", ORCHESTRATOR_NODE_METRICS_STORE, node_address);
        let metric_key = format!("{}:{}", task_id, cleaned_label);

        let deleted_new: i32 = con.hdel(&node_key, &metric_key).await?;
        if deleted_new > 0 {
            return Ok(true);
        }

        // Fallback to old model for backward compatibility
        let old_redis_key = format!(
            "{}:{}:{}",
            ORCHESTRATOR_METRICS_STORE, task_id, cleaned_label
        );

        match con.hdel::<_, _, i32>(old_redis_key, address).await {
            Ok(deleted) => Ok(deleted == 1),
            Err(err) => {
                error!("Could not delete metric from redis: {}", err);
                Err(anyhow!("Failed to delete metric from redis: {}", err))
            }
        }
    }
    pub async fn get_aggregate_metrics_for_task(
        &self,
        task_id: &str,
    ) -> Result<HashMap<String, f64>> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;
        let pattern = format!("{}:{}:*", ORCHESTRATOR_METRICS_STORE, task_id);

        let mut iter: redis::AsyncIter<String> = con.scan_match(&pattern).await?;
        let mut all_keys = Vec::new();
        while let Some(key) = iter.next_item().await {
            all_keys.push(key);
        }

        // Drop the iterator to release the borrow on con
        drop(iter);

        let mut result: HashMap<String, f64> = HashMap::new();

        for key in all_keys {
            let values: HashMap<String, f64> = con.hgetall(&key).await?;
            let total: f64 = values.values().sum();

            if let Some(clean_key) = key.split(':').next_back() {
                result.insert(clean_key.to_string(), total);
            }
        }

        Ok(result)
    }

    pub async fn get_aggregate_metrics_for_all_tasks(&self) -> Result<HashMap<String, f64>> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;
        let pattern = format!("{}:*:*", ORCHESTRATOR_METRICS_STORE);

        // Use SCAN instead of KEYS
        let mut iter: redis::AsyncIter<String> = con.scan_match(&pattern).await?;
        let mut all_keys = Vec::new();
        while let Some(key) = iter.next_item().await {
            all_keys.push(key);
        }

        let tasks: HashSet<String> = all_keys
            .iter()
            .filter_map(|key| key.split(':').nth(2).map(String::from))
            .collect();

        let mut result: HashMap<String, f64> = HashMap::new();

        for task in tasks {
            let metrics = self.get_aggregate_metrics_for_task(&task).await?;
            for (label, value) in metrics {
                *result.entry(label).or_insert(0.0) += value;
            }
        }

        Ok(result)
    }
    pub async fn get_metrics_for_node(
        &self,
        node_address: Address,
    ) -> Result<HashMap<String, HashMap<String, f64>>> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;
        let node_key = format!("{}:{}", ORCHESTRATOR_NODE_METRICS_STORE, node_address);

        // Try to get from new node-centric model first (O(1) operation)
        let node_metrics: HashMap<String, f64> = con.hgetall(&node_key).await?;

        if !node_metrics.is_empty() {
            let mut result: HashMap<String, HashMap<String, f64>> = HashMap::new();
            for (metric_key, value) in node_metrics {
                if let Some((task_id, metric_name)) = metric_key.split_once(':') {
                    result
                        .entry(task_id.to_string())
                        .or_default()
                        .insert(metric_name.to_string(), value);
                }
            }
            return Ok(result);
        }

        // Fallback to old model if new model is empty (migration not yet done)
        let pattern = format!("{}:*:*", ORCHESTRATOR_METRICS_STORE);
        let mut iter: redis::AsyncIter<String> = con.scan_match(&pattern).await?;
        let mut all_keys = Vec::new();
        while let Some(key) = iter.next_item().await {
            all_keys.push(key);
        }
        drop(iter);

        let mut result: HashMap<String, HashMap<String, f64>> = HashMap::new();
        for key in all_keys {
            if let Some(value_str) = con
                .hget::<_, _, Option<String>>(&key, node_address.to_string())
                .await?
            {
                if let Ok(val) = value_str.parse::<f64>() {
                    let parts: Vec<&str> = key.split(':').collect();
                    if parts.len() >= 4 {
                        let task_id = parts[2].to_string();
                        let metric_name = parts[3].to_string();
                        result.entry(task_id).or_default().insert(metric_name, val);
                    }
                }
            }
        }
        Ok(result)
    }

    pub async fn get_all_metrics(
        &self,
    ) -> Result<HashMap<String, HashMap<String, HashMap<String, f64>>>> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;
        let pattern = format!("{}:*:*", ORCHESTRATOR_METRICS_STORE);

        // Use SCAN instead of KEYS
        let mut iter: redis::AsyncIter<String> = con.scan_match(&pattern).await?;
        let mut result: HashMap<String, HashMap<String, HashMap<String, f64>>> = HashMap::new();
        let mut all_keys = Vec::new();
        while let Some(key) = iter.next_item().await {
            all_keys.push(key);
        }
        drop(iter);

        for key in all_keys {
            if let [_, _, task_id, metric_name] = key.split(':').collect::<Vec<&str>>()[..] {
                let values: HashMap<String, f64> = con.hgetall(&key).await?;
                for (node_addr, val) in values {
                    result
                        .entry(task_id.to_string())
                        .or_default()
                        .entry(metric_name.to_string())
                        .or_default()
                        .insert(node_addr, val);
                }
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests::helper::create_test_app_state;
    use shared::models::metric::MetricEntry;
    use shared::models::metric::MetricKey;
    use std::str::FromStr;

    #[tokio::test]
    async fn test_store_metrics() {
        let app_state = create_test_app_state().await;
        let metrics_store = app_state.store_context.metrics_store.clone();

        let metrics = vec![MetricEntry {
            key: MetricKey {
                task_id: "task_1".to_string(),
                label: "cpu_usage".to_string(),
            },
            value: 1.0,
        }];
        metrics_store
            .store_metrics(Some(metrics), Address::ZERO)
            .await
            .unwrap();

        let metrics = vec![MetricEntry {
            key: MetricKey {
                task_id: "task_0".to_string(),
                label: "cpu_usage".to_string(),
            },
            value: 2.0,
        }];
        metrics_store
            .store_metrics(Some(metrics), Address::ZERO)
            .await
            .unwrap();

        let metrics = metrics_store
            .get_aggregate_metrics_for_task("task_1")
            .await
            .unwrap();
        assert_eq!(metrics.get("cpu_usage"), Some(&1.0));
        let metrics = metrics_store
            .get_aggregate_metrics_for_all_tasks()
            .await
            .unwrap();
        assert_eq!(metrics.get("cpu_usage"), Some(&3.0));
    }

    #[tokio::test]
    async fn test_get_metrics_for_node() {
        let app_state = create_test_app_state().await;
        let metrics_store = app_state.store_context.metrics_store.clone();

        let node_addr_0 = Address::ZERO;
        let node_addr_1 = Address::from_str("0x1234567890123456789012345678901234567890").unwrap();

        let metrics1 = vec![MetricEntry {
            key: MetricKey {
                task_id: "task_1".to_string(),
                label: "cpu_usage".to_string(),
            },
            value: 1.0,
        }];
        metrics_store
            .store_metrics(Some(metrics1.clone()), node_addr_0)
            .await
            .unwrap();
        metrics_store
            .store_metrics(Some(metrics1), node_addr_1)
            .await
            .unwrap();

        let metrics2 = vec![MetricEntry {
            key: MetricKey {
                task_id: "task_2".to_string(),
                label: "cpu_usage".to_string(),
            },
            value: 1.0,
        }];
        metrics_store
            .store_metrics(Some(metrics2), node_addr_1)
            .await
            .unwrap();

        let metrics = metrics_store
            .get_metrics_for_node(node_addr_0)
            .await
            .unwrap();
        assert_eq!(metrics.get("task_1").unwrap().get("cpu_usage"), Some(&1.0));
        assert_eq!(metrics.get("task_2"), None);

        let metrics_1 = metrics_store
            .get_metrics_for_node(node_addr_1)
            .await
            .unwrap();
        assert_eq!(
            metrics_1.get("task_1").unwrap().get("cpu_usage"),
            Some(&1.0)
        );
        assert_eq!(
            metrics_1.get("task_2").unwrap().get("cpu_usage"),
            Some(&1.0)
        );
    }
    #[tokio::test]
    async fn test_store_metrics_value_overwrite() {
        let app_state = create_test_app_state().await;
        let metrics_store = app_state.store_context.metrics_store.clone();
        let node_addr = Address::ZERO;

        // Test dashboard-progress metric maintains max value
        metrics_store
            .store_metrics(
                Some(vec![MetricEntry {
                    key: MetricKey {
                        task_id: "task_1".to_string(),
                        label: "dashboard-progress/test/value".to_string(),
                    },
                    value: 2.0,
                }]),
                node_addr,
            )
            .await
            .unwrap();

        metrics_store
            .store_metrics(
                Some(vec![MetricEntry {
                    key: MetricKey {
                        task_id: "task_1".to_string(),
                        label: "dashboard-progress/test/value".to_string(),
                    },
                    value: 1.0,
                }]),
                node_addr,
            )
            .await
            .unwrap();

        let metrics = metrics_store.get_metrics_for_node(node_addr).await.unwrap();
        assert_eq!(
            metrics
                .get("task_1")
                .unwrap()
                .get("dashboard-progress/test/value"),
            Some(&2.0)
        );

        metrics_store
            .store_metrics(
                Some(vec![MetricEntry {
                    key: MetricKey {
                        task_id: "task_1".to_string(),
                        label: "dashboard-progress/test/value".to_string(),
                    },
                    value: 3.0,
                }]),
                node_addr,
            )
            .await
            .unwrap();

        let metrics = metrics_store.get_metrics_for_node(node_addr).await.unwrap();
        assert_eq!(
            metrics
                .get("task_1")
                .unwrap()
                .get("dashboard-progress/test/value"),
            Some(&3.0)
        );

        // Test non-dashboard metric gets overwritten regardless of value
        metrics_store
            .store_metrics(
                Some(vec![MetricEntry {
                    key: MetricKey {
                        task_id: "task_1".to_string(),
                        label: "cpu_usage".to_string(),
                    },
                    value: 2.0,
                }]),
                node_addr,
            )
            .await
            .unwrap();

        metrics_store
            .store_metrics(
                Some(vec![MetricEntry {
                    key: MetricKey {
                        task_id: "task_1".to_string(),
                        label: "cpu_usage".to_string(),
                    },
                    value: 1.0,
                }]),
                node_addr,
            )
            .await
            .unwrap();

        let metrics = metrics_store.get_metrics_for_node(node_addr).await.unwrap();
        assert_eq!(metrics.get("task_1").unwrap().get("cpu_usage"), Some(&1.0));
    }
}
