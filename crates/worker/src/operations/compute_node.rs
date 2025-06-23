use crate::{console::Console, state::system_state::SystemState};
use alloy::{primitives::utils::keccak256 as keccak, primitives::U256, signers::Signer};
use anyhow::Result;
use shared::web3::wallet::Wallet;
use shared::web3::{contracts::core::builder::Contracts, wallet::WalletProvider};
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;

pub struct ComputeNodeOperations<'c> {
    provider_wallet: &'c Wallet,
    node_wallet: &'c Wallet,
    contracts: Contracts<WalletProvider>,
    system_state: Arc<SystemState>,
}

impl<'c> ComputeNodeOperations<'c> {
    pub fn new(
        provider_wallet: &'c Wallet,
        node_wallet: &'c Wallet,
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

    pub fn start_monitoring(
        &self,
        cancellation_token: CancellationToken,
        pool_id: String,
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
                        Console::info("Monitor", "Shutting down node status monitor...");
                        break;
                    }
                    _ = async {
                        match contracts.compute_registry.get_node(provider_address, node_address).await {
                            Ok((active, validated)) => {
                                if first_check || active != last_active {
                                    if !first_check {
                                        Console::info("🔄 Chain Sync - Pool membership changed", &format!("From {last_active} to {active}"
                                        ));
                                    } else {
                                        Console::info("🔄 Chain Sync - Node pool membership", &format!("{active}"));
                                    }
                                    last_active = active;
                                }
                                let is_running = system_state.is_running().await;
                                if !active && is_running {
                                    Console::warning("Node is not longer in pool, shutting down heartbeat...");
                                    if let Err(e) = system_state.set_running(false, None).await {
                                        log::error!("Failed to set running to false: {e:?}");
                                    }
                                }

                                if first_check || validated != last_validated {
                                    if !first_check {
                                        Console::info("🔄 Chain Sync - Validation changed", &format!("From {last_validated} to {validated}"
                                        ));
                                    } else {
                                        Console::info("🔄 Chain Sync - Node validation", &format!("{validated}"));
                                    }
                                    last_validated = validated;
                                }

                                // Check rewards for the current compute pool
                                if let Ok(pool_id_u32) = pool_id.parse::<u32>() {
                                    match contracts.compute_pool.calculate_node_rewards(
                                        U256::from(pool_id_u32),
                                        node_address,
                                    ).await {
                                        Ok((claimable, locked)) => {
                                            if last_claimable.is_none() || last_locked.is_none() || claimable != last_claimable.unwrap() || locked != last_locked.unwrap() {
                                                last_claimable = Some(claimable);
                                                last_locked = Some(locked);
                                                let claimable_formatted = claimable.to_string().parse::<f64>().unwrap_or(0.0) / 10f64.powf(18.0);
                                                let locked_formatted = locked.to_string().parse::<f64>().unwrap_or(0.0) / 10f64.powf(18.0);
                                                Console::info("Rewards", &format!("{claimable_formatted} claimable, {locked_formatted} locked"));
                                            }
                                        }
                                        Err(e) => {
                                            log::debug!("Failed to check rewards for pool {pool_id_u32}: {e}");
                                        }
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

    pub async fn check_compute_node_exists(&self) -> Result<bool, Box<dyn std::error::Error>> {
        let compute_node = self
            .contracts
            .compute_registry
            .get_node(
                self.provider_wallet.wallet.default_signer().address(),
                self.node_wallet.wallet.default_signer().address(),
            )
            .await;

        match compute_node {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    // Returns true if the compute node was added, false if it already exists
    pub async fn add_compute_node(
        &self,
        compute_units: U256,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        Console::title("🔄 Adding compute node");

        if self.check_compute_node_exists().await? {
            return Ok(false);
        }

        Console::progress("Adding compute node");
        let provider_address = self.provider_wallet.wallet.default_signer().address();
        let node_address = self.node_wallet.wallet.default_signer().address();
        let digest = keccak([provider_address.as_slice(), node_address.as_slice()].concat());

        let signature = self
            .node_wallet
            .signer
            .sign_message(digest.as_slice())
            .await?
            .as_bytes();

        // Create the signature bytes
        let add_node_tx = self
            .contracts
            .prime_network
            .add_compute_node(node_address, compute_units, signature.to_vec())
            .await?;
        Console::success(&format!("Add node tx: {add_node_tx:?}"));
        Ok(true)
    }

    pub async fn remove_compute_node(&self) -> Result<bool, Box<dyn std::error::Error>> {
        Console::title("🔄 Removing compute node");

        if !self.check_compute_node_exists().await? {
            return Ok(false);
        }

        Console::progress("Removing compute node");
        let provider_address = self.provider_wallet.wallet.default_signer().address();
        let node_address = self.node_wallet.wallet.default_signer().address();
        let remove_node_tx = self
            .contracts
            .prime_network
            .remove_compute_node(provider_address, node_address)
            .await?;
        Console::success(&format!("Remove node tx: {remove_node_tx:?}"));
        Ok(true)
    }
}
