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

pub struct ProviderOperations<'c> {
    wallet: &'c Wallet,
    compute_registry: &'c ComputeRegistryContract,
    ai_token: &'c AIToken,
    prime_network: &'c PrimeNetworkContract,
}

impl<'c> ProviderOperations<'c> {
    pub fn new(
        wallet: &'c Wallet,
        compute_registry: &'c ComputeRegistryContract,
        ai_token: &'c AIToken,
        prime_network: &'c PrimeNetworkContract,
    ) -> Self {
        Self {
            wallet,
            compute_registry,
            ai_token,
            prime_network,
        }
    }

    pub async fn register_provider(&self) -> Result<(), Box<dyn std::error::Error>> {
        // TODO[MOVE]: Move provider registration logic to a separate module for better organization
        let address = self.wallet.wallet.default_signer().address();
        println!("Address: {:?}", address);
        let balance: U256 = self.ai_token.balance_of(address).await?;

        // Check if we are already provider
        let provider = self.compute_registry.get_provider(address).await?;
        println!("Provider address: {:?}", provider.provider_address);
        println!("Is whitelisted: {:?}", provider.is_whitelisted);

        let provider_exists = provider.provider_address != Address::default();

        println!("Provider registered: {}", provider_exists);

        println!("Balance: {} tokens", balance);
        if !provider_exists {
            let stake: U256 = U256::from(100);
            let approve_tx = self.ai_token.approve(stake).await?;
            println!("Transaction approved: {:?}", approve_tx);

            let register_tx = self.prime_network.register_provider(stake).await?;
            println!("Provider registered: {:?}", register_tx);
        }

        // Get provider details again  - cleanup later
        let provider = self.compute_registry.get_provider(address).await?;
        println!("Provider address: {:?}", provider.provider_address);
        println!("Is whitelisted: {:?}", provider.is_whitelisted);

        let provider_exists = provider.provider_address != Address::default();
        if !provider_exists {
            eprintln!("Provider could not be registered.");
            std::process::exit(1);
        }

        if !provider.is_whitelisted {
            eprintln!("Provider is not whitelisted. Cannot proceed.");
            std::process::exit(1);
        }

        Ok(())
    }
}
