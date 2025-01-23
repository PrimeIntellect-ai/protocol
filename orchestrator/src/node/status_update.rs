use crate::store::context::StoreContext;
use crate::types::NodeStatus;
use anyhow::Ok;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;

pub struct NodeStatusUpdater {
    store_context: Arc<StoreContext>,
    update_interval: u64,
    missing_heartbeat_threshold: u32,
}

impl NodeStatusUpdater {
    pub fn new(
        store_context: Arc<StoreContext>,
        update_interval: u64,
        missing_heartbeat_threshold: Option<u32>,
    ) -> Self {
        Self {
            store_context,
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
        let nodes = self.store_context.node_store.get_nodes();
        for node in nodes {
            let mut node = node.clone();
            let heartbeat = self
                .store_context
                .heartbeat_store
                .get_heartbeat(&node.address);
            let unhealthy_counter: u32 = self
                .store_context
                .heartbeat_store
                .get_unhealthy_counter(&node.address);

            match heartbeat {
                Some(_) => {
                    // We have a heartbeat
                    if node.status == NodeStatus::Unhealthy
                        || node.status == NodeStatus::WaitingForHeartbeat
                    {
                        node.status = NodeStatus::Healthy;
                        let _: () = self
                            .store_context
                            .node_store
                            .update_node_status(&node.address, NodeStatus::Healthy);
                    }
                    let _: () = self
                        .store_context
                        .heartbeat_store
                        .clear_unhealthy_counter(&node.address);
                }
                None => {
                    // We don't have a heartbeat, increment unhealthy counter
                    self.store_context
                        .heartbeat_store
                        .increment_unhealthy_counter(&node.address);

                    match node.status {
                        NodeStatus::Healthy => {
                            self.store_context
                                .node_store
                                .update_node_status(&node.address, NodeStatus::Unhealthy);
                        }
                        NodeStatus::Unhealthy => {
                            if unhealthy_counter + 1 >= self.missing_heartbeat_threshold {
                                // TODO Report death to chain
                                self.store_context
                                    .node_store
                                    .update_node_status(&node.address, NodeStatus::Dead);
                            }
                        }
                        _ => (),
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests::helper::create_test_app_state;
    use crate::types::Node;
    use crate::types::NodeStatus;
    use alloy::primitives::Address;
    use std::str::FromStr;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_node_status_updater_runs() {
        let app_state = create_test_app_state().await;

        let updater = NodeStatusUpdater::new(app_state.store_context.clone(), 5, None);
        let node = Node {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::WaitingForHeartbeat,
            task_id: None,
            task_state: None,
        };

        let _: () = app_state.store_context.node_store.add_node(node.clone());
        let _: () = app_state.store_context.heartbeat_store.beat(&node.address);

        let node = app_state
            .store_context
            .node_store
            .get_node(&node.address)
            .unwrap();
        println!("Node: {:?}", node);
        assert_eq!(node.status, NodeStatus::WaitingForHeartbeat);

        tokio::spawn(async move {
            updater
                .run()
                .await
                .expect("Failed to run NodeStatusUpdater");
        });

        sleep(Duration::from_secs(2)).await;

        let node = app_state
            .store_context
            .node_store
            .get_node(&node.address)
            .unwrap();
        println!("Node: {:?}", node);
        assert_eq!(node.status, NodeStatus::Healthy);
    }

    #[tokio::test]
    async fn test_node_status_updater_runs_with_unhealthy_node() {
        let app_state = create_test_app_state().await;

        let node = Node {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Healthy,
            task_id: None,
            task_state: None,
        };

        let _: () = app_state.store_context.node_store.add_node(node.clone());
        let updater = NodeStatusUpdater::new(app_state.store_context.clone(), 5, None);
        tokio::spawn(async move {
            updater
                .run()
                .await
                .expect("Failed to run NodeStatusUpdater");
        });

        sleep(Duration::from_secs(2)).await;

        let node = app_state
            .store_context
            .node_store
            .get_node(&node.address)
            .unwrap();
        println!("Node: {:?}", node);
        assert_eq!(node.status, NodeStatus::Unhealthy);
    }

    #[tokio::test]
    async fn test_node_status_with_unhealthy_node_but_no_counter() {
        let app_state = create_test_app_state().await;

        let node = Node {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Unhealthy,
            task_id: None,
            task_state: None,
        };

        let _: () = app_state.store_context.node_store.add_node(node.clone());
        let updater = NodeStatusUpdater::new(app_state.store_context.clone(), 5, None);
        tokio::spawn(async move {
            updater
                .run()
                .await
                .expect("Failed to run NodeStatusUpdater");
        });

        sleep(Duration::from_secs(2)).await;

        let node = app_state
            .store_context
            .node_store
            .get_node(&node.address)
            .unwrap();
        println!("Node: {:?}", node);
        assert_eq!(node.status, NodeStatus::Unhealthy);
        let counter = app_state
            .store_context
            .heartbeat_store
            .get_unhealthy_counter(&node.address);
        println!("Counter: {:?}", counter);
        assert_eq!(counter, 1);
    }

    #[tokio::test]
    async fn test_node_status_updater_runs_with_dead_node() {
        let app_state = create_test_app_state().await;

        let node = Node {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Unhealthy,
            task_id: None,
            task_state: None,
        };

        let _: () = app_state.store_context.node_store.add_node(node.clone());
        let _: () = app_state
            .store_context
            .heartbeat_store
            .set_unhealthy_counter(&node.address, 2);

        let updater = NodeStatusUpdater::new(app_state.store_context.clone(), 5, None);
        tokio::spawn(async move {
            updater
                .run()
                .await
                .expect("Failed to run NodeStatusUpdater");
        });

        sleep(Duration::from_secs(2)).await;

        let node = app_state
            .store_context
            .node_store
            .get_node(&node.address)
            .unwrap();
        println!("Node: {:?}", node);
        assert_eq!(node.status, NodeStatus::Dead);
    }

    #[tokio::test]
    async fn transition_from_unhealthy_to_healthy() {
        let app_state = create_test_app_state().await;

        let node = Node {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Unhealthy,
            task_id: None,
            task_state: None,
        };
        let _: () = app_state
            .store_context
            .heartbeat_store
            .set_unhealthy_counter(&node.address, 2);
        let _: () = app_state.store_context.heartbeat_store.beat(&node.address);
        let _: () = app_state.store_context.node_store.add_node(node.clone());

        let updater = NodeStatusUpdater::new(app_state.store_context.clone(), 5, None);
        tokio::spawn(async move {
            updater
                .run()
                .await
                .expect("Failed to run NodeStatusUpdater");
        });

        sleep(Duration::from_secs(2)).await;

        let node = app_state
            .store_context
            .node_store
            .get_node(&node.address)
            .unwrap();
        assert_eq!(node.status, NodeStatus::Healthy);

        let counter = app_state
            .store_context
            .heartbeat_store
            .get_unhealthy_counter(&node.address);
        assert_eq!(counter, 0);
    }

    #[tokio::test]
    async fn test_multiple_nodes_with_diff_status() {
        let app_state = create_test_app_state().await;

        let node1 = Node {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Unhealthy,
            task_id: None,
            task_state: None,
        };
        let _: () = app_state
            .store_context
            .heartbeat_store
            .set_unhealthy_counter(&node1.address, 1);
        let _: () = app_state.store_context.node_store.add_node(node1.clone());

        let node2 = Node {
            address: Address::from_str("0x0000000000000000000000000000000000000001").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Healthy,
            task_id: None,
            task_state: None,
        };

        let _: () = app_state.store_context.node_store.add_node(node2.clone());

        let updater = NodeStatusUpdater::new(app_state.store_context.clone(), 5, None);
        tokio::spawn(async move {
            updater
                .run()
                .await
                .expect("Failed to run NodeStatusUpdater");
        });

        sleep(Duration::from_secs(2)).await;

        let node1 = app_state
            .store_context
            .node_store
            .get_node(&node1.address)
            .unwrap();
        assert_eq!(node1.status, NodeStatus::Unhealthy);
        let counter = app_state
            .store_context
            .heartbeat_store
            .get_unhealthy_counter(&node1.address);
        assert_eq!(counter, 2);

        let node2 = app_state
            .store_context
            .node_store
            .get_node(&node2.address)
            .unwrap();
        assert_eq!(node2.status, NodeStatus::Unhealthy);
        let counter = app_state
            .store_context
            .heartbeat_store
            .get_unhealthy_counter(&node2.address);
        assert_eq!(counter, 1);
    }
}
