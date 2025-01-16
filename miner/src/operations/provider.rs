use crate::console::Console;
use crate::web3::contracts::implementations::{
    ai_token_contract::AIToken, compute_registry_contract::ComputeRegistryContract,
    prime_network_contract::PrimeNetworkContract,
};
use crate::web3::wallet::Wallet;
use alloy::{
    network::TransactionBuilder,
    primitives::{Address, U256},
    signers::Signer,
}; // Import Console for logging

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
        Console::section("🏗️ Registering Provider");

        let address = self.wallet.wallet.default_signer().address();
        let balance: U256 = self.ai_token.balance_of(address).await?;

        // Check if we are already provider
        let provider = self.compute_registry.get_provider(address).await?;

        let provider_exists = provider.provider_address != Address::default();

        Console::info(
            "Provider address",
            &format!("{:?}", provider.provider_address),
        );
        Console::info("AI Token Balance", &format!("{} tokens", balance));
        Console::info("Is whitelisted", &format!("{:?}", provider.is_whitelisted));
        Console::info("Provider registered", &format!("{}", provider_exists));

        if !provider_exists {
            let stake: U256 = U256::from(100);
            let spinner = Console::spinner("Approving AI Token");
            let approve_tx = self.ai_token.approve(stake).await?;
            Console::info("Transaction approved", &format!("{:?}", approve_tx));
            spinner.finish();

            let spinner = Console::spinner("Registering Provider");
            let register_tx = self.prime_network.register_provider(stake).await?;
            spinner.finish();
            Console::success(format!("Provider registered: {:?}", register_tx).as_str());
        }

        // Get provider details again  - cleanup later
        let spinner = Console::spinner("Getting provider details");
        let provider = self.compute_registry.get_provider(address).await?;
        spinner.finish();
        Console::info("Is whitelisted", &format!("{:?}", provider.is_whitelisted));

        let provider_exists = provider.provider_address != Address::default();
        if !provider_exists {
            Console::error("Provider could not be registered.");
            std::process::exit(1);
        }

        if !provider.is_whitelisted {
            Console::error("Provider is not whitelisted. Cannot proceed.");
            std::process::exit(1);
        }

        Ok(())
    }
}
