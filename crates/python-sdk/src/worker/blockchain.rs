use alloy::primitives::utils::format_ether;
use alloy::primitives::{Address, U256};
use anyhow::{Context, Result};
use operations::operations::compute_node::ComputeNodeOperations;
use operations::operations::provider::ProviderOperations;
use shared::web3::contracts::core::builder::{ContractBuilder, Contracts};
use shared::web3::contracts::structs::compute_pool::PoolStatus;
use shared::web3::wallet::{Wallet, WalletProvider};
use std::sync::Arc;
use url::Url;

use crate::constants::{BLOCKCHAIN_OPERATION_TIMEOUT, DEFAULT_COMPUTE_UNITS};

/// Configuration for blockchain operations
pub struct BlockchainConfig {
    pub rpc_url: String,
    pub compute_pool_id: u64,
    pub private_key_provider: String,
    pub private_key_node: String,
    pub auto_accept_transactions: bool,
    pub funding_retry_count: u32,
}

/// Handles all blockchain-related operations for the worker
pub struct BlockchainService {
    config: BlockchainConfig,
    provider_wallet: Option<Wallet>,
    node_wallet: Option<Wallet>,
    contracts: Option<Contracts<WalletProvider>>,
}

impl BlockchainService {
    pub fn new(config: BlockchainConfig) -> Result<Self> {
        // Validate RPC URL
        Url::parse(&config.rpc_url).context("Invalid RPC URL format")?;

        Ok(Self {
            config,
            provider_wallet: None,
            node_wallet: None,
            contracts: None,
        })
    }

    /// Get the node wallet (used for authentication)
    pub fn node_wallet(&self) -> Option<&Wallet> {
        self.node_wallet.as_ref()
    }

    /// Get the provider wallet
    pub fn provider_wallet(&self) -> Option<&Wallet> {
        self.provider_wallet.as_ref()
    }

    /// Initialize blockchain components and ensure the node is properly registered
    pub async fn initialize(&mut self) -> Result<()> {
        let (provider_wallet, node_wallet, contracts) = self.create_wallets_and_contracts().await?;

        // Store the wallets
        self.provider_wallet = Some(provider_wallet.clone());
        self.node_wallet = Some(node_wallet.clone());
        self.contracts = Some(contracts.clone());

        self.wait_for_active_pool(&contracts).await?;
        self.ensure_provider_registered(&provider_wallet, &contracts)
            .await?;
        self.ensure_compute_node_registered(&provider_wallet, &node_wallet, &contracts)
            .await?;

        Ok(())
    }

    async fn create_wallets_and_contracts(
        &self,
    ) -> Result<(Wallet, Wallet, Contracts<WalletProvider>)> {
        let rpc_url = Url::parse(&self.config.rpc_url)?;

        let provider_wallet = Wallet::new(&self.config.private_key_provider, rpc_url.clone())
            .map_err(|e| anyhow::anyhow!("Failed to create provider wallet: {}", e))?;

        let node_wallet = Wallet::new(&self.config.private_key_node, rpc_url.clone())
            .map_err(|e| anyhow::anyhow!("Failed to create node wallet: {}", e))?;

        let contracts = ContractBuilder::new(provider_wallet.provider())
            .with_compute_pool()
            .with_compute_registry()
            .with_ai_token()
            .with_prime_network()
            .with_stake_manager()
            .build()
            .context("Failed to build contracts")?;

        Ok((provider_wallet, node_wallet, contracts))
    }

