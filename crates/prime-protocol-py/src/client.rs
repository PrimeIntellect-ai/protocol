use crate::error::{PrimeProtocolError, Result};
use alloy::primitives::utils::format_ether;
use alloy::primitives::U256;
use prime_core::operations::compute_node::ComputeNodeOperations;
use prime_core::operations::provider::ProviderOperations;
use shared::web3::contracts::core::builder::{ContractBuilder, Contracts};
use shared::web3::contracts::structs::compute_pool::PoolStatus;
use shared::web3::wallet::{Wallet, WalletProvider};
use url::Url;

pub struct PrimeProtocolClientCore {
    rpc_url: String,
    compute_pool_id: u64,
    private_key_provider: Option<String>,
    private_key_node: Option<String>,
    auto_accept_transactions: bool,
    funding_retry_count: u32,
}

impl PrimeProtocolClientCore {
    pub fn new(
        compute_pool_id: u64,
        rpc_url: String,
        private_key_provider: Option<String>,
        private_key_node: Option<String>,
        auto_accept_transactions: Option<bool>,
        funding_retry_count: Option<u32>,
    ) -> Result<Self> {
        if rpc_url.is_empty() {
            return Err(PrimeProtocolError::InvalidConfig(
                "RPC URL cannot be empty".to_string(),
            ));
        }

        Url::parse(&rpc_url)
            .map_err(|_| PrimeProtocolError::InvalidConfig("Invalid RPC URL format".to_string()))?;

        Ok(Self {
            rpc_url,
            compute_pool_id,
            private_key_provider,
            private_key_node,
            auto_accept_transactions: auto_accept_transactions.unwrap_or(true),
            funding_retry_count: funding_retry_count.unwrap_or(10),
        })
    }

    pub async fn start_async(&self) -> Result<()> {
        let (provider_wallet, node_wallet, contracts) =
            self.initialize_blockchain_components().await?;
        let pool_info = self.wait_for_active_pool(&contracts).await?;

        log::info!("Pool info: {:?}", pool_info);

        self.ensure_provider_registered(&provider_wallet, &contracts)
            .await?;
        self.ensure_compute_node_registered(&provider_wallet, &node_wallet, &contracts)
            .await?;

        // TODO: Optional - run hardware check?
        // TODO: p2p reachable?

        Ok(())
    }

    async fn initialize_blockchain_components(
        &self,
    ) -> Result<(Wallet, Wallet, Contracts<WalletProvider>)> {
        let private_key_provider = self.get_private_key_provider()?;
        let private_key_node = self.get_private_key_node()?;
        let rpc_url = Url::parse(&self.rpc_url).unwrap();

        let provider_wallet = Wallet::new(&private_key_provider, rpc_url.clone()).map_err(|e| {
            PrimeProtocolError::BlockchainError(format!("Failed to create provider wallet: {}", e))
        })?;

        let node_wallet = Wallet::new(&private_key_node, rpc_url.clone()).map_err(|e| {
            PrimeProtocolError::BlockchainError(format!("Failed to create node wallet: {}", e))
        })?;

        let contracts = ContractBuilder::new(provider_wallet.provider())
            .with_compute_pool()
            .with_compute_registry()
            .with_ai_token()
            .with_prime_network()
            .with_stake_manager()
            .build()
            .map_err(|e| PrimeProtocolError::BlockchainError(e.to_string()))?;

        Ok((provider_wallet, node_wallet, contracts))
    }

