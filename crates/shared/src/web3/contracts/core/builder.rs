use alloy::primitives::Address;

use crate::web3::contracts::{
    core::error::ContractError, // Using custom error ContractError
    implementations::{
        ai_token_contract::AIToken, compute_pool_contract::ComputePool,
        compute_registry_contract::ComputeRegistryContract,
        domain_registry_contract::DomainRegistryContract,
        prime_network_contract::PrimeNetworkContract, stake_manager::StakeManagerContract,
        work_validators::synthetic_data_validator::SyntheticDataWorkValidator,
    },
};
use std::option::Option;
use std::result::Result;

#[derive(Clone)]
pub struct Contracts<P: alloy_provider::Provider> {
    pub compute_registry: ComputeRegistryContract<P>,
    pub ai_token: AIToken<P>,
    pub prime_network: PrimeNetworkContract<P>,
    pub compute_pool: ComputePool<P>,
    pub stake_manager: Option<StakeManagerContract<P>>,
    pub synthetic_data_validator: Option<SyntheticDataWorkValidator<P>>,
    pub domain_registry: Option<DomainRegistryContract<P>>,
}

pub struct ContractBuilder<P: alloy_provider::Provider + Clone> {
    provider: P,
    compute_registry: Option<ComputeRegistryContract<P>>,
    ai_token: Option<AIToken<P>>,
    prime_network: Option<PrimeNetworkContract<P>>,
    compute_pool: Option<ComputePool<P>>,
    stake_manager: Option<StakeManagerContract<P>>,
    synthetic_data_validator: Option<SyntheticDataWorkValidator<P>>,
    domain_registry: Option<DomainRegistryContract<P>>,
}

impl<P: alloy_provider::Provider + Clone> ContractBuilder<P> {
    pub fn new(provider: P) -> Self {
        Self {
            provider,
            compute_registry: None,
            ai_token: None,
            prime_network: None,
            compute_pool: None,
            stake_manager: None,
            synthetic_data_validator: None,
            domain_registry: None,
        }
    }

    pub fn with_compute_registry(mut self) -> Self {
        self.compute_registry = Some(ComputeRegistryContract::new(
            self.provider.clone(),
            "compute_registry.json",
        ));
        self
    }

    pub fn with_ai_token(mut self) -> Self {
        self.ai_token = Some(AIToken::new(self.provider.clone(), "ai_token.json"));
        self
    }

    pub fn with_prime_network(mut self) -> Self {
        self.prime_network = Some(PrimeNetworkContract::new(
            self.provider.clone(),
            "prime_network.json",
        ));
        self
    }

    pub fn with_compute_pool(mut self) -> Self {
        self.compute_pool = Some(ComputePool::new(self.provider.clone(), "compute_pool.json"));
        self
    }

    pub fn with_synthetic_data_validator(mut self, address: Option<Address>) -> Self {
        self.synthetic_data_validator = Some(SyntheticDataWorkValidator::new(
            address.unwrap_or(Address::ZERO),
            self.provider.clone(),
            "synthetic_data_work_validator.json",
        ));
        self
    }

    pub fn with_domain_registry(mut self) -> Self {
        self.domain_registry = Some(DomainRegistryContract::new(
            self.provider.clone(),
            "domain_registry.json",
        ));
        self
    }

    pub fn with_stake_manager(mut self) -> Self {
        self.stake_manager = Some(StakeManagerContract::new(
            self.provider.clone(),
            "stake_manager.json",
        ));
        self
    }

    // TODO: This is not ideal yet - now you have to init all contracts all the time
    pub fn build(self) -> Result<Contracts<P>, ContractError> {
        // Using custom error ContractError
        Ok(Contracts {
            compute_pool: match self.compute_pool {
                Some(pool) => pool,
                None => return Err(ContractError::Other("ComputePool not initialized".into())),
            },
            compute_registry: match self.compute_registry {
                Some(registry) => registry,
                None => {
                    return Err(ContractError::Other(
                        "ComputeRegistry not initialized".into(),
                    ))
                } // Custom error handling
            },
            ai_token: match self.ai_token {
                Some(token) => token,
                None => return Err(ContractError::Other("AIToken not initialized".into())), // Custom error handling
            },
            prime_network: match self.prime_network {
                Some(network) => network,
                None => return Err(ContractError::Other("PrimeNetwork not initialized".into())), // Custom error handling
            },
            synthetic_data_validator: self.synthetic_data_validator,
            domain_registry: self.domain_registry,
            stake_manager: self.stake_manager,
        })
    }

    pub fn build_partial(&self) -> Result<Contracts<P>, ContractError> {
        Ok(Contracts {
            compute_registry: self
                .compute_registry
                .as_ref()
                .ok_or_else(|| ContractError::Other("ComputeRegistry not initialized".into()))?
                .clone(),

            ai_token: self
                .ai_token
                .as_ref()
                .ok_or_else(|| ContractError::Other("AIToken not initialized".into()))?
                .clone(),

            prime_network: self
                .prime_network
                .as_ref()
                .ok_or_else(|| ContractError::Other("PrimeNetwork not initialized".into()))?
                .clone(),

            compute_pool: self
                .compute_pool
                .as_ref()
                .ok_or_else(|| ContractError::Other("ComputePool not initialized".into()))?
                .clone(),

            domain_registry: Some(
                self.domain_registry
                    .as_ref()
                    .ok_or_else(|| ContractError::Other("DomainRegistry not initialized".into()))?
                    .clone(),
            ),

            stake_manager: Some(
                self.stake_manager
                    .as_ref()
                    .ok_or_else(|| ContractError::Other("StakeManager not initialized".into()))?
                    .clone(),
            ),
            synthetic_data_validator: None,
        })
    }
}
