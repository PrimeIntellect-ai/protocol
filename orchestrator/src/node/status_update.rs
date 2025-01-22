use crate::store::redis::RedisStore;
use crate::types::Node;
use crate::types::NodeStatus;
use crate::types::ORCHESTRATOR_BASE_KEY;
use crate::types::ORCHESTRATOR_HEARTBEAT_KEY;
use crate::types::ORCHESTRATOR_UNHEALTHY_COUNTER_KEY;
use anyhow::Ok;
use redis::Commands;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;

pub struct NodeStatusUpdater {
    store: Arc<RedisStore>,
    update_interval: u64,
    missing_heartbeat_threshold: u32,
}

impl NodeStatusUpdater {
    pub fn new(
        store: Arc<RedisStore>,
        update_interval: u64,
        missing_heartbeat_threshold: Option<u32>,
    ) -> Self {
        Self {
            store,
            update_interval,
            missing_heartbeat_threshold: missing_heartbeat_threshold.unwrap_or(3),
        }
    }

    pub async fn run(&self) -> Result<(), anyhow::Error> {
        let mut interval = interval(Duration::from_secs(self.update_interval));

        loop {
            interval.tick().await;
            println!("Running NodeStatusUpdater to process nodes heartbeats");
            if let Err(e) = self.process_nodes().await {
                eprintln!("Error processing nodes: {}", e);
            }
        }
    }

