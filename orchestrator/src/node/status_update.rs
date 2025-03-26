use crate::models::node::{NodeStatus, OrchestratorNode};
use crate::store::core::StoreContext;
use anyhow::Ok;
use log::{debug, error};
use shared::web3::contracts::core::builder::Contracts;
use std::result::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;

pub struct NodeStatusUpdater {
    store_context: Arc<StoreContext>,
    update_interval: u64,
    missing_heartbeat_threshold: u32,
    contracts: Arc<Contracts>,
    pool_id: u32,
    disable_ejection: bool,
}

impl NodeStatusUpdater {
    pub fn new(
        store_context: Arc<StoreContext>,
        update_interval: u64,
        missing_heartbeat_threshold: Option<u32>,
        contracts: Arc<Contracts>,
        pool_id: u32,
        disable_ejection: bool,
    ) -> Self {
        Self {
            store_context,
            update_interval,
            missing_heartbeat_threshold: missing_heartbeat_threshold.unwrap_or(3),
            contracts,
            pool_id,
            disable_ejection,
        }
    }

    pub async fn run(&self) -> Result<(), anyhow::Error> {
        let mut interval = interval(Duration::from_secs(self.update_interval));

        loop {
            interval.tick().await;
            debug!("Running NodeStatusUpdater to process nodes heartbeats");
            if let Err(e) = self.process_nodes().await {
                error!("Error processing nodes: {}", e);
            }
            if let Err(e) = self.sync_chain_with_nodes().await {
                error!("Error syncing chain with nodes: {}", e);
            }
        }
    }

    async fn is_node_in_pool(&self, node: &OrchestratorNode) -> bool {
        let node_in_pool: bool = match self
            .contracts
            .compute_pool
            .is_node_in_pool(self.pool_id, node.address)
            .await
        {
            Result::Ok(result) => result,
            Result::Err(e) => {
                println!("Error checking if node is in pool: {}", e);
                false
            }
        };
        node_in_pool
    }

