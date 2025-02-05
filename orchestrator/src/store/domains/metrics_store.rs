use crate::store::core::RedisStore;
use alloy::primitives::Address;
use log::error;
use redis::Commands;
use redis::RedisResult;
use shared::models::metric::MetricEntry;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;

const ORCHESTRATOR_METRICS_STORE: &str = "orchestrator:metrics";

pub struct MetricsStore {
    redis: Arc<RedisStore>,
}

impl MetricsStore {
    pub fn new(redis: Arc<RedisStore>) -> Self {
        Self { redis }
    }

    fn clean_label(&self, label: &str) -> String {
        label.replace(":", "")
    }
    pub fn store_metrics(&self, metrics: Option<Vec<MetricEntry>>, sender_address: Address) {
        let Some(metrics) = metrics else {
            return;
        };

        if metrics.is_empty() {
            return;
        }

        for entry in metrics {
            let cleaned_label = self.clean_label(&entry.key.label);
            let redis_key = format!(
                "{}:{}:{}",
                ORCHESTRATOR_METRICS_STORE, entry.key.task_id, cleaned_label
            );
            let mut con = self.redis.client.get_connection().unwrap();
            if let Err(err) = con.hset(
                redis_key,
                sender_address.to_string(),
                entry.value.to_string(),
            ) as RedisResult<()>
            {
                error!("Could not update metric value in redis: {}", err);
            }
        }
    }

    pub fn get_aggregate_metrics_for_task(&self, task_id: &str) -> HashMap<String, f64> {
        let mut con = self.redis.client.get_connection().unwrap();
        let all_keys: Vec<String> = con
            .keys(format!("{}:{}:*", ORCHESTRATOR_METRICS_STORE, task_id))
            .unwrap();

        let mut result: HashMap<String, f64> = HashMap::new();

        for key in all_keys {
            let values: HashMap<String, String> = con.hgetall(&key).unwrap();
            let total: f64 = values.values().filter_map(|v| v.parse::<f64>().ok()).sum();

            let clean_key = key.split(":").last().unwrap();
            result.insert(clean_key.to_string(), total);
        }

        println!("result {:?}", result);
        result
    }

    pub fn get_aggregate_metrics_for_all_tasks(&self) -> HashMap<String, f64> {
        let mut con = self.redis.client.get_connection().unwrap();
        let all_keys: Vec<String> = con
            .keys(format!("{}:*:*", ORCHESTRATOR_METRICS_STORE))
            .unwrap();
        println!("all_keys {:?}", all_keys);

        let tasks = all_keys
            .iter()
            .map(|key| key.split(":").nth(2).unwrap().to_string())
            .collect::<HashSet<String>>();

        let mut result: HashMap<String, f64> = HashMap::new();

        for task in tasks {
            let metrics = self.get_aggregate_metrics_for_task(&task);
            for (label, value) in metrics {
                result
                    .entry(label)
                    .and_modify(|v| *v += value)
                    .or_insert(value);
            }
        }

        println!("result {:?}", result);
        result
    } 

    pub fn get_metrics_for_node(&self, node_address: Address) -> HashMap<String, HashMap<String, f64>> {
        let mut con = self.redis.client.get_connection().unwrap();
        let all_keys: Vec<String> = con
            .keys(format!("{}:*:*", ORCHESTRATOR_METRICS_STORE))
            .unwrap();
        let mut result: HashMap<String, HashMap<String, f64>> = HashMap::new();

        for key in all_keys {
            let values: HashMap<String, String> = con.hgetall(&key).unwrap();
            
            // Get the metric value for this specific node address
            if let Some(value) = values.get(&node_address.to_string()) {
                if let Ok(val) = value.parse::<f64>() {
                    // Extract task ID and metric name from the key
                    let parts: Vec<&str> = key.split(":").collect();
                    let task_id = parts[2].to_string();
                    let metric_name = parts[3].to_string();

                    result
                        .entry(task_id)
                        .or_insert_with(HashMap::new)
                        .insert(metric_name, val);
                }
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests::helper::create_test_app_state;
    use shared::models::metric::MetricKey;
    use shared::models::metric::MetricEntry;
    use std::str::FromStr; 

    #[tokio::test]
    async fn test_store_metrics() {
        let app_state = create_test_app_state().await;
        let metrics_store = app_state.store_context.metrics_store.clone();

        let mut metrics = Vec::new();
        let task_id = "task_1";
        let metric_key = MetricKey {
            task_id: task_id.to_string(),
            label: "cpu_usage".to_string(),
        };
        let metric = MetricEntry {
            key: metric_key,
            value: 1.0,
        };
        metrics.push(metric);
        metrics_store.store_metrics(Some(metrics), Address::ZERO);

        let mut metrics = Vec::new();
        let task_id = "task_0";
        let metric_key = MetricKey {
            task_id: task_id.to_string(),
            label: "cpu_usage".to_string(),
        };
        let metric = MetricEntry {
            key: metric_key,
            value: 2.0,
        };
        metrics.push(metric);
        metrics_store.store_metrics(Some(metrics), Address::ZERO);

        let metrics: HashMap<String, f64> = metrics_store.get_aggregate_metrics_for_task("task_1");
        assert_eq!(metrics.get("cpu_usage"), Some(&1.0));
        let metrics: HashMap<String, f64> = metrics_store.get_aggregate_metrics_for_all_tasks();
        assert_eq!(metrics.get("cpu_usage"), Some(&3.0));
    }

    #[tokio::test]
    async fn test_get_metrics_for_node() {
        let app_state = create_test_app_state().await;
        let metrics_store = app_state.store_context.metrics_store.clone();

        let node_addr_0 = Address::ZERO;
        let node_addr_1 = Address::from_str("0x1234567890123456789012345678901234567890").unwrap();

        let mut metrics = Vec::new();
        let task_id = "task_1";
        let metric_key = MetricKey {
            task_id: task_id.to_string(),
            label: "cpu_usage".to_string(),
        };
        let metric = MetricEntry {
            key: metric_key,
            value: 1.0,
        };
        metrics.push(metric);
        let metrics2 = metrics.clone();
        metrics_store.store_metrics(Some(metrics), node_addr_0);
        metrics_store.store_metrics(Some(metrics2), node_addr_1);

        let mut metrics = Vec::new();
        let task_id = "task_2";
        let metric_key = MetricKey {
            task_id: task_id.to_string(),
            label: "cpu_usage".to_string(),
        };
        let metric = MetricEntry {
            key: metric_key,
            value: 1.0,
        };
        metrics.push(metric);
        metrics_store.store_metrics(Some(metrics), node_addr_1);

        let metrics = metrics_store.get_metrics_for_node(node_addr_0);
        assert_eq!(metrics.get("task_1").unwrap().get("cpu_usage"), Some(&1.0));
        assert_eq!(metrics.get("task_2"), None);

        let metrics_1 = metrics_store.get_metrics_for_node(node_addr_1);
        assert_eq!(metrics_1.get("task_1").unwrap().get("cpu_usage"), Some(&1.0));
        assert_eq!(metrics_1.get("task_2").unwrap().get("cpu_usage"), Some(&1.0));
    }
}
