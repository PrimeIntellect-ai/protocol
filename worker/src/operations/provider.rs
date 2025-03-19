use crate::console::Console;
use alloy::primitives::{Address, U256};
use shared::web3::contracts::implementations::{
    ai_token_contract::AIToken, compute_registry_contract::ComputeRegistryContract,
    prime_network_contract::PrimeNetworkContract,
};
use shared::web3::wallet::Wallet;
use std::fmt;
use std::io::{self, Write};

pub struct ProviderOperations<'c> {
    wallet: &'c Wallet,
    compute_registry: &'c ComputeRegistryContract,
    ai_token: &'c AIToken,
    prime_network: &'c PrimeNetworkContract,
    auto_accept: &'c bool,
}

impl<'c> ProviderOperations<'c> {
    pub fn new(
        wallet: &'c Wallet,
        compute_registry: &'c ComputeRegistryContract,
        ai_token: &'c AIToken,
        prime_network: &'c PrimeNetworkContract,
        auto_accept: &'c bool,
    ) -> Self {
        Self {
            wallet,
            compute_registry,
            ai_token,
            prime_network,
            auto_accept,
        }
    }

    fn prompt_user_confirmation(&self, message: &str) -> bool {
        if *self.auto_accept {
            return true;
        }

        print!("{} [y/N]: ", message);
        io::stdout().flush().unwrap();

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_ok() {
            input.trim().to_lowercase() == "y"
        } else {
            false
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
        Console::info(
            "AI Token Balance",
            &format!("{} tokens", balance / U256::from(10u128.pow(18))),
        );
        Console::info(
            "ETH Balance",
            &format!("{} ETH", eth_balance / U256::from(10u128.pow(18))),
        );
        Console::info("Provider registered:", &format!("{}", provider_exists));

        if !provider_exists {
            let stake_amount = stake / U256::from(10u128.pow(18));
            if !self.prompt_user_confirmation(&format!(
                "Do you want to approve staking {} tokens?",
                stake_amount
            )) {
                Console::info("Operation cancelled by user", "Staking approval declined");
                return Err(ProviderError::UserCancelled);
            }

            let approve_tx = self
                .ai_token
                .approve(stake)
                .await
                .map_err(|_| ProviderError::Other)?;
            Console::info("Transaction approved", &format!("{:?}", approve_tx));

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

        Console::info(
            "Current AI Token Balance",
            &format!("{} tokens", balance / U256::from(10u128.pow(18))),
        );
        Console::info(
            "Additional stake amount",
            &format!("{}", additional_stake / U256::from(10u128.pow(18))),
        );

        if balance < additional_stake {
            Console::error("Insufficient token balance for stake increase");
            return Err(ProviderError::Other);
        }

        if !self.prompt_user_confirmation(&format!(
            "Do you want to approve staking {} additional tokens?",
            additional_stake / U256::from(10u128.pow(18))
        )) {
            Console::info("Operation cancelled by user", "Staking approval declined");
            return Err(ProviderError::UserCancelled);
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
    UserCancelled,
    Other,
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotWhitelisted => write!(f, "Provider is not whitelisted"),
            Self::UserCancelled => write!(f, "Operation cancelled by user"),
            Self::Other => write!(f, "Provider could not be registered"),
        }
    }
}
