use crate::metrics::MetricsContext;
use crate::models::node::{NodeStatus, OrchestratorNode};
use crate::plugins::StatusUpdatePlugin;
use crate::store::core::StoreContext;
use crate::utils::loop_heartbeats::LoopHeartbeats;
use log::{debug, error, info};
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::wallet::WalletProvider;
use std::result::Result;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::interval;

pub struct NodeStatusUpdater {
    store_context: Arc<StoreContext>,
    update_interval: u64,
    missing_heartbeat_threshold: u32,
    contracts: Contracts<WalletProvider>,
    pool_id: u32,
    disable_ejection: bool,
    heartbeats: Arc<LoopHeartbeats>,
    plugins: Vec<Box<dyn StatusUpdatePlugin>>,
    metrics: Arc<MetricsContext>,
}

impl NodeStatusUpdater {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store_context: Arc<StoreContext>,
        update_interval: u64,
        missing_heartbeat_threshold: Option<u32>,
        contracts: Contracts<WalletProvider>,
        pool_id: u32,
        disable_ejection: bool,
        heartbeats: Arc<LoopHeartbeats>,
        plugins: Vec<Box<dyn StatusUpdatePlugin>>,
        metrics: Arc<MetricsContext>,
    ) -> Self {
        Self {
            store_context,
            update_interval,
            missing_heartbeat_threshold: missing_heartbeat_threshold.unwrap_or(3),
            contracts,
            pool_id,
            disable_ejection,
            heartbeats,
            plugins,
            metrics,
        }
    }

    pub async fn run(&self) -> Result<(), anyhow::Error> {
        let mut interval = interval(Duration::from_secs(self.update_interval));

        loop {
            interval.tick().await;
            debug!("Running NodeStatusUpdater to process nodes heartbeats");
            if let Err(e) = self.process_nodes().await {
                error!("Error processing nodes: {e}");
            }
            if let Err(e) = self.sync_chain_with_nodes().await {
                error!("Error syncing chain with nodes: {e}");
            }
            self.heartbeats.update_status_updater();
        }
    }

    #[cfg(test)]
    async fn is_node_in_pool(&self, _: &OrchestratorNode) -> bool {
        true
    }

    #[cfg(not(test))]
    async fn is_node_in_pool(&self, node: &OrchestratorNode) -> bool {
        let node_in_pool: bool = (self
            .contracts
            .compute_pool
            .is_node_in_pool(self.pool_id, node.address)
            .await)
            .unwrap_or(false);
        node_in_pool
    }

    async fn sync_dead_node_with_chain(
        &self,
        node: &OrchestratorNode,
    ) -> Result<(), anyhow::Error> {
        let node_in_pool = self.is_node_in_pool(node).await;
        if node_in_pool {
            match self
                .contracts
                .compute_pool
                .eject_node(self.pool_id, node.address)
                .await
            {
                Result::Ok(_) => {
                    info!("Ejected node: {:?}", node.address);
                    return Ok(());
                }
                Result::Err(e) => {
                    error!("Error ejecting node: {e}");
                    return Err(anyhow::anyhow!("Error ejecting node: {}", e));
                }
            }
        } else {
            debug!(
                "Dead Node {:?} is not in pool, skipping ejection",
                node.address
            );
        }
        Ok(())
    }

    pub async fn sync_chain_with_nodes(&self) -> Result<(), anyhow::Error> {
        let nodes = self.store_context.node_store.get_nodes().await?;
        for node in nodes {
            if node.status == NodeStatus::Dead {
                let node_in_pool = self.is_node_in_pool(&node).await;
                debug!("Node {:?} is in pool: {}", node.address, node_in_pool);
                if node_in_pool {
                    if !self.disable_ejection {
                        if let Err(e) = self.sync_dead_node_with_chain(&node).await {
                            error!("Error syncing dead node with chain: {e}");
                        }
                    } else {
                        debug!(
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
        let nodes = self.store_context.node_store.get_nodes().await?;
        for node in nodes {
            let start_time = Instant::now();
            let node = node.clone();
            let old_status = node.status.clone();
            let heartbeat = self
                .store_context
                .heartbeat_store
                .get_heartbeat(&node.address)
                .await?;
            let unhealthy_counter: u32 = self
                .store_context
                .heartbeat_store
                .get_unhealthy_counter(&node.address)
                .await?;

            let is_node_in_pool = self.is_node_in_pool(&node).await;
            let mut status_changed = false;
            let mut new_status = node.status.clone();

            match heartbeat {
                Some(beat) => {
                    // Update version if necessary
                    if let Some(version) = &beat.version {
                        if node.version.as_ref() != Some(version) {
                            if let Err(e) = self
                                .store_context
                                .node_store
                                .update_node_version(&node.address, version)
                                .await
                            {
                                error!("Error updating node version: {e}");
                            }
                        }
                    }

                    // Check if the node is in the pool (needed for status transitions)

                    // If node is Unhealthy or WaitingForHeartbeat:
                    if node.status == NodeStatus::Unhealthy
                        || node.status == NodeStatus::WaitingForHeartbeat
                    {
                        if is_node_in_pool {
                            new_status = NodeStatus::Healthy;
                        } else {
                            // Reset to discovered to init re-invite to pool
                            new_status = NodeStatus::Discovered;
                        }
                        status_changed = true;
                    }
                    // If node is Discovered or Dead:
                    else if node.status == NodeStatus::Discovered
                        || node.status == NodeStatus::Dead
                    {
                        if is_node_in_pool {
                            new_status = NodeStatus::Healthy;
                        } else {
                            new_status = NodeStatus::Discovered;
                        }
                        status_changed = true;
                    }

                    // Clear unhealthy counter on heartbeat receipt
                    if let Err(e) = self
                        .store_context
                        .heartbeat_store
                        .clear_unhealthy_counter(&node.address)
                        .await
                    {
                        error!("Error clearing unhealthy counter: {e}");
                    }
                }
                None => {
                    // We don't have a heartbeat, increment unhealthy counter
                    if let Err(e) = self
                        .store_context
                        .heartbeat_store
                        .increment_unhealthy_counter(&node.address)
                        .await
                    {
                        error!("Error incrementing unhealthy counter: {e}");
                    }

                    match node.status {
                        NodeStatus::Healthy => {
                            new_status = NodeStatus::Unhealthy;
                            status_changed = true;
                        }
                        NodeStatus::Unhealthy => {
                            if unhealthy_counter + 1 >= self.missing_heartbeat_threshold {
                                new_status = NodeStatus::Dead;
                                status_changed = true;
                            }
                        }
                        NodeStatus::Discovered => {
                            if is_node_in_pool {
                                // We have caught a very interesting edge case here.
                                // The node is in pool but does not send heartbeats - maybe due to a downtime of the orchestrator?
                                // Node invites fail now since the node cannot be in pool again.
                                // We have to eject and re-invite - we can simply do this by setting the status to unhealthy. The node will eventually be ejected.
                                new_status = NodeStatus::Unhealthy;
                                status_changed = true;
                            } else {
                                // if we've been trying to invite this node for a while, we eventually give up and mark it as dead
                                // The node will simply be in status discovered again when the discovery svc date > status change date.
                                if unhealthy_counter + 1 > 360 {
                                    new_status = NodeStatus::Dead;
                                    status_changed = true;
                                }
                            }
                        }
                        NodeStatus::WaitingForHeartbeat => {
                            if unhealthy_counter + 1 >= self.missing_heartbeat_threshold {
                                // Unhealthy counter is reset when node is invited
                                // usually it starts directly with heartbeat
                                new_status = NodeStatus::Unhealthy;
                                status_changed = true;
                            }
                        }
                        _ => (),
                    }
                }
            }

            if status_changed {
                // Clean up metrics when node becomes Dead, Ejected, or Banned
                if matches!(
                    &new_status,
                    NodeStatus::Dead | NodeStatus::Ejected | NodeStatus::Banned
                ) {
                    let node_metrics = match self
                        .store_context
                        .metrics_store
                        .get_metrics_for_node(node.address)
                        .await
                    {
                        Ok(metrics) => metrics,
                        Err(e) => {
                            error!("Error getting metrics for node: {e}");
                            Default::default()
                        }
                    };

                    for (task_id, task_metrics) in node_metrics {
                        for (label, _value) in task_metrics {
                            // Remove from Redis metrics store
                            if let Err(e) = self
                                .store_context
                                .metrics_store
                                .delete_metric(&task_id, &label, &node.address.to_string())
                                .await
                            {
                                error!("Error deleting metric: {e}");
                            }
                        }
                    }
                }

                if let Err(e) = self
                    .store_context
                    .node_store
                    .update_node_status(&node.address, new_status)
                    .await
                {
                    error!("Error updating node status: {e}");
                }

                if let Some(updated_node) = self
                    .store_context
                    .node_store
                    .get_node(&node.address)
                    .await?
                {
                    for plugin in self.plugins.iter() {
                        if let Err(e) = plugin
                            .handle_status_change(&updated_node, &old_status)
                            .await
                        {
                            error!("Error handling status change: {e}");
                        }
                    }
                }
            }
            // Record status update execution time
            let duration = start_time.elapsed();
            self.metrics.record_status_update_execution_time(
                &node.address.to_string(),
                duration.as_secs_f64(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests::helper::create_test_app_state_with_metrics;
    use crate::api::tests::helper::setup_contract;
    use crate::models::node::NodeStatus;
    use crate::models::node::OrchestratorNode;
    use crate::ServerMode;
    use alloy::primitives::Address;
    use shared::models::heartbeat::HeartbeatRequest;
    use std::str::FromStr;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_node_status_updater_runs() {
        let app_state = create_test_app_state_with_metrics().await;
        let contracts = setup_contract();
        let mode = ServerMode::Full;
        let updater = NodeStatusUpdater::new(
            app_state.store_context.clone(),
            5,
            None,
            contracts,
            0,
            false,
            Arc::new(LoopHeartbeats::new(&mode)),
            vec![],
            app_state.metrics.clone(),
        );
        let node = OrchestratorNode {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::WaitingForHeartbeat,
            ..Default::default()
        };

        if let Err(e) = app_state
            .store_context
            .node_store
            .add_node(node.clone())
            .await
        {
            error!("Error adding node: {}", e);
        }
        let heartbeat = HeartbeatRequest {
            address: node.address.to_string(),
            task_id: None,
            task_state: None,
            metrics: None,
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            timestamp: None,
            p2p_id: None,
            task_details: None,
        };
        if let Err(e) = app_state
            .store_context
            .heartbeat_store
            .beat(&heartbeat)
            .await
        {
            error!("Heartbeat Error: {}", e);
        }

        let _ = updater.process_nodes().await;

        let node_opt = app_state
            .store_context
            .node_store
            .get_node(&node.address)
            .await
            .unwrap();
        assert!(node_opt.is_some());

        if let Some(node) = node_opt {
            assert_eq!(node.status, NodeStatus::Healthy);
            assert_ne!(node.last_status_change, None);
        }

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
            .await
            .unwrap();
        assert!(node.is_some());
        if let Some(node) = node {
            assert_eq!(node.status, NodeStatus::Healthy);
            assert_ne!(node.last_status_change, None);
        }
    }

    #[tokio::test]
    async fn test_node_status_updater_runs_with_unhealthy_node() {
        let app_state = create_test_app_state_with_metrics().await;
        let contracts = setup_contract();

        let node = OrchestratorNode {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Healthy,
            ..Default::default()
        };

        if let Err(e) = app_state
            .store_context
            .node_store
            .add_node(node.clone())
            .await
        {
            error!("Error adding node: {}", e);
        }
        let mode = ServerMode::Full;
        let updater = NodeStatusUpdater::new(
            app_state.store_context.clone(),
            5,
            None,
            contracts,
            0,
            false,
            Arc::new(LoopHeartbeats::new(&mode)),
            vec![],
            app_state.metrics.clone(),
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
            .await
            .unwrap()
            .unwrap();
        assert_eq!(node.status, NodeStatus::Unhealthy);
        assert_ne!(node.last_status_change, None);
    }

    #[tokio::test]
    async fn test_node_status_with_unhealthy_node_but_no_counter() {
        let app_state = create_test_app_state_with_metrics().await;
        let contracts = setup_contract();

        let node = OrchestratorNode {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Unhealthy,
            ..Default::default()
        };

        if let Err(e) = app_state
            .store_context
            .node_store
            .add_node(node.clone())
            .await
        {
            error!("Error adding node: {}", e);
        }
        let mode = ServerMode::Full;
        let updater = NodeStatusUpdater::new(
            app_state.store_context.clone(),
            5,
            None,
            contracts,
            0,
            false,
            Arc::new(LoopHeartbeats::new(&mode)),
            vec![],
            app_state.metrics.clone(),
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
            .await
            .unwrap()
            .unwrap();
        assert_eq!(node.status, NodeStatus::Unhealthy);
        let counter = app_state
            .store_context
            .heartbeat_store
            .get_unhealthy_counter(&node.address)
            .await
            .unwrap();
        assert_eq!(counter, 1);
        assert_eq!(node.last_status_change, None);
    }

    #[tokio::test]
    async fn test_node_status_updater_runs_with_dead_node() {
        let app_state = create_test_app_state_with_metrics().await;
        let contracts = setup_contract();

        let node = OrchestratorNode {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Unhealthy,
            ..Default::default()
        };

        if let Err(e) = app_state
            .store_context
            .node_store
            .add_node(node.clone())
            .await
        {
            error!("Error adding node: {}", e);
        }
        if let Err(e) = app_state
            .store_context
            .heartbeat_store
            .set_unhealthy_counter(&node.address, 2)
            .await
        {
            error!("Error setting unhealthy counter: {}", e);
        }

        let mode = ServerMode::Full;
        let updater = NodeStatusUpdater::new(
            app_state.store_context.clone(),
            5,
            None,
            contracts,
            0,
            false,
            Arc::new(LoopHeartbeats::new(&mode)),
            vec![],
            app_state.metrics.clone(),
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
            .await
            .unwrap();

        assert!(node.is_some());
        if let Some(node) = node {
            assert_eq!(node.status, NodeStatus::Dead);
            assert_ne!(node.last_status_change, None);
        }
    }

    #[tokio::test]
    async fn transition_from_unhealthy_to_healthy() {
        let app_state = create_test_app_state_with_metrics().await;
        let contracts = setup_contract();

        let node = OrchestratorNode {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Unhealthy,
            ..Default::default()
        };
        if let Err(e) = app_state
            .store_context
            .heartbeat_store
            .set_unhealthy_counter(&node.address, 2)
            .await
        {
            error!("Error setting unhealthy counter: {}", e);
        };

        let heartbeat = HeartbeatRequest {
            address: node.address.to_string(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            task_details: None,
            ..Default::default()
        };
        if let Err(e) = app_state
            .store_context
            .heartbeat_store
            .beat(&heartbeat)
            .await
        {
            error!("Heartbeat Error: {}", e);
        }
        if let Err(e) = app_state
            .store_context
            .node_store
            .add_node(node.clone())
            .await
        {
            error!("Error adding node: {}", e);
        }

        let mode = ServerMode::Full;
        let updater = NodeStatusUpdater::new(
            app_state.store_context.clone(),
            5,
            None,
            contracts,
            0,
            false,
            Arc::new(LoopHeartbeats::new(&mode)),
            vec![],
            app_state.metrics.clone(),
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
            .await
            .unwrap();
        assert!(node.is_some());
        if let Some(node) = node {
            assert_eq!(node.status, NodeStatus::Healthy);
            assert_ne!(node.last_status_change, None);
            let counter = app_state
                .store_context
                .heartbeat_store
                .get_unhealthy_counter(&node.address)
                .await
                .unwrap();
            assert_eq!(counter, 0);
        }
    }

    #[tokio::test]
    async fn test_multiple_nodes_with_diff_status() {
        let app_state = create_test_app_state_with_metrics().await;
        let contracts = setup_contract();

        let node1 = OrchestratorNode {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Unhealthy,
            ..Default::default()
        };
        if let Err(e) = app_state
            .store_context
            .heartbeat_store
            .set_unhealthy_counter(&node1.address, 1)
            .await
        {
            error!("Error setting unhealthy counter: {}", e);
        };
        if let Err(e) = app_state
            .store_context
            .node_store
            .add_node(node1.clone())
            .await
        {
            error!("Error adding node: {}", e);
        }

        let node2 = OrchestratorNode {
            address: Address::from_str("0x0000000000000000000000000000000000000001").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Healthy,
            ..Default::default()
        };

        if let Err(e) = app_state
            .store_context
            .node_store
            .add_node(node2.clone())
            .await
        {
            error!("Error adding node: {}", e);
        }

        let mode = ServerMode::Full;
        let updater = NodeStatusUpdater::new(
            app_state.store_context.clone(),
            5,
            None,
            contracts,
            0,
            false,
            Arc::new(LoopHeartbeats::new(&mode)),
            vec![],
            app_state.metrics.clone(),
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
            .await
            .unwrap()
            .unwrap();
        assert_eq!(node1.status, NodeStatus::Unhealthy);
        let counter = app_state
            .store_context
            .heartbeat_store
            .get_unhealthy_counter(&node1.address)
            .await
            .unwrap();
        assert_eq!(counter, 2);

        let node2 = app_state
            .store_context
            .node_store
            .get_node(&node2.address)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(node2.status, NodeStatus::Unhealthy);
        let counter = app_state
            .store_context
            .heartbeat_store
            .get_unhealthy_counter(&node2.address)
            .await
            .unwrap();
        assert_eq!(counter, 1);
    }

    #[tokio::test]
    async fn test_node_rediscovery_after_death() {
        let app_state = create_test_app_state_with_metrics().await;
        let contracts = setup_contract();

        let node = OrchestratorNode {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Unhealthy,
            ..Default::default()
        };

        if let Err(e) = app_state
            .store_context
            .node_store
            .add_node(node.clone())
            .await
        {
            error!("Error adding node: {}", e);
        }
        if let Err(e) = app_state
            .store_context
            .heartbeat_store
            .set_unhealthy_counter(&node.address, 2)
            .await
        {
            error!("Error setting unhealthy counter: {}", e);
        }

        let mode = ServerMode::Full;
        let updater = NodeStatusUpdater::new(
            app_state.store_context.clone(),
            5,
            None,
            contracts,
            0,
            false,
            Arc::new(LoopHeartbeats::new(&mode)),
            vec![],
            app_state.metrics.clone(),
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
            .await
            .unwrap()
            .unwrap();
        assert_eq!(node.status, NodeStatus::Dead);

        let heartbeat = HeartbeatRequest {
            address: node.address.to_string(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            task_details: None,
            ..Default::default()
        };
        if let Err(e) = app_state
            .store_context
            .heartbeat_store
            .beat(&heartbeat)
            .await
        {
            error!("Heartbeat Error: {}", e);
        }

        sleep(Duration::from_secs(5)).await;

        let node = app_state
            .store_context
            .node_store
            .get_node(&node.address)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(node.status, NodeStatus::Healthy);
    }

    #[tokio::test]
    async fn test_node_status_with_discovered_node() {
        let app_state = create_test_app_state_with_metrics().await;
        let contracts = setup_contract();

        let node = OrchestratorNode {
            address: Address::from_str("0x98dBe56Cac21e07693c558E4f27E7de4073e2aF3").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Discovered,
            ..Default::default()
        };

        if let Err(e) = app_state
            .store_context
            .node_store
            .add_node(node.clone())
            .await
        {
            error!("Error adding node: {}", e);
        }
        let mode = ServerMode::Full;
        let updater = NodeStatusUpdater::new(
            app_state.store_context.clone(),
            5,
            None,
            contracts,
            0,
            false,
            Arc::new(LoopHeartbeats::new(&mode)),
            vec![],
            app_state.metrics.clone(),
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
            .await
            .unwrap()
            .unwrap();

        let counter = app_state
            .store_context
            .heartbeat_store
            .get_unhealthy_counter(&node.address)
            .await
            .unwrap();
        assert_eq!(counter, 1);

        // Node has unhealthy counter
        assert_eq!(node.status, NodeStatus::Unhealthy);
    }

    #[tokio::test]
    async fn test_node_status_with_waiting_for_heartbeat() {
        let app_state = create_test_app_state_with_metrics().await;
        let contracts = setup_contract();

        let node = OrchestratorNode {
            address: Address::from_str("0x98dBe56Cac21e07693c558E4f27E7de4073e2aF3").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::WaitingForHeartbeat,
            ..Default::default()
        };
        let counter = app_state
            .store_context
            .heartbeat_store
            .get_unhealthy_counter(&node.address)
            .await
            .unwrap();
        assert_eq!(counter, 0);

        if let Err(e) = app_state
            .store_context
            .node_store
            .add_node(node.clone())
            .await
        {
            error!("Error adding node: {}", e);
        }
        let mode = ServerMode::Full;
        let updater = NodeStatusUpdater::new(
            app_state.store_context.clone(),
            5,
            None,
            contracts,
            0,
            false,
            Arc::new(LoopHeartbeats::new(&mode)),
            vec![],
            app_state.metrics.clone(),
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
            .await
            .unwrap();
        assert!(node.is_some());
        if let Some(node) = node {
            let counter = app_state
                .store_context
                .heartbeat_store
                .get_unhealthy_counter(&node.address)
                .await
                .unwrap();
            assert_eq!(counter, 1);
            assert_eq!(node.status, NodeStatus::WaitingForHeartbeat);
        }
    }
}