    pub async fn sync_chain_with_nodes(&self) -> Result<(), anyhow::Error> {
        let nodes = self.store_context.node_store.get_nodes();
        for node in nodes {
            if node.status == NodeStatus::Dead {
                let node_in_pool = self.is_node_in_pool(&node).await;
                if node_in_pool {
                    if !self.disable_ejection {
                        let tx = self
                            .contracts
                            .compute_pool
                            .eject_node(self.pool_id, node.address)
                            .await;
                        match tx {
                            Result::Ok(_) => {
                                println!("Ejected node: {:?}", node.address);
                            }
                            Result::Err(e) => {
                                println!("Error ejecting node: {}", e);
                            }
                        }
                    } else {
                        println!(
                            "Ejection is disabled, skipping ejection of node: {:?}",
                            node.address
                        );
                    }
                }
            }
        }
        Ok(())
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
                Some(beat) => {
                    // We have a heartbeat
                    if let Some(version) = &beat.version {
                        if node.version.as_ref() != Some(version) {
                            let _: () = self
                                .store_context
                                .node_store
                                .update_node_version(&node.address, version);
                        }
                    }

                    if node.status == NodeStatus::Unhealthy
                        || node.status == NodeStatus::WaitingForHeartbeat
                    {
                        node.status = NodeStatus::Healthy;
                        let _: () = self
                            .store_context
                            .node_store
                            .update_node_status(&node.address, NodeStatus::Healthy);
                    } else if node.status == NodeStatus::Dead {
                        // Node is dead but we receive a heartbeat
                        // Source of truth is the chain, if the node is still on chain
                        // this might be a state issue / recovery of the orchestrator process.
                        // Reinviting the node will fail as its already part of the pool hence we set status to Healthy
                        let is_node_in_pool = self.is_node_in_pool(&node).await;
                        if is_node_in_pool {
                            node.status = NodeStatus::Healthy;
                        } else {
                            node.status = NodeStatus::Discovered;
                        }
                        let _: () = self
                            .store_context
                            .node_store
                            .update_node_status(&node.address, node.status);
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
    use crate::api::tests::helper::setup_contract;
    use crate::models::node::NodeStatus;
    use crate::models::node::OrchestratorNode;
    use alloy::primitives::Address;
    use shared::models::heartbeat::HeartbeatRequest;
    use std::str::FromStr;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_node_status_updater_runs() {
        let app_state = create_test_app_state().await;
        let contracts = setup_contract();

        let updater = NodeStatusUpdater::new(
            app_state.store_context.clone(),
            5,
            None,
            Arc::new(contracts),
            0,
            false,
        );
        let node = OrchestratorNode {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::WaitingForHeartbeat,
            task_id: None,
            task_state: None,
            version: None,
            last_status_change: None,
        };

        let _: () = app_state.store_context.node_store.add_node(node.clone());
        let heartbeat = HeartbeatRequest {
            address: node.address.to_string(),
            task_id: None,
            task_state: None,
            metrics: None,
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        };
        let _: () = app_state.store_context.heartbeat_store.beat(&heartbeat);

        let node = app_state
            .store_context
            .node_store
            .get_node(&node.address)
            .unwrap();
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
        assert_eq!(node.status, NodeStatus::Healthy);
        assert_ne!(node.last_status_change, None);
    }

    #[tokio::test]
    async fn test_node_status_updater_runs_with_unhealthy_node() {
        let app_state = create_test_app_state().await;
        let contracts = setup_contract();

        let node = OrchestratorNode {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Healthy,
            task_id: None,
            task_state: None,
            version: None,
            last_status_change: None,
        };

        let _: () = app_state.store_context.node_store.add_node(node.clone());
        let updater = NodeStatusUpdater::new(
            app_state.store_context.clone(),
            5,
            None,
            Arc::new(contracts),
            0,
            false,
        );
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
        assert_eq!(node.status, NodeStatus::Unhealthy);
        assert_ne!(node.last_status_change, None);
    }

    #[tokio::test]
    async fn test_node_status_with_unhealthy_node_but_no_counter() {
        let app_state = create_test_app_state().await;
        let contracts = setup_contract();

        let node = OrchestratorNode {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Unhealthy,
            task_id: None,
            task_state: None,
            version: None,
            last_status_change: None,
        };

        let _: () = app_state.store_context.node_store.add_node(node.clone());
        let updater = NodeStatusUpdater::new(
            app_state.store_context.clone(),
            5,
            None,
            Arc::new(contracts),
            0,
            false,
        );
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
        assert_eq!(node.status, NodeStatus::Unhealthy);
        let counter = app_state
            .store_context
            .heartbeat_store
            .get_unhealthy_counter(&node.address);
        assert_eq!(counter, 1);
        assert_eq!(node.last_status_change, None);
    }

    #[tokio::test]
    async fn test_node_status_updater_runs_with_dead_node() {
        let app_state = create_test_app_state().await;
        let contracts = setup_contract();

        let node = OrchestratorNode {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Unhealthy,
            task_id: None,
            task_state: None,
            version: None,
            last_status_change: None,
        };

        let _: () = app_state.store_context.node_store.add_node(node.clone());
        let _: () = app_state
            .store_context
            .heartbeat_store
            .set_unhealthy_counter(&node.address, 2);

        let updater = NodeStatusUpdater::new(
            app_state.store_context.clone(),
            5,
            None,
            Arc::new(contracts),
            0,
            false,
        );
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
        assert_eq!(node.status, NodeStatus::Dead);
        assert_ne!(node.last_status_change, None);
    }

    #[tokio::test]
    async fn transition_from_unhealthy_to_healthy() {
        let app_state = create_test_app_state().await;
        let contracts = setup_contract();

        let node = OrchestratorNode {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Unhealthy,
            task_id: None,
            task_state: None,
            version: None,
            last_status_change: None,
        };
        let _: () = app_state
            .store_context
            .heartbeat_store
            .set_unhealthy_counter(&node.address, 2);

        let heartbeat = HeartbeatRequest {
            address: node.address.to_string(),
            task_id: None,
            task_state: None,
            metrics: None,
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        };
        let _: () = app_state.store_context.heartbeat_store.beat(&heartbeat);
        let _: () = app_state.store_context.node_store.add_node(node.clone());

        let updater = NodeStatusUpdater::new(
            app_state.store_context.clone(),
            5,
            None,
            Arc::new(contracts),
            0,
            false,
        );
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
        assert_ne!(node.last_status_change, None);
        let counter = app_state
            .store_context
            .heartbeat_store
            .get_unhealthy_counter(&node.address);
        assert_eq!(counter, 0);
    }

    #[tokio::test]
    async fn test_multiple_nodes_with_diff_status() {
        let app_state = create_test_app_state().await;
        let contracts = setup_contract();

        let node1 = OrchestratorNode {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Unhealthy,
            task_id: None,
            task_state: None,
            version: None,
            last_status_change: None,
        };
        let _: () = app_state
            .store_context
            .heartbeat_store
            .set_unhealthy_counter(&node1.address, 1);
        let _: () = app_state.store_context.node_store.add_node(node1.clone());

        let node2 = OrchestratorNode {
            address: Address::from_str("0x0000000000000000000000000000000000000001").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Healthy,
            task_id: None,
            task_state: None,
            version: None,
            last_status_change: None,
        };

        let _: () = app_state.store_context.node_store.add_node(node2.clone());

        let updater = NodeStatusUpdater::new(
            app_state.store_context.clone(),
            5,
            None,
            Arc::new(contracts),
            0,
            false,
        );
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

    #[tokio::test]
    async fn test_node_rediscovery_after_death() {
        let app_state = create_test_app_state().await;
        let contracts = setup_contract();

        let node = OrchestratorNode {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Unhealthy,
            task_id: None,
            task_state: None,
            version: None,
            last_status_change: None,
        };

        let _: () = app_state.store_context.node_store.add_node(node.clone());
        let _: () = app_state
            .store_context
            .heartbeat_store
            .set_unhealthy_counter(&node.address, 2);

        let updater = NodeStatusUpdater::new(
            app_state.store_context.clone(),
            5,
            None,
            Arc::new(contracts),
            0,
            false,
        );
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
        assert_eq!(node.status, NodeStatus::Dead);

        let heartbeat = HeartbeatRequest {
            address: node.address.to_string(),
            task_id: None,
            task_state: None,
            metrics: None,
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        };
        let _: () = app_state.store_context.heartbeat_store.beat(&heartbeat);

        sleep(Duration::from_secs(5)).await;

        let node = app_state
            .store_context
            .node_store
            .get_node(&node.address)
            .unwrap();
        assert_eq!(node.status, NodeStatus::Discovered);
    }
}
