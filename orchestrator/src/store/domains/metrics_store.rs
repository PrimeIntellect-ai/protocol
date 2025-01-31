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

    fn clean_label(&self, label: &str) -> String {
        label.replace(":", "")
    }

    pub fn store_metrics(&self, metric: Option<MetricsMap>, sender_address: Address) {
        if metric.is_none() {
            return;
        }
        let metrics = metric.unwrap();

        for (key, metric) in metrics {
            let cleaned_label = self.clean_label(&key.label);
            let redis_key = format!(
                "{}:{}:{}",
                ORCHESTRATOR_METRICS_STORE, key.task_id, cleaned_label
            );
            let mut con = self.redis.client.get_connection().unwrap();
            if let Err(err) = con.hset(
                redis_key,
                sender_address.to_string(),
                metric.value.to_string(),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests::helper::create_test_app_state;
    use shared::models::metric::{Metric, MetricKey};
    #[tokio::test]
    async fn test_store_metrics() {
        let app_state = create_test_app_state().await;
        let metrics_store = app_state.store_context.metrics_store.clone();

        let mut taskid_metrics_store = MetricsMap::new();
        let task_id = "task_1";
        let metric_key = MetricKey {
            task_id: task_id.to_string(),
            label: "cpu_usage".to_string(),
        };
        let metric = Metric { value: 1.0 };
        taskid_metrics_store.insert(metric_key, metric);
        metrics_store.store_metrics(Some(taskid_metrics_store), Address::ZERO);

        let mut taskid_metrics_store = MetricsMap::new();
        let task_id = "task_0";
        let metric_key = MetricKey {
            task_id: task_id.to_string(),
            label: "cpu_usage".to_string(),
        };
        let metric = Metric { value: 2.0 };
        taskid_metrics_store.insert(metric_key, metric);
        metrics_store.store_metrics(Some(taskid_metrics_store), Address::ZERO);

        let metrics: HashMap<String, f64> = metrics_store.get_aggregate_metrics_for_task("task_1");
        println!("metrics {:?}", metrics);
        assert_eq!(metrics.get("cpu_usage"), Some(&1.0));
        let metrics: HashMap<String, f64> = metrics_store.get_aggregate_metrics_for_all_tasks();
        assert_eq!(metrics.get("cpu_usage"), Some(&3.0));
    }
}
