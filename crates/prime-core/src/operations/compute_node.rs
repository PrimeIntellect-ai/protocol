use alloy::{primitives::utils::keccak256 as keccak, primitives::U256, signers::Signer};
use anyhow::Result;
use shared::web3::wallet::Wallet;
use shared::web3::{contracts::core::builder::Contracts, wallet::WalletProvider};

pub struct ComputeNodeOperations<'c> {
    provider_wallet: &'c Wallet,
    node_wallet: &'c Wallet,
    contracts: Contracts<WalletProvider>,
}

impl<'c> ComputeNodeOperations<'c> {
    pub fn new(
        provider_wallet: &'c Wallet,
        node_wallet: &'c Wallet,
        contracts: Contracts<WalletProvider>,
    ) -> Self {
        Self {
            provider_wallet,
            node_wallet,
            contracts,
        }
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
        log::info!("🔄 Adding compute node");

        if self.check_compute_node_exists().await? {
            return Ok(false);
        }

        log::info!("Adding compute node");
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
        log::info!("Add node tx: {add_node_tx:?}");
        Ok(true)
    }

    pub async fn remove_compute_node(&self) -> Result<bool, Box<dyn std::error::Error>> {
        log::info!("🔄 Removing compute node");

        if !self.check_compute_node_exists().await? {
            return Ok(false);
        }

        log::info!("Removing compute node");
        let provider_address = self.provider_wallet.wallet.default_signer().address();
        let node_address = self.node_wallet.wallet.default_signer().address();
        let remove_node_tx = self
            .contracts
            .prime_network
            .remove_compute_node(provider_address, node_address)
            .await?;
        log::info!("Remove node tx: {remove_node_tx:?}");
        Ok(true)
    }
}
