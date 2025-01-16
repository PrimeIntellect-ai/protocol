use crate::web3::contracts::implementations::{
    ai_token_contract::AIToken, compute_registry_contract::ComputeRegistryContract,
    prime_network_contract::PrimeNetworkContract,
};
use crate::web3::wallet::Wallet;
use alloy::{
    network::TransactionBuilder,
    primitives::utils::keccak256 as keccak,
    primitives::{Address, U256},
    providers::Provider,
    signers::Signer,
};

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

    pub async fn add_compute_node(&self) -> Result<(), Box<dyn std::error::Error>> {
        let compute_node = self
            .compute_registry
            .get_node(
                self.provider_wallet.wallet.default_signer().address(),
                self.node_wallet.wallet.default_signer().address(),
            )
            .await;

        match compute_node {
            Ok(()) => {
                println!("Compute node already exists");
                return Ok(());
            }
            Err(e) => {
                println!("Compute node does not exist - creating");
            }
        }

        println!("Adding compute node");
        println!(
            "Provider wallet: {:?}",
            self.provider_wallet.wallet.default_signer().address()
        );

        println!(
            "Node wallet: {:?}",
            self.node_wallet.wallet.default_signer().address()
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
        println!("Signature: {:?}", signature);

        // Create the signature bytes
        let compute_units: U256 = U256::from(10);
        let add_node_tx = self
            .prime_network
            .add_compute_node(node_address, compute_units, signature.to_vec())
            .await?;
        println!("Add node tx: {:?}", add_node_tx);
        Ok(())
    }
}
