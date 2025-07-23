use alloy::primitives::utils::format_ether;
use alloy::primitives::{Address, U256};
use log::error;
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::wallet::{Wallet, WalletProvider};
use std::io::Write;
use std::{fmt, io};
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;

pub struct ProviderOperations {
    wallet: Wallet,
    contracts: Contracts<WalletProvider>,
    auto_accept: bool,
}

impl ProviderOperations {
    pub fn new(wallet: Wallet, contracts: Contracts<WalletProvider>, auto_accept: bool) -> Self {
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

        print!("{message} [y/N]: ");
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
                        log::info!("Shutting down provider status monitor...");
                        break;
                    }
                    _ = async {
                        let Some(stake_manager) = contracts.stake_manager.as_ref() else {
                                log::error!("Cannot start monitoring - stake manager not initialized");
                                return;
                            };

                        // Monitor stake
                        match stake_manager.get_stake(provider_address).await {
                            Ok(stake) => {
                                if first_check || stake != last_stake {
                                    log::info!("🔄 Chain Sync - Provider stake: {}", format_ether(stake));
                                    if !first_check {
                                        if stake < last_stake {
                                            log::warn!("Stake decreased - possible slashing detected: From {} to {}",
                                                format_ether(last_stake),
                                                format_ether(stake)
                                            );
                                            if stake == U256::ZERO {
                                                log::warn!("Stake is 0 - you might have to restart the node to increase your stake (if you still have balance left)");
                                            }
                                        } else {
                                            log::info!("🔄 Chain Sync - Stake increased: From {} to {}",
                                                format_ether(last_stake),
                                                format_ether(stake)
                                            );
                                        }
                                    }
                                    last_stake = stake;
                                }
                                Some(stake)
                            },
                            Err(e) => {
                                error!("Failed to get stake: {e}");
                                None
                            }
                        };

                        // Monitor balance
                        match contracts.ai_token.balance_of(provider_address).await {
                            Ok(balance) => {
                                if first_check || balance != last_balance {
                                    log::info!("🔄 Chain Sync - Balance: {}", format_ether(balance));
                                    last_balance = balance;
                                }
                                Some(balance)
                            },
                            Err(e) => {
                                error!("Failed to get balance: {e}");
                                None
                            }
                        };

                        // Monitor whitelist status
                        match contracts.compute_registry.get_provider(provider_address).await {
                            Ok(provider) => {
                                if first_check || provider.is_whitelisted != last_whitelist_status {
                                    log::info!("🔄 Chain Sync - Whitelist status: {}", provider.is_whitelisted);
                                    if !first_check {
                                        log::info!("🔄 Chain Sync - Whitelist status changed: {} -> {}",
                                            last_whitelist_status,
                                            provider.is_whitelisted
                                        );
                                    }
                                    last_whitelist_status = provider.is_whitelisted;
                                }
                            },
                            Err(e) => {
                                error!("Failed to get provider whitelist status: {e}");
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
        cancellation_token: Option<CancellationToken>,
    ) -> Result<(), ProviderError> {
        log::info!("Registering Provider");
        let mut attempts = 0;
        while attempts < max_attempts || max_attempts == 0 {
            log::info!("Registering provider...");
            match self.register_provider(stake).await {
                Ok(_) => {
                    return Ok(());
                }
                Err(e) => match e {
                    ProviderError::NotWhitelisted | ProviderError::InsufficientBalance => {
                        log::info!("Retrying in 10 seconds...");
                        if let Some(ref token) = cancellation_token {
                            tokio::select! {
                                _ = tokio::time::sleep(tokio::time::Duration::from_secs(10)) => {}
                                _ = token.cancelled() => {
                                    return Err(e);
                                }
                            }
                        } else {
                            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                        }
                        attempts += 1;
                        continue;
                    }
                    _ => return Err(e),
                },
            }
        }
        log::error!("❌ Failed to register provider after {attempts} attempts");
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
            log::info!("Balance: {}", &format_ether(balance));
            log::info!(
                "ETH Balance: {}",
                &format!("{} ETH", format_ether(U256::from(eth_balance))),
            );
            if balance < stake {
                log::error!("Insufficient balance for stake: {}", format_ether(stake));
                return Err(ProviderError::InsufficientBalance);
            }
            if !self.prompt_user_confirmation(&format!(
                "Do you want to approve staking {}?",
                format_ether(stake)
            )) {
                log::info!("Operation cancelled by user: Staking approval declined");
                return Err(ProviderError::UserCancelled);
            }

            log::info!("Approving for Stake transaction");
            self.contracts
                .ai_token
                .approve(stake)
                .await
                .map_err(|_| ProviderError::Other)?;
            log::info!("Registering Provider");
            let Ok(register_tx) = self.contracts.prime_network.register_provider(stake).await
            else {
                return Err(ProviderError::Other);
            };
            log::info!("Registration tx: {}", &format!("{register_tx:?}"));
        }

        // Get provider details again  - cleanup later
        log::info!("Getting provider details");
        let _ = self
            .contracts
            .compute_registry
            .get_provider(address)
            .await
            .map_err(|_| ProviderError::Other)?;

        let provider_exists = self.check_provider_exists().await?;

        if !provider_exists {
            log::info!("Balance: {}", &format_ether(balance));
            log::info!(
                "ETH Balance: {}",
                &format!("{} ETH", format_ether(U256::from(eth_balance))),
            );
            if balance < stake {
                log::error!("Insufficient balance for stake: {}", format_ether(stake));
                return Err(ProviderError::InsufficientBalance);
            }
            if !self.prompt_user_confirmation(&format!(
                "Do you want to approve staking {}?",
                format_ether(stake)
            )) {
                log::info!("Operation cancelled by user: Staking approval declined");
                return Err(ProviderError::UserCancelled);
            }

            log::info!("Approving Stake transaction");
            self.contracts.ai_token.approve(stake).await.map_err(|e| {
                error!("Failed to approve stake: {e}");
                ProviderError::Other
            })?;
            log::info!("Registering Provider");
            let register_tx = match self.contracts.prime_network.register_provider(stake).await {
                Ok(tx) => tx,
                Err(e) => {
                    error!("Registration Error: {e}");
                    return Err(ProviderError::Other);
                }
            };
            log::info!("Registration tx: {register_tx:?}");
        }

        let provider = self
            .contracts
            .compute_registry
            .get_provider(address)
            .await
            .map_err(|_| ProviderError::Other)?;

        let provider_exists = provider.provider_address != Address::default();
        if !provider_exists {
            log::error!(
                "Provider could not be registered. Please ensure your balance is high enough."
            );
            return Err(ProviderError::Other);
        }

        log::info!("Provider registered");
        if !provider.is_whitelisted {
            log::error!("Provider is not whitelisted yet.");
            return Err(ProviderError::NotWhitelisted);
        }

        Ok(())
    }

    pub async fn increase_stake(&self, additional_stake: U256) -> Result<(), ProviderError> {
        log::info!("💰 Increasing Provider Stake");

        let address = self.wallet.wallet.default_signer().address();
        let balance: U256 = self
            .contracts
            .ai_token
            .balance_of(address)
            .await
            .map_err(|_| ProviderError::Other)?;

        log::info!("Current Balance: {}", &format_ether(balance));
        log::info!(
            "Additional stake amount: {}",
            &format_ether(additional_stake)
        );

        if balance < additional_stake {
            log::error!("Insufficient balance for stake increase");
            return Err(ProviderError::Other);
        }

        if !self.prompt_user_confirmation(&format!(
            "Do you want to approve staking {} additional funds?",
            format_ether(additional_stake)
        )) {
            log::info!("Operation cancelled by user: Staking approval declined");
            return Err(ProviderError::UserCancelled);
        }

        log::info!("Approving additional stake");
        let approve_tx = self
            .contracts
            .ai_token
            .approve(additional_stake)
            .await
            .map_err(|_| ProviderError::Other)?;
        log::info!("Transaction approved: {}", &format!("{approve_tx:?}"));

        log::info!("Increasing stake");
        let stake_tx = match self.contracts.prime_network.stake(additional_stake).await {
            Ok(tx) => tx,
            Err(e) => {
                println!("Failed to increase stake: {e:?}");
                return Err(ProviderError::Other);
            }
        };
        log::info!(
            "Stake increase transaction completed: {}",
            &format!("{stake_tx:?}")
        );

        Ok(())
    }

    pub async fn reclaim_stake(&self, amount: U256) -> Result<(), ProviderError> {
        let reclaim_tx = match self.contracts.prime_network.reclaim_stake(amount).await {
            Ok(tx) => tx,
            Err(e) => {
                println!("Failed to reclaim stake: {e:?}");
                return Err(ProviderError::Other);
            }
        };
        log::info!(
            "Stake reclaim transaction completed: {}",
            &format!("{reclaim_tx:?}")
        );
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
            Self::InsufficientBalance => write!(f, "Insufficient balance for stake"),
        }
    }
}