    async fn wait_for_active_pool(
        &self,
        contracts: &Contracts<WalletProvider>,
    ) -> Result<shared::web3::contracts::structs::compute_pool::PoolInfo> {
        loop {
            match contracts
                .compute_pool
                .get_pool_info(U256::from(self.compute_pool_id))
                .await
            {
                Ok(pool) if pool.status == PoolStatus::ACTIVE => return Ok(pool),
                Ok(_) => {
                    log::info!("Pool not active yet, waiting...");
                    tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;
                }
                Err(e) => {
                    return Err(PrimeProtocolError::BlockchainError(format!(
                        "Failed to get pool info: {}",
                        e
                    )));
                }
            }
        }
    }
    async fn ensure_provider_registered(
        &self,
        provider_wallet: &Wallet,
        contracts: &Contracts<WalletProvider>,
    ) -> Result<()> {
        let provider_ops = ProviderOperations::new(
            provider_wallet.clone(),
            contracts.clone(),
            self.auto_accept_transactions,
        );

        // Check if provider exists
        let provider_exists = provider_ops.check_provider_exists().await.map_err(|e| {
            PrimeProtocolError::BlockchainError(format!(
                "Failed to check if provider exists: {}",
                e
            ))
        })?;

        let Some(stake_manager) = contracts.stake_manager.as_ref() else {
            return Err(PrimeProtocolError::BlockchainError(
                "Stake manager not initialized".to_string(),
            ));
        };

        // Check if provider is whitelisted
        let is_whitelisted = provider_ops
            .check_provider_whitelisted()
            .await
            .map_err(|e| {
                PrimeProtocolError::BlockchainError(format!(
                    "Failed to check provider whitelist status: {}",
                    e
                ))
            })?;

        // todo: revisit this
        if provider_exists && is_whitelisted {
            log::info!("Provider is registered and whitelisted");
        } else {
            // For now, we'll use a default compute_units value - this should be configurable
            let compute_units = U256::from(1);

            let required_stake = stake_manager
                .calculate_stake(compute_units, U256::from(0))
                .await
                .map_err(|e| {
                    PrimeProtocolError::BlockchainError(format!(
                        "Failed to calculate required stake: {}",
                        e
                    ))
                })?;

            log::info!("Required stake: {}", format_ether(required_stake));

            provider_ops
                .retry_register_provider(required_stake, self.funding_retry_count, None)
                .await
                .map_err(|e| {
                    PrimeProtocolError::BlockchainError(format!(
                        "Failed to register provider: {}",
                        e
                    ))
                })?;

            log::info!("Provider registered successfully");
        }

        // Get provider's current total compute and stake
        let provider_total_compute = contracts
            .compute_registry
            .get_provider_total_compute(provider_wallet.wallet.default_signer().address())
            .await
            .map_err(|e| {
                PrimeProtocolError::BlockchainError(format!(
                    "Failed to get provider total compute: {}",
                    e
                ))
            })?;

        let provider_stake = stake_manager
            .get_stake(provider_wallet.wallet.default_signer().address())
            .await
            .unwrap_or_default();

        // For now, we'll use a default compute_units value - this should be configurable
        let compute_units = U256::from(1);

        let required_stake = stake_manager
            .calculate_stake(compute_units, provider_total_compute)
            .await
            .map_err(|e| {
                PrimeProtocolError::BlockchainError(format!(
                    "Failed to calculate required stake: {}",
                    e
                ))
            })?;

        if required_stake > provider_stake {
            log::info!(
                "Provider stake is less than required stake. Required: {} tokens, Current: {} tokens",
                format_ether(required_stake),
                format_ether(provider_stake)
            );

            provider_ops
                .increase_stake(required_stake - provider_stake)
                .await
                .map_err(|e| {
                    PrimeProtocolError::BlockchainError(format!("Failed to increase stake: {}", e))
                })?;

            log::info!("Successfully increased stake");
        }

        Ok(())
    }

    async fn ensure_compute_node_registered(
        &self,
        provider_wallet: &Wallet,
        node_wallet: &Wallet,
        contracts: &Contracts<WalletProvider>,
    ) -> Result<()> {
        let compute_node_ops =
            ComputeNodeOperations::new(provider_wallet, node_wallet, contracts.clone());

        // Check if compute node exists
        let compute_node_exists =
            compute_node_ops
                .check_compute_node_exists()
                .await
                .map_err(|e| {
                    PrimeProtocolError::BlockchainError(format!(
                        "Failed to check if compute node exists: {}",
                        e
                    ))
                })?;

        if compute_node_exists {
            log::info!("Compute node is already registered");
            return Ok(());
        }

        // If compute node doesn't exist, register it
        // For now, we'll use default compute specs - this should be configurable
        compute_node_ops
            .add_compute_node(U256::from(1))
            .await
            .map_err(|e| {
                PrimeProtocolError::BlockchainError(format!(
                    "Failed to register compute node: {}",
                    e
                ))
            })?;

        log::info!("Compute node registered successfully");
        Ok(())
    }

    fn get_private_key_provider(&self) -> Result<String> {
        match &self.private_key_provider {
            Some(key) => Ok(key.clone()),
            None => std::env::var("PRIVATE_KEY_PROVIDER").map_err(|_| {
                PrimeProtocolError::InvalidConfig("PRIVATE_KEY_PROVIDER must be set".to_string())
            }),
        }
    }

    fn get_private_key_node(&self) -> Result<String> {
        match &self.private_key_node {
            Some(key) => Ok(key.clone()),
            None => std::env::var("PRIVATE_KEY_NODE").map_err(|_| {
                PrimeProtocolError::InvalidConfig("PRIVATE_KEY_NODE must be set".to_string())
            }),
        }
    }
}
