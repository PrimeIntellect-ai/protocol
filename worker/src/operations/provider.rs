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

    pub async fn register_provider(&self, stake: U256) -> Result<(), ProviderError> {
        Console::section("🏗️ Registering Provider");

        let address = self.wallet.wallet.default_signer().address();
        let balance: U256 = self
            .ai_token
            .balance_of(address)
            .await
            .map_err(|_| ProviderError::Other)?;

        let eth_balance = self
            .wallet
            .get_balance()
            .await
            .map_err(|_| ProviderError::Other)?;

        // Check if we are already provider
        let provider = self
            .compute_registry
            .get_provider(address)
            .await
            .map_err(|_| ProviderError::Other)?;

        let provider_exists = provider.provider_address != Address::default();
        Console::info("AI Token Balance", &format!("{} tokens", balance));
        Console::info("ETH Balance", &format!("{} ETH", eth_balance));
        Console::info("Provider registered:", &format!("{}", provider_exists));
        if !provider_exists {
            let spinner = Console::spinner("Approving AI Token for Stake transaction");
            let approve_tx = self
                .ai_token
                .approve(stake)
                .await
                .map_err(|_| ProviderError::Other)?;
            Console::info("Transaction approved", &format!("{:?}", approve_tx));
            spinner.finish_and_clear();

            let spinner = Console::spinner("Registering Provider");
            let register_tx = match self.prime_network.register_provider(stake).await {
                Ok(tx) => tx,
                Err(e) => {
                    println!("Failed to register provider: {:?}", e);
                    return Err(ProviderError::Other);
                }
            };
            Console::info(
                "Registration transaction completed: ",
                &format!("{:?}", register_tx),
            );
            spinner.finish_and_clear();
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
            Console::error("Provider could not be registered. Please ensure your token balance is high enough.");
            return Err(ProviderError::Other);
        }

        Console::success("Provider registered");
        if !provider.is_whitelisted {
            return Err(ProviderError::NotWhitelisted);
        }

        Ok(())
    }

    pub async fn increase_stake(&self, additional_stake: U256) -> Result<(), ProviderError> {
        Console::section("💰 Increasing Provider Stake");

        let address = self.wallet.wallet.default_signer().address();
        let balance: U256 = self
            .ai_token
            .balance_of(address)
            .await
            .map_err(|_| ProviderError::Other)?;

        Console::info("Current AI Token Balance", &format!("{} tokens", balance));
        Console::info("Additional stake amount", &format!("{}", additional_stake));

        if balance < additional_stake {
            Console::error("Insufficient token balance for stake increase");
            return Err(ProviderError::Other);
        }

        let spinner = Console::spinner("Approving AI Token for additional stake");
        let approve_tx = self
            .ai_token
            .approve(additional_stake)
            .await
            .map_err(|_| ProviderError::Other)?;
        Console::info("Transaction approved", &format!("{:?}", approve_tx));
        spinner.finish_and_clear();

        let spinner = Console::spinner("Increasing stake");
        let stake_tx = match self.prime_network.stake(additional_stake).await {
            Ok(tx) => tx,
            Err(e) => {
                println!("Failed to increase stake: {:?}", e);
                return Err(ProviderError::Other);
            }
        };
        Console::info(
            "Stake increase transaction completed: ",
            &format!("{:?}", stake_tx),
        );
        spinner.finish_and_clear();

        Console::success("Provider stake increased successfully");
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
