use crate::console::Console;
use alloy::primitives::{Address, U256};
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::wallet::Wallet;
use std::io::Write;
use std::sync::Arc;
use std::{fmt, io};
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;

pub struct ProviderOperations {
    wallet: Arc<Wallet>,
    contracts: Arc<Contracts>,
    auto_accept: bool,
}

impl ProviderOperations {
    pub fn new(wallet: Arc<Wallet>, contracts: Arc<Contracts>, auto_accept: bool) -> Self {
        Self {
            wallet,
            contracts,
            auto_accept,
        }
    }

    fn prompt_user_confirmation(&self, message: &str) -> bool {
        if self.auto_accept {
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
                                    Console::info("🔄 Chain Sync - Provider stake", &format!("{} tokens", stake / U256::from(10u128.pow(18))));
                                    if !first_check {
                                        if stake < last_stake {
                                            Console::warning(&format!("Stake decreased - possible slashing detected: From {} to {} tokens",
                                                last_stake / U256::from(10u128.pow(18)),
                                                stake / U256::from(10u128.pow(18))
                                            ));
                                            if stake == U256::ZERO {
                                                Console::warning("Stake is 0 - you might have to restart the node to increase your stake (if you still have tokens left)");
                                            }
                                        } else {
                                            Console::info("🔄 Chain Sync - Stake changed", &format!("From {} to {} tokens",
                                                last_stake / U256::from(10u128.pow(18)),
                                                stake / U256::from(10u128.pow(18))
                                            ));
                                        }
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
                                    Console::info("🔄 Chain Sync - AI Token Balance", &format!("{} tokens", balance / U256::from(10u128.pow(18))));
                                    if !first_check {
                                        Console::info("🔄 Chain Sync - Balance changed", &format!("From {} to {} tokens",
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
                                    Console::info("🔄 Chain Sync - Whitelist status", &format!("{}", provider.is_whitelisted));
                                    if !first_check {
                                        Console::info("🔄 Chain Sync - Whitelist status changed", &format!("From {} to {}",
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

    pub async fn check_provider_exists(&self) -> Result<bool, ProviderError> {
        let address = self.wallet.wallet.default_signer().address();

        let provider = self
            .contracts
            .compute_registry
            .get_provider(address)
            .await
            .map_err(|_| ProviderError::Other)?;

        Ok(provider.provider_address != Address::default())
    }

    pub async fn check_provider_whitelisted(&self) -> Result<bool, ProviderError> {
        let address = self.wallet.wallet.default_signer().address();

        let provider = self
            .contracts
            .compute_registry
            .get_provider(address)
            .await
            .map_err(|_| ProviderError::Other)?;

        Ok(provider.is_whitelisted)
    }

    pub async fn retry_register_provider(
        &self,
        stake: U256,
        max_attempts: u32,
        cancellation_token: CancellationToken,
    ) -> Result<(), ProviderError> {
        Console::title("Registering Provider");
        let mut attempts = 0;
        while attempts < max_attempts || max_attempts == 0 {
            Console::progress("Registering provider...");
            match self.register_provider(stake).await {
                Ok(_) => {
                    return Ok(());
                }
                Err(e) => match e {
                    ProviderError::NotWhitelisted | ProviderError::InsufficientBalance => {
                        Console::info("Info", "Retrying in 10 seconds...");
                        tokio::select! {
                            _ = tokio::time::sleep(tokio::time::Duration::from_secs(10)) => {}
                            _ = cancellation_token.cancelled() => {
                                return Err(e);
                            }
                        }
                        attempts += 1;
                        continue;
                    }
                    _ => return Err(e),
                },
            }
        }
        Console::error(&format!(
            "❌ Failed to register provider after {} attempts",
            attempts
        ));
        Err(ProviderError::Other)
    }

    pub async fn register_provider(&self, stake: U256) -> Result<(), ProviderError> {
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

        let provider_exists = self.check_provider_exists().await?;

        if !provider_exists {
            Console::info(
                "AI Token Balance",
                &format!("{} tokens", balance / U256::from(10u128.pow(18))),
            );
            Console::info(
                "ETH Balance",
                &format!("{} ETH", eth_balance / U256::from(10u128.pow(18))),
            );
            if balance < stake {
                Console::error(&format!(
                    "Insufficient AI Token balance for stake: {} tokens",
                    stake / U256::from(10u128.pow(18))
                ));
                return Err(ProviderError::InsufficientBalance);
            }
            if !self.prompt_user_confirmation(&format!(
                "Do you want to approve staking {} tokens?",
                stake / U256::from(10u128.pow(18))
            )) {
                Console::info("Operation cancelled by user", "Staking approval declined");
                return Err(ProviderError::UserCancelled);
            }

            Console::progress("Approving AI Token for Stake transaction");
            self.contracts
                .ai_token
                .approve(stake)
                .await
                .map_err(|_| ProviderError::Other)?;
            Console::progress("Registering Provider");
            let register_tx = match self.contracts.prime_network.register_provider(stake).await {
                Ok(tx) => tx,
                Err(_) => {
                    return Err(ProviderError::Other);
                }
            };
            Console::info("Registration tx", &format!("{:?}", register_tx));
        }

        // Get provider details again  - cleanup later
        Console::progress("Getting provider details");
        let provider = self
            .contracts
            .compute_registry
            .get_provider(address)
            .await
            .map_err(|_| ProviderError::Other)?;

        let provider_exists = provider.provider_address != Address::default();
        if !provider_exists {
            Console::error("Provider could not be registered. Please ensure your token balance is high enough.");
            return Err(ProviderError::Other);
        }

        Console::success("Provider registered");
        if !provider.is_whitelisted {
            Console::error("Provider is not whitelisted yet.");
            return Err(ProviderError::NotWhitelisted);
        }

        Ok(())
    }

    pub async fn increase_stake(&self, additional_stake: U256) -> Result<(), ProviderError> {
        Console::title("💰 Increasing Provider Stake");

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

        if !self.prompt_user_confirmation(&format!(
            "Do you want to approve staking {} additional tokens?",
            additional_stake / U256::from(10u128.pow(18))
        )) {
            Console::info("Operation cancelled by user", "Staking approval declined");
            return Err(ProviderError::UserCancelled);
        }

        Console::progress("Approving AI Token for additional stake");
        let approve_tx = self
            .contracts
            .ai_token
            .approve(additional_stake)
            .await
            .map_err(|_| ProviderError::Other)?;
        Console::info("Transaction approved", &format!("{:?}", approve_tx));

        Console::progress("Increasing stake");
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

        Console::success("Provider stake increased successfully");
        Ok(())
    }
}

#[derive(Debug)]
pub enum ProviderError {
    NotWhitelisted,
    UserCancelled,
    Other,
    InsufficientBalance,
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotWhitelisted => write!(f, "Provider is not whitelisted"),
            Self::UserCancelled => write!(f, "Operation cancelled by user"),
            Self::Other => write!(f, "Provider could not be registered"),
            Self::InsufficientBalance => write!(f, "Insufficient AI Token balance for stake"),
        }
    }
}
