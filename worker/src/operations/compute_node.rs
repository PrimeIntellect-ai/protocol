use crate::console::Console;
use alloy::{primitives::utils::keccak256 as keccak, primitives::U256, signers::Signer};
use shared::web3::contracts::implementations::{
    compute_registry_contract::ComputeRegistryContract,
    prime_network_contract::PrimeNetworkContract,
};
use shared::web3::wallet::Wallet;

pub struct ComputeNodeOperations<'c> {
    provider_wallet: &'c Wallet,
    node_wallet: &'c Wallet,
    compute_registry: &'c ComputeRegistryContract,
    prime_network: &'c PrimeNetworkContract,
}

impl<'c> ComputeNodeOperations<'c> {
    pub fn new(
        provider_wallet: &'c Wallet,
        node_wallet: &'c Wallet,
        compute_registry: &'c ComputeRegistryContract,
        prime_network: &'c PrimeNetworkContract,
    ) -> Self {
        Self {
            provider_wallet,
            node_wallet,
            compute_registry,
            prime_network,
        }
    }

    // Returns true if the compute node was added, false if it already exists
    pub async fn add_compute_node(
        &self,
        compute_units: U256,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        Console::section("🔄 Adding compute node");
        let compute_node = self
            .compute_registry
            .get_node(
                self.provider_wallet.wallet.default_signer().address(),
                self.node_wallet.wallet.default_signer().address(),
            )
            .await;

        match compute_node {
            Ok(_) => {
                Console::info("Compute node status", "Compute node already exists");
                return Ok(false);
            }
            Err(_) => {
                Console::info(
                    "Compute node status",
                    "Compute node does not exist - creating",
                );
            }
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
            .prime_network
            .add_compute_node(node_address, compute_units, signature.to_vec())
            .await?;
        Console::success(&format!("Add node tx: {:?}", add_node_tx));
        Ok(true)
    }
}
