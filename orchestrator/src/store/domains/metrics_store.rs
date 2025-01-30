use crate::store::core::RedisStore;
use alloy::primitives::Address;
use log::error;
use redis::Commands;
use redis::RedisResult;
use shared::models::metric::MetricsMap;
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

    pub fn store_metrics(&self, metric: Option<MetricsMap>, sender_address: Address) {
        if metric.is_none() {
            return;
        }
        let metrics = metric.unwrap();

        for (task_id, metrics) in metrics {
            for (label, metric) in metrics {
                let key = format!("{}:{}:{}", ORCHESTRATOR_METRICS_STORE, task_id, label);
                let mut con = self.redis.client.get_connection().unwrap();
                if let Err(err) =
                    con.hset(key, sender_address.to_string(), metric.value.to_string())
                        as RedisResult<()>
                {
                    error!("Could not update metric value in redis: {}", err);
                }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests::helper::create_test_app_state;
    use shared::models::metric::Metric;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_store_metrics() {
        let app_state = create_test_app_state().await;
        let metrics_store = app_state.store_context.metrics_store.clone();

        let mut taskid_metrics_store = MetricsMap::new();
        let task_id = "task_1";
        let metric_key = "cpu_usage".to_string();
        let metric = Metric::new(metric_key.clone(), 1.0, task_id.to_string()).unwrap();
        let mut metrics_map: HashMap<String, Metric> = HashMap::new();
        metrics_map.insert(metric_key.clone(), metric);
        taskid_metrics_store.insert(task_id.to_string(), metrics_map);
        metrics_store.store_metrics(Some(taskid_metrics_store), Address::ZERO);

        let mut taskid_metrics_store = MetricsMap::new();
        let task_id = "task_0";
        let metric_key = "cpu_usage".to_string();
        let metric = Metric::new(metric_key.clone(), 2.0, task_id.to_string()).unwrap();
        let mut metrics_map_0: HashMap<String, Metric> = HashMap::new();
        metrics_map_0.insert(metric_key.clone(), metric);
        taskid_metrics_store.insert(task_id.to_string(), metrics_map_0);
        metrics_store.store_metrics(Some(taskid_metrics_store), Address::ZERO);

        let metrics: HashMap<String, f64> = metrics_store.get_aggregate_metrics_for_task("task_1");
        assert_eq!(metrics.get("cpu_usage"), Some(&1.0));
        let metrics: HashMap<String, f64> = metrics_store.get_aggregate_metrics_for_all_tasks();
        assert_eq!(metrics.get("cpu_usage"), Some(&3.0));
    }
}