    async fn wait_for_active_pool(
        &self,
        contracts: &Contracts<WalletProvider>,
    ) -> Result<shared::web3::contracts::structs::compute_pool::PoolInfo> {
        loop {
            match contracts
                .compute_pool
                .get_pool_info(U256::from(self.config.compute_pool_id))
                .await
            {
                Ok(pool) if pool.status == PoolStatus::ACTIVE => {
                    log::info!("Pool {} is active", self.config.compute_pool_id);
                    return Ok(pool);
                }
                Ok(pool) => {
                    log::info!(
                        "Pool {} is not active yet (status: {:?}), waiting...",
                        self.config.compute_pool_id,
                        pool.status
                    );
                    tokio::time::sleep(crate::constants::POOL_STATUS_CHECK_INTERVAL).await;
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("Failed to get pool info: {}", e));
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
            self.config.auto_accept_transactions,
        );

        let provider_exists = provider_ops
            .check_provider_exists()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to check if provider exists: {}", e))?;

        let is_whitelisted = provider_ops
            .check_provider_whitelisted()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to check provider whitelist status: {}", e))?;

        if !provider_exists || !is_whitelisted {
            self.register_provider(&provider_ops, contracts).await?;
        } else {
            log::info!("Provider is already registered and whitelisted");
        }

        self.ensure_adequate_stake(&provider_ops, provider_wallet, contracts)
            .await?;

        Ok(())
    }

    async fn register_provider(
        &self,
        provider_ops: &ProviderOperations,
        contracts: &Contracts<WalletProvider>,
    ) -> Result<()> {
        let stake_manager = contracts
            .stake_manager
            .as_ref()
            .context("Stake manager not initialized")?;

        let compute_units = U256::from(DEFAULT_COMPUTE_UNITS);
        let required_stake = stake_manager
            .calculate_stake(compute_units, U256::from(0))
            .await
            .map_err(|e| anyhow::anyhow!("Failed to calculate required stake: {}", e))?;

        log::info!(
            "Required stake for registration: {}",
            format_ether(required_stake)
        );

        tokio::time::timeout(
            BLOCKCHAIN_OPERATION_TIMEOUT,
            provider_ops.retry_register_provider(
                required_stake,
                self.config.funding_retry_count,
                None,
            ),
        )
        .await
        .context("Provider registration timed out")?
        .map_err(|e| anyhow::anyhow!("Failed to register provider: {}", e))?;

        log::info!("Provider registered successfully");
        Ok(())
    }

    async fn ensure_adequate_stake(
        &self,
        provider_ops: &ProviderOperations,
        provider_wallet: &Wallet,
        contracts: &Contracts<WalletProvider>,
    ) -> Result<()> {
        let stake_manager = contracts
            .stake_manager
            .as_ref()
            .context("Stake manager not initialized")?;

        let provider_address = provider_wallet.wallet.default_signer().address();

        let provider_total_compute = contracts
            .compute_registry
            .get_provider_total_compute(provider_address)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get provider total compute: {}", e))?;

        let provider_stake = stake_manager
            .get_stake(provider_address)
            .await
            .unwrap_or_default();

        let compute_units = U256::from(DEFAULT_COMPUTE_UNITS);
        let required_stake = stake_manager
            .calculate_stake(compute_units, provider_total_compute)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to calculate required stake: {}", e))?;

        if required_stake > provider_stake {
            log::info!(
                "Increasing provider stake. Required: {} tokens, Current: {} tokens",
                format_ether(required_stake),
                format_ether(provider_stake)
            );

            tokio::time::timeout(
                BLOCKCHAIN_OPERATION_TIMEOUT,
                provider_ops.increase_stake(required_stake - provider_stake),
            )
            .await
            .context("Stake increase timed out")?
            .map_err(|e| anyhow::anyhow!("Failed to increase stake: {}", e))?;

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

        let compute_node_exists = compute_node_ops
            .check_compute_node_exists()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to check if compute node exists: {}", e))?;

        if compute_node_exists {
            log::info!("Compute node is already registered");
            return Ok(());
        }

        let compute_units = U256::from(DEFAULT_COMPUTE_UNITS);

        tokio::time::timeout(
            BLOCKCHAIN_OPERATION_TIMEOUT,
            compute_node_ops.add_compute_node(compute_units),
        )
        .await
        .context("Compute node registration timed out")?
        .map_err(|e| anyhow::anyhow!("Failed to register compute node: {}", e))?;

        log::info!("Compute node registered successfully");
        Ok(())
    }

    /// Join compute pool using an invite
    pub async fn join_compute_pool_with_invite(&self, invite: &p2p::InviteRequest) -> Result<()> {
        use shared::web3::contracts::core::builder::ContractBuilder;
        use shared::web3::contracts::helpers::utils::retry_call;

        let invite_bytes = hex::decode(&invite.invite).context("Failed to decode invite")?;

        if invite_bytes.len() < 65 {
            anyhow::bail!("Invite data is too short");
        }

        let bytes_array: [u8; 65] = invite_bytes[..65]
            .try_into()
            .map_err(|_| anyhow::anyhow!("Failed to convert invite to array"))?;

        // Get wallets
        let provider_wallet = self
            .provider_wallet
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Provider wallet not initialized"))?;
        let node_wallet = self
            .node_wallet
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Node wallet not initialized"))?;

        // Create contracts
        let contracts = ContractBuilder::new(provider_wallet.provider())
            .with_compute_pool()
            .with_compute_registry()
            .with_ai_token()
            .with_prime_network()
            .with_stake_manager()
            .build()
            .context("Failed to build contracts")?;

        let pool_id = alloy::primitives::U256::from(invite.pool_id);
        let provider_address = provider_wallet.wallet.default_signer().address();
        let node_address = vec![node_wallet.wallet.default_signer().address()];
        let signatures = vec![alloy::primitives::FixedBytes::from(&bytes_array)];

        let call = contracts
            .compute_pool
            .build_join_compute_pool_call(
                pool_id,
                provider_address,
                node_address,
                vec![invite.nonce],
                vec![invite.expiration],
                signatures,
            )
            .map_err(|e| anyhow::anyhow!("Failed to build join compute pool call: {}", e))?;

        let result = retry_call(call, 3, provider_wallet.provider.clone(), None)
            .await
            .context("Failed to join compute pool")?;

        log::info!("Successfully joined compute pool with tx: {}", result);
        Ok(())
    }

    /// Get all validator addresses from the PrimeNetwork contract
    pub async fn get_validator_addresses(&self) -> Arc<std::collections::HashSet<Address>> {
        let contracts = match self.contracts.as_ref() {
            Some(contracts) => contracts,
            None => {
                log::error!("Contracts not initialized");
                return Arc::new(std::collections::HashSet::new());
            }
        };

        match contracts.prime_network.get_validator_role().await {
            Ok(validators) => {
                log::info!(
                    "Fetched {} validator addresses from chain",
                    validators.len()
                );
                let validator_set: std::collections::HashSet<Address> = validators
                    .into_iter()
                    .inspect(|&addr| {
                        log::debug!("Validator address: {:?}", addr);
                    })
                    .collect();
                log::info!("Validator addresses: {:?}", validator_set);
                Arc::new(validator_set)
            }
            Err(e) => {
                log::error!("Failed to get validator addresses: {}", e);
                Arc::new(std::collections::HashSet::new())
            }
        }
    }

    /// Get the pool owner address for the current compute pool
    pub async fn get_pool_owner_address(&self) -> Option<Address> {
        let contracts = match self.contracts.as_ref() {
            Some(contracts) => contracts,
            None => {
                log::error!("Contracts not initialized");
                return None;
            }
        };

        log::info!(
            "Fetching pool owner for pool ID: {}",
            self.config.compute_pool_id
        );

        match contracts
            .compute_pool
            .get_pool_info(U256::from(self.config.compute_pool_id))
            .await
        {
            Ok(pool_info) => {
                log::info!(
                    "Pool {} owner address: {:?}",
                    self.config.compute_pool_id,
                    pool_info.creator
                );
                log::info!(
                    "Pool {} owner address (checksummed): {}",
                    self.config.compute_pool_id,
                    pool_info.creator
                );
                Some(pool_info.creator)
            }
            Err(e) => {
                log::error!(
                    "Failed to get pool info for pool {}: {}",
                    self.config.compute_pool_id,
                    e
                );
                None
            }
        }
    }

    /// Get the compute manager address for the current compute pool
    pub async fn get_compute_manager_address(&self) -> Option<Address> {
        let contracts = match self.contracts.as_ref() {
            Some(contracts) => contracts,
            None => {
                log::error!("Contracts not initialized");
                return None;
            }
        };

        match contracts
            .compute_pool
            .get_pool_info(U256::from(self.config.compute_pool_id))
            .await
        {
            Ok(pool_info) => {
                log::info!(
                    "Pool {} compute manager address: {:?}",
                    self.config.compute_pool_id,
                    pool_info.compute_manager_key
                );
                log::info!(
                    "Pool {} compute manager address (checksummed): {}",
                    self.config.compute_pool_id,
                    pool_info.compute_manager_key
                );
                Some(pool_info.compute_manager_key)
            }
            Err(e) => {
                log::error!(
                    "Failed to get pool info for pool {}: {}",
                    self.config.compute_pool_id,
                    e
                );
                None
            }
        }
    }
}
