use crate::console::Console;
use alloy::primitives::{Address, U256};
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::wallet::Wallet;
use std::fmt;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;

pub struct ProviderOperations {
    wallet: Arc<Wallet>,
    contracts: Arc<Contracts>,
}

impl ProviderOperations {
    pub fn new(wallet: Arc<Wallet>, contracts: Arc<Contracts>) -> Self {
        Self { wallet, contracts }
    }
    pub fn start_monitoring(&self, cancellation_token: CancellationToken) {
        let provider_address = self.wallet.wallet.default_signer().address();
        let contracts = self.contracts.clone();

        // Only start monitoring if we have a stake manager
        let mut last_stake = U256::ZERO;
        let mut last_balance = U256::ZERO;
        let mut last_whitelist_status = false;
        let mut first_check = true;

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancellation_token.cancelled() => {
                        Console::info("Monitor", "Shutting down provider status monitor...");
                        break;
                    }
                    _ = async {
                        let stake_manager = match contracts.stake_manager.as_ref() {
                            Some(sm) => sm,
                            None => {
                                Console::error("Cannot start monitoring - stake manager not initialized");
                                return;
                            }
                        };

                        // Monitor stake
                        match stake_manager.get_stake(provider_address).await {
                            Ok(stake) => {
                                if first_check || stake != last_stake {
                                    Console::info("Provider stake", &format!("{} tokens", stake / U256::from(10u128.pow(18))));
                                    if !first_check {
                                        Console::info("Stake changed", &format!("From {} to {} tokens",
                                            last_stake / U256::from(10u128.pow(18)),
                                            stake / U256::from(10u128.pow(18))
                                        ));
                                    }
                                    last_stake = stake;
                                }
                                Some(stake)
                            },
                            Err(e) => {
                                Console::error(&format!("Failed to get stake: {}", e));
                                None
                            }
                        };

                        // Monitor AI token balance
                        match contracts.ai_token.balance_of(provider_address).await {
                            Ok(balance) => {
                                if first_check || balance != last_balance {
                                    Console::info("AI Token Balance", &format!("{} tokens", balance / U256::from(10u128.pow(18))));
                                    if !first_check {
                                        Console::info("Balance changed", &format!("From {} to {} tokens",
                                            last_balance / U256::from(10u128.pow(18)),
                                            balance / U256::from(10u128.pow(18))
                                        ));
                                    }
                                    last_balance = balance;
                                }
                                Some(balance)
                            },
                            Err(e) => {
                                Console::error(&format!("Failed to get AI token balance: {}", e));
                                None
                            }
                        };

                        // Monitor whitelist status
                        match contracts.compute_registry.get_provider(provider_address).await {
                            Ok(provider) => {
                                if first_check || provider.is_whitelisted != last_whitelist_status {
                                    Console::info("Whitelist status", &format!("{}", provider.is_whitelisted));
                                    if !first_check {
                                        Console::info("Whitelist status changed", &format!("From {} to {}",
                                            last_whitelist_status,
                                            provider.is_whitelisted
                                        ));
                                    }
                                    last_whitelist_status = provider.is_whitelisted;
                                }
                            },
                            Err(e) => {
                                Console::error(&format!("Failed to get provider whitelist status: {}", e));
                            }
                        };

                        first_check = false;
                        sleep(Duration::from_secs(5)).await;
                    } => {}
                }
            }
        });
    }

    pub async fn register_provider(&self, stake: U256) -> Result<(), ProviderError> {
        Console::section("🏗️ Registering Provider");

        let address = self.wallet.wallet.default_signer().address();
        let balance: U256 = self
            .contracts
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
            .contracts
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
            let spinner = Console::spinner("Approving AI Token for Stake transaction");
            let approve_tx = self
                .contracts
                .ai_token
                .approve(stake)
                .await
                .map_err(|_| ProviderError::Other)?;
            Console::info("Transaction approved", &format!("{:?}", approve_tx));
            spinner.finish_and_clear();

            let spinner = Console::spinner("Registering Provider");
            let register_tx = match self.contracts.prime_network.register_provider(stake).await {
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
            .contracts
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
            .contracts
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

        let spinner = Console::spinner("Approving AI Token for additional stake");
        let approve_tx = self
            .contracts
            .ai_token
            .approve(additional_stake)
            .await
            .map_err(|_| ProviderError::Other)?;
        Console::info("Transaction approved", &format!("{:?}", approve_tx));
        spinner.finish_and_clear();

        let spinner = Console::spinner("Increasing stake");
        let stake_tx = match self.contracts.prime_network.stake(additional_stake).await {
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