    pub async fn process_nodes(&self) -> Result<(), anyhow::Error> {
        let mut con = self.store.client.get_connection()?;
        let keys: Vec<String> = con.keys(format!("{}:*", ORCHESTRATOR_BASE_KEY))?;
        let nodes: Vec<Node> = keys
            .iter()
            .map(|key| con.get::<_, String>(key).ok())
            .map(|node_json| Node::from_string(&node_json.unwrap()))
            .collect();

        for node in nodes {
            let mut node = node.clone();
            let heartbeat_key = format!("{}:{}", ORCHESTRATOR_HEARTBEAT_KEY, node.address);
            let heartbeat = con
                .get::<_, String>(&heartbeat_key)
                .unwrap_or_else(|_| "".to_string());

            let unhealthy_counter_key =
                format!("{}:{}", ORCHESTRATOR_UNHEALTHY_COUNTER_KEY, node.address);
            let unhealthy_counter: u32 = con.get::<_, u32>(&unhealthy_counter_key).unwrap_or(0);

            if heartbeat.is_empty() {
                // TODO: Cover case waiting for heartbeat
                if node.status == NodeStatus::Healthy {
                    node.status = NodeStatus::Unhealthy;
                    println!("Setting node {} from healthy to unhealthy", node.address);
                    let _: () = con.set(node.orchestrator_key(), node.to_string()).unwrap();
                    let _: () = con.set(unhealthy_counter_key, "1").unwrap(); // Set counter to 1 when unhealthy
                } else if node.status == NodeStatus::Unhealthy {
                    let count_to_dead = if unhealthy_counter < self.missing_heartbeat_threshold {
                        self.missing_heartbeat_threshold - unhealthy_counter
                    } else {
                        0
                    };
                    if unhealthy_counter >= self.missing_heartbeat_threshold {
                        println!("Node {} is dead", node.address);
                        node.status = NodeStatus::Dead;
                        let _: () = con.set(node.orchestrator_key(), node.to_string()).unwrap();
                        // TODO: Should report this to chain
                    } else {
                        let seconds_to_dead = count_to_dead as u64 * self.update_interval;
                        println!("Node {} is unhealthy - require heartbeat in {}s before marking as dead: {} ", node.address, seconds_to_dead, unhealthy_counter);
                        let _: () = con
                            .set(unhealthy_counter_key, (unhealthy_counter + 1).to_string())
                            .unwrap();
                    }
                }
            } else {
                if node.status == NodeStatus::Unhealthy
                    || node.status == NodeStatus::WaitingForHeartbeat
                {
                    node.status = NodeStatus::Healthy;
                    let _: () = con.set(node.orchestrator_key(), node.to_string()).unwrap();
                }
                let _: () = con.del(&unhealthy_counter_key).unwrap();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::NodeStatus;
    use crate::types::ORCHESTRATOR_HEARTBEAT_KEY;
    use alloy::primitives::Address;
    use std::str::FromStr;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_node_status_updater_runs() {
        let store = Arc::new(RedisStore::new_test());
        let mut con = store.client.get_connection().unwrap();

        let updater = NodeStatusUpdater::new(store.clone(), 5, None);
        let node = Node {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::WaitingForHeartbeat,
            task_id: None,
            task_state: None,
        };

        let heartbeat_key = format!("{}:{}", ORCHESTRATOR_HEARTBEAT_KEY, node.address);
        let _: () = con
            .set_options(
                &heartbeat_key,
                "1",
                redis::SetOptions::default().with_expiration(redis::SetExpiry::EX(60)),
            )
            .unwrap();

        let _: () = con.set(node.orchestrator_key(), node.to_string()).unwrap();

        let node = con.get::<_, String>(node.orchestrator_key()).unwrap();
        let node = Node::from_string(&node);
        println!("Node: {:?}", node);
        assert_eq!(node.status, NodeStatus::WaitingForHeartbeat);

        tokio::spawn(async move {
            updater
                .run()
                .await
                .expect("Failed to run NodeStatusUpdater");
        });

        sleep(Duration::from_secs(2)).await;

        let node = con.get::<_, String>(node.orchestrator_key()).unwrap();
        let node = Node::from_string(&node);
        println!("Node: {:?}", node);
        assert_eq!(node.status, NodeStatus::Healthy);
    }

    #[tokio::test]
    async fn test_node_status_updater_runs_with_unhealthy_node() {
        let store = Arc::new(RedisStore::new_test());
        let mut con = store.client.get_connection().unwrap();

        let node = Node {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Healthy,
            task_id: None,
            task_state: None,
        };

        let _: () = con.set(node.orchestrator_key(), node.to_string()).unwrap();
        let updater = NodeStatusUpdater::new(store.clone(), 5, None);
        tokio::spawn(async move {
            updater
                .run()
                .await
                .expect("Failed to run NodeStatusUpdater");
        });

        sleep(Duration::from_secs(2)).await;

        let redis_node = con.get::<_, String>(node.orchestrator_key()).unwrap();
        println!("Node: {:?}", redis_node);
        let node = Node::from_string(&redis_node);
        println!("Node: {:?}", node);
        assert_eq!(node.status, NodeStatus::Unhealthy);
    }

    #[tokio::test]
    async fn test_node_status_with_unhealthy_node_but_no_counter() {
        let store = Arc::new(RedisStore::new_test());
        let mut con = store.client.get_connection().unwrap();

        let node = Node {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Unhealthy,
            task_id: None,
            task_state: None,
        };

        let _: () = con.set(node.orchestrator_key(), node.to_string()).unwrap();
        let updater = NodeStatusUpdater::new(store.clone(), 5, None);
        tokio::spawn(async move {
            updater
                .run()
                .await
                .expect("Failed to run NodeStatusUpdater");
        });

        sleep(Duration::from_secs(2)).await;

        let redis_node = con.get::<_, String>(node.orchestrator_key()).unwrap();
        println!("Node: {:?}", redis_node);
        let node = Node::from_string(&redis_node);
        println!("Node: {:?}", node);
        assert_eq!(node.status, NodeStatus::Unhealthy);
        let unhealthy_counter_key =
            format!("{}:{}", ORCHESTRATOR_UNHEALTHY_COUNTER_KEY, node.address);
        let counter = con.get::<_, u32>(unhealthy_counter_key).unwrap();
        println!("Counter: {:?}", counter);
        assert_eq!(counter, 1);
    }

    #[tokio::test]
    async fn test_node_status_updater_runs_with_dead_node() {
        let store = Arc::new(RedisStore::new_test());
        let mut con = store.client.get_connection().unwrap();

        let node = Node {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Unhealthy,
            task_id: None,
            task_state: None,
        };

        let _: () = con.set(node.orchestrator_key(), node.to_string()).unwrap();

        let unhealthy_counter_key =
            format!("{}:{}", ORCHESTRATOR_UNHEALTHY_COUNTER_KEY, node.address);
        let _: () = con.set(unhealthy_counter_key, "3").unwrap();
        let updater = NodeStatusUpdater::new(store.clone(), 5, None);
        tokio::spawn(async move {
            updater
                .run()
                .await
                .expect("Failed to run NodeStatusUpdater");
        });

        sleep(Duration::from_secs(2)).await;

        let redis_node = con.get::<_, String>(node.orchestrator_key()).unwrap();
        println!("Node: {:?}", redis_node);
        let node = Node::from_string(&redis_node);
        println!("Node: {:?}", node);
        assert_eq!(node.status, NodeStatus::Dead);
    }

    #[tokio::test]
    async fn transition_from_unhealthy_to_healthy() {
        let store = Arc::new(RedisStore::new_test());
        let mut con = store.client.get_connection().unwrap();

        let node = Node {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Unhealthy,
            task_id: None,
            task_state: None,
        };
        let unhealthy_counter_key =
            format!("{}:{}", ORCHESTRATOR_UNHEALTHY_COUNTER_KEY, node.address);
        let heartbeat_key = format!("{}:{}", ORCHESTRATOR_HEARTBEAT_KEY, node.address);
        let _: () = con.set(unhealthy_counter_key, "2").unwrap();
        let _: () = con.set(heartbeat_key, "1").unwrap();
        let _: () = con.set(node.orchestrator_key(), node.to_string()).unwrap();

        let updater = NodeStatusUpdater::new(store.clone(), 5, None);
        tokio::spawn(async move {
            updater
                .run()
                .await
                .expect("Failed to run NodeStatusUpdater");
        });

        sleep(Duration::from_secs(2)).await;

        let redis_node = con.get::<_, String>(node.orchestrator_key()).unwrap();
        let node = Node::from_string(&redis_node);
        assert_eq!(node.status, NodeStatus::Healthy);

        let unhealthy_counter_key =
            format!("{}:{}", ORCHESTRATOR_UNHEALTHY_COUNTER_KEY, node.address);
        let exists: Result<bool, _> = con.exists(unhealthy_counter_key);
        assert!(
            !exists.unwrap_or(false),
            "Unhealthy counter key should not exist"
        );
    }

    #[tokio::test]
    async fn test_multiple_nodes_with_diff_status() {
        let store = Arc::new(RedisStore::new_test());
        let mut con = store.client.get_connection().unwrap();

        let keys: Vec<String> = con.keys(format!("{}:*", ORCHESTRATOR_BASE_KEY)).unwrap();
        println!("Keys: {:?}", keys);
        if !keys.is_empty() {
            panic!("Keys should be empty");
        }

        let node1 = Node {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Unhealthy,
            task_id: None,
            task_state: None,
        };
        let node1_unhealthy_counter_key =
            format!("{}:{}", ORCHESTRATOR_UNHEALTHY_COUNTER_KEY, node1.address);
        let _: () = con.set(node1_unhealthy_counter_key, "1").unwrap();

        let node2 = Node {
            address: Address::from_str("0x0000000000000000000000000000000000000001").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Healthy,
            task_id: None,
            task_state: None,
        };

        let _: () = con
            .set(node1.orchestrator_key(), node1.to_string())
            .unwrap();
        let _: () = con
            .set(node2.orchestrator_key(), node2.to_string())
            .unwrap();

        let updater = NodeStatusUpdater::new(store.clone(), 5, None);
        tokio::spawn(async move {
            updater
                .run()
                .await
                .expect("Failed to run NodeStatusUpdater");
        });

        sleep(Duration::from_secs(2)).await;

        let redis_node1 = con.get::<_, String>(node1.orchestrator_key()).unwrap();
        let node1 = Node::from_string(&redis_node1);
        let redis_node1_unhealthy_counter =
            format!("{}:{}", ORCHESTRATOR_UNHEALTHY_COUNTER_KEY, node1.address);
        assert_eq!(node1.status, NodeStatus::Unhealthy);
        let counter = con.get::<_, u32>(redis_node1_unhealthy_counter).unwrap();
        assert_eq!(counter, 2);

        let redis_node2 = con.get::<_, String>(node2.orchestrator_key()).unwrap();
        let node2 = Node::from_string(&redis_node2);
        let redis_node2_unhealthy_counter =
            format!("{}:{}", ORCHESTRATOR_UNHEALTHY_COUNTER_KEY, node2.address);
        assert_eq!(node2.status, NodeStatus::Unhealthy);
        let counter = con.get::<_, u32>(redis_node2_unhealthy_counter).unwrap();
        assert_eq!(counter, 1);
    }
}
