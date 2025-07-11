use crate::state::system_state::SystemState;
use alloy::primitives::U256;
use anyhow::Result;
use shared::web3::wallet::Wallet;
use shared::web3::{contracts::core::builder::Contracts, wallet::WalletProvider};
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;

pub(crate) struct NodeMonitor {
    provider_wallet: Wallet,
    node_wallet: Wallet,
    contracts: Contracts<WalletProvider>,
    system_state: Arc<SystemState>,
}

impl NodeMonitor {
    pub(crate) fn new(
        provider_wallet: Wallet,
        node_wallet: Wallet,
        contracts: Contracts<WalletProvider>,
        system_state: Arc<SystemState>,
    ) -> Self {
        Self {
            provider_wallet,
            node_wallet,
            contracts,
            system_state,
        }
    }

    pub(crate) fn start_monitoring(
        &self,
        cancellation_token: CancellationToken,
        pool_id: u32,
    ) -> Result<()> {
        let provider_address = self.provider_wallet.wallet.default_signer().address();
        let node_address = self.node_wallet.wallet.default_signer().address();
        let contracts = self.contracts.clone();
        let system_state = self.system_state.clone();
        let mut last_active = false;
        let mut last_validated = false;
        let mut last_claimable = None;
        let mut last_locked = None;
        let mut first_check = true;

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancellation_token.cancelled() => {
                        log::info!("Shutting down node status monitor...");
                        break;
                    }
                    _ = async {
                        match contracts.compute_registry.get_node(provider_address, node_address).await {
                            Ok((active, validated)) => {
                                if first_check || active != last_active {
                                    if !first_check {
                                        log::info!("🔄 Chain Sync - Pool membership changed: From {last_active} to {active}");
                                    } else {
                                        log::info!("🔄 Chain Sync - Node pool membership: {active}");
                                    }
                                    last_active = active;
                                }
                                let is_running = system_state.is_running().await;
                                if !active && is_running {
                                    log::warn!("Node is not longer in pool, shutting down heartbeat...");
                                    if let Err(e) = system_state.set_running(false, None).await {
                                        log::error!("Failed to set running to false: {e:?}");
                                    }
                                }

                                if first_check || validated != last_validated {
                                    if !first_check {
                                        log::info!("🔄 Chain Sync - Validation changed: From {last_validated} to {validated}");
                                    } else {
                                        log::info!("🔄 Chain Sync - Node validation: {validated}");
                                    }
                                    last_validated = validated;
                                }

                                // Check rewards for the current compute pool
                                    match contracts.compute_pool.calculate_node_rewards(
                                        U256::from(pool_id),
                                        node_address,
                                    ).await {
                                        Ok((claimable, locked)) => {
                                            if last_claimable.is_none() || last_locked.is_none() || claimable != last_claimable.unwrap() || locked != last_locked.unwrap() {
                                                last_claimable = Some(claimable);
                                                last_locked = Some(locked);
                                                let claimable_formatted = claimable.to_string().parse::<f64>().unwrap_or(0.0) / 10f64.powf(18.0);
                                                let locked_formatted = locked.to_string().parse::<f64>().unwrap_or(0.0) / 10f64.powf(18.0);
                                                log::info!("Rewards: {claimable_formatted} claimable, {locked_formatted} locked");
                                            }
                                        }
                                        Err(e) => {
                                            log::debug!("Failed to check rewards for pool {pool_id}: {e}");
                                        }

                                }

                                first_check = false;
                            }
                            Err(e) => {
                                log::error!("Failed to get node status: {e}");
                            }
                        }
                        sleep(Duration::from_secs(5)).await;
                    } => {}
                }
            }
        });
        Ok(())
    }
}
