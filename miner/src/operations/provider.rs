use crate::console::Console;
use alloy::primitives::{Address, U256};
use shared::web3::contracts::implementations::{
    ai_token_contract::AIToken, compute_registry_contract::ComputeRegistryContract,
    prime_network_contract::PrimeNetworkContract,
};
use shared::web3::wallet::Wallet;
use std::fmt;
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

    pub async fn register_provider(&self) -> Result<(), ProviderError> {
        Console::section("🏗️ Registering Provider");

        let address = self.wallet.wallet.default_signer().address();
        let balance: U256 = self
            .ai_token
            .balance_of(address)
            .await
            .map_err(|_| ProviderError::Other)?;

        // Check if we are already provider
        let provider = self
            .compute_registry
            .get_provider(address)
            .await
            .map_err(|_| ProviderError::Other)?;

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
            let approve_tx = self
                .ai_token
                .approve(stake)
                .await
                .map_err(|_| ProviderError::Other)?;
            Console::info("Transaction approved", &format!("{:?}", approve_tx));
            spinner.finish_and_clear();

            let spinner = Console::spinner("Registering Provider");
            let register_tx = self
                .prime_network
                .register_provider(stake)
                .await
                .map_err(|_| ProviderError::Other)?;
            spinner.finish_and_clear();
            Console::success(format!("Provider registered: {:?}", register_tx).as_str());
        }

        // Get provider details again  - cleanup later
        let spinner = Console::spinner("Getting provider details");
        let provider = self
            .compute_registry
            .get_provider(address)
            .await
            .map_err(|_| ProviderError::Other)?;
        spinner.finish_and_clear();
        spinner.abandon();
        Console::info("Is whitelisted", &format!("{:?}", provider.is_whitelisted));

        let provider_exists = provider.provider_address != Address::default();
        if !provider_exists {
            Console::error("Provider could not be registered.");
            return Err(ProviderError::Other);
        }

        if !provider.is_whitelisted {
            return Err(ProviderError::NotWhitelisted);
        }

        Ok(())
    }
}

#[derive(Debug)]
pub enum ProviderError {
    NotWhitelisted,
    Other,
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotWhitelisted => write!(f, "Provider is not whitelisted"),
            Self::Other => write!(f, "Provider could not be registered"),
        }
    }
}
