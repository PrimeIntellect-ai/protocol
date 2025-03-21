use crate::console::Console;
use alloy::{primitives::utils::keccak256 as keccak, primitives::U256, signers::Signer};
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::wallet::Wallet;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;

pub struct ComputeNodeOperations<'c> {
    provider_wallet: &'c Wallet,
    node_wallet: &'c Wallet,
    contracts: Arc<Contracts>,
}

impl<'c> ComputeNodeOperations<'c> {
    pub fn new(
        provider_wallet: &'c Wallet,
        node_wallet: &'c Wallet,
        contracts: Arc<Contracts>,
    ) -> Self {
        Self {
            provider_wallet,
            node_wallet,
            contracts,
        }
    }
    pub fn start_monitoring(&self, cancellation_token: CancellationToken) {
        let provider_address = self.provider_wallet.wallet.default_signer().address();
        let node_address = self.node_wallet.wallet.default_signer().address();
        let contracts = self.contracts.clone();
        let mut last_active = false;
        let mut last_validated = false;
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
                                if first_check {
                                    Console::info("Initial node status", &format!("Part of compute pool: {}, Validated: {}", active, validated));
                                    first_check = false;
                                    last_active = active;
                                    last_validated = validated;
                                } else if active != last_active {
                                    Console::info(
                                        "Node pool membership status changed on chain",
                                        &format!("Part of compute pool: {}", active),
                                    );
                                    last_active = active;
                                } else if validated != last_validated {
                                    Console::info(
                                        "Node validation status changed on chain",
                                        &format!("Validated: {}", validated),
                                    );
                                    last_validated = validated;
                                }
                            }
                            Err(e) => {
                                Console::error(&format!("Failed to get node status: {}", e));
                            }
                        }
                        sleep(Duration::from_secs(5)).await;
                    } => {}
                }
            }
        });
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
            Ok(_) => {
                Console::info("Compute node status", "Compute node already exists");
                Ok(true)
            }
            Err(_) => {
                Console::info(
                    "Compute node status",
                    "Compute node does not exist - creating",
                );
                Ok(false)
            }
        }
    }

    // Returns true if the compute node was added, false if it already exists
    pub async fn add_compute_node(
        &self,
        compute_units: U256,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        Console::section("🔄 Adding compute node");

        if self.check_compute_node_exists().await? {
            return Ok(false);
        }

        Console::progress("Adding compute node");
        Console::info(
            "Provider wallet",
            &format!(
                "{:?}",
                self.provider_wallet.wallet.default_signer().address()
            ),
        );
        Console::info(
            "Node wallet",
            &format!("{:?}", self.node_wallet.wallet.default_signer().address()),
        );

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
        Console::success(&format!("Add node tx: {:?}", add_node_tx));
        Ok(true)
    }
}
