use crate::error::{PrimeProtocolError, Result};
use crate::worker::message_queue::MessageQueue;
use alloy::primitives::utils::format_ether;
use alloy::primitives::{Address, U256};
use prime_core::operations::compute_node::ComputeNodeOperations;
use prime_core::operations::provider::ProviderOperations;
use shared::web3::contracts::core::builder::{ContractBuilder, Contracts};
use shared::web3::contracts::structs::compute_pool::PoolStatus;
use shared::web3::wallet::{Wallet, WalletProvider};
use std::sync::Arc;
use url::Url;

pub struct WorkerClientCore {
    rpc_url: String,
    compute_pool_id: u64,
    private_key_provider: Option<String>,
    private_key_node: Option<String>,
    auto_accept_transactions: bool,
    funding_retry_count: u32,
    message_queue: Arc<MessageQueue>,
}

impl WorkerClientCore {
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
            message_queue: Arc::new(MessageQueue::new()),
        })
    }

    pub async fn start_async(&self) -> Result<()> {
        let (provider_wallet, node_wallet, contracts) =
            self.initialize_blockchain_components().await?;
        let pool_info = self.wait_for_active_pool(&contracts).await?;

        log::debug!("Pool info: {:?}", pool_info);
        log::debug!("Checking provider");
        self.ensure_provider_registered(&provider_wallet, &contracts)
            .await?;
        log::debug!("Checking compute node");
        self.ensure_compute_node_registered(&provider_wallet, &node_wallet, &contracts)
            .await?;

        log::debug!("blockchain components initialized");
        log::debug!("starting queues");

        // Start the message queue listener
        self.message_queue.start_listener().await.map_err(|e| {
            PrimeProtocolError::InvalidConfig(format!("Failed to start message listener: {}", e))
        })?;

        log::debug!("Message queue listener started");

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

        let provider_exists = self.check_provider_exists(&provider_ops).await?;
        let is_whitelisted = self.check_provider_whitelisted(&provider_ops).await?;

        if provider_exists && is_whitelisted {
            log::info!("Provider is registered and whitelisted");
        } else {
            self.register_provider_if_needed(&provider_ops, contracts)
                .await?;
        }

        self.ensure_adequate_stake(&provider_ops, provider_wallet, contracts)
            .await?;

        Ok(())
    }

    async fn check_provider_exists(&self, provider_ops: &ProviderOperations) -> Result<bool> {
        provider_ops.check_provider_exists().await.map_err(|e| {
            PrimeProtocolError::BlockchainError(format!(
                "Failed to check if provider exists: {}",
                e
            ))
        })
    }

    async fn check_provider_whitelisted(&self, provider_ops: &ProviderOperations) -> Result<bool> {
        provider_ops
            .check_provider_whitelisted()
            .await
            .map_err(|e| {
                PrimeProtocolError::BlockchainError(format!(
                    "Failed to check provider whitelist status: {}",
                    e
                ))
            })
    }

    async fn register_provider_if_needed(
        &self,
        provider_ops: &ProviderOperations,
        contracts: &Contracts<WalletProvider>,
    ) -> Result<()> {
        let stake_manager = contracts.stake_manager.as_ref().ok_or_else(|| {
            PrimeProtocolError::BlockchainError("Stake manager not initialized".to_string())
        })?;
        let compute_units = U256::from(1); // TODO: Make configurable

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
                PrimeProtocolError::BlockchainError(format!("Failed to register provider: {}", e))
            })?;

        log::info!("Provider registered successfully");
        Ok(())
    }

    async fn ensure_adequate_stake(
        &self,
        provider_ops: &ProviderOperations,
        provider_wallet: &Wallet,
        contracts: &Contracts<WalletProvider>,
    ) -> Result<()> {
        let stake_manager = contracts.stake_manager.as_ref().ok_or_else(|| {
            PrimeProtocolError::BlockchainError("Stake manager not initialized".to_string())
        })?;
        let provider_address = provider_wallet.wallet.default_signer().address();

        let provider_total_compute = self
            .get_provider_total_compute(contracts, provider_address)
            .await?;
        let provider_stake = self.get_provider_stake(contracts, provider_address).await;
        let compute_units = U256::from(1); // TODO: Make configurable

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
            self.increase_provider_stake(provider_ops, required_stake, provider_stake)
                .await?;
        }

        Ok(())
    }

    async fn get_provider_total_compute(
        &self,
        contracts: &Contracts<WalletProvider>,
        provider_address: Address,
    ) -> Result<U256> {
        contracts
            .compute_registry
            .get_provider_total_compute(provider_address)
            .await
            .map_err(|e| {
                PrimeProtocolError::BlockchainError(format!(
                    "Failed to get provider total compute: {}",
                    e
                ))
            })
    }

    async fn get_provider_stake(
        &self,
        contracts: &Contracts<WalletProvider>,
        provider_address: Address,
    ) -> U256 {
        let stake_manager = contracts.stake_manager.as_ref();
        match stake_manager {
            Some(manager) => manager
                .get_stake(provider_address)
                .await
                .unwrap_or_default(),
            None => U256::from(0),
        }
    }

    async fn increase_provider_stake(
        &self,
        provider_ops: &ProviderOperations,
        required_stake: U256,
        current_stake: U256,
    ) -> Result<()> {
        log::info!(
            "Provider stake is less than required stake. Required: {} tokens, Current: {} tokens",
            format_ether(required_stake),
            format_ether(current_stake)
        );

        provider_ops
            .increase_stake(required_stake - current_stake)
            .await
            .map_err(|e| {
                PrimeProtocolError::BlockchainError(format!("Failed to increase stake: {}", e))
            })?;

        log::info!("Successfully increased stake");
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

        let compute_node_exists = self.check_compute_node_exists(&compute_node_ops).await?;

        if compute_node_exists {
            log::info!("Compute node is already registered");
            return Ok(());
        }

        self.register_compute_node(&compute_node_ops).await?;
        Ok(())
    }

    async fn check_compute_node_exists(
        &self,
        compute_node_ops: &ComputeNodeOperations<'_>,
    ) -> Result<bool> {
        compute_node_ops
            .check_compute_node_exists()
            .await
            .map_err(|e| {
                PrimeProtocolError::BlockchainError(format!(
                    "Failed to check if compute node exists: {}",
                    e
                ))
            })
    }

    async fn register_compute_node(
        &self,
        compute_node_ops: &ComputeNodeOperations<'_>,
    ) -> Result<()> {
        let compute_units = U256::from(1); // TODO: Make configurable

        compute_node_ops
            .add_compute_node(compute_units)
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

    /// Get the shared message queue instance
    pub fn get_message_queue(&self) -> Arc<MessageQueue> {
        self.message_queue.clone()
    }

    /// Stop the message queue listener
    pub async fn stop_async(&self) -> Result<()> {
        self.message_queue.stop_listener().await.map_err(|e| {
            PrimeProtocolError::InvalidConfig(format!("Failed to stop message listener: {}", e))
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use test_log::test;

    #[test(tokio::test)]
    async fn test_start_async() {
        // standard anvil blockchain keys for local testing
        let node_key = "0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6";
        let provider_key = "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a";

        // todo: currently still have to make up the local blockchain incl. smart contract deployments
        let worker = WorkerClientCore::new(
            0,
            "http://localhost:8545".to_string(),
            Some(provider_key.to_string()),
            Some(node_key.to_string()),
            None,
            None,
        )
        .unwrap();
        worker.start_async().await.unwrap();
    }
}
