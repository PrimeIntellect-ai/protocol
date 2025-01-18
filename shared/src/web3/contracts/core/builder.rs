use crate::web3::{
    contracts::{
        core::error::ContractError, // Using custom error ContractError
        implementations::{
            ai_token_contract::AIToken, compute_pool_contract::ComputePool,
            compute_registry_contract::ComputeRegistryContract,
            prime_network_contract::PrimeNetworkContract,
        },
    },
    wallet::Wallet,
};
use std::option::Option;
use std::result::Result;

pub struct Contracts {
    pub compute_registry: ComputeRegistryContract,
    pub ai_token: AIToken,
    pub prime_network: PrimeNetworkContract,
    pub compute_pool: ComputePool,
}

pub struct ContractBuilder<'a> {
    wallet: &'a Wallet,
    compute_registry: Option<ComputeRegistryContract>,
    ai_token: Option<AIToken>,
    prime_network: Option<PrimeNetworkContract>,
    compute_pool: Option<ComputePool>,
}

impl<'a> ContractBuilder<'a> {
    pub fn new(wallet: &'a Wallet) -> Self {
        Self {
            wallet,
            compute_registry: None,
            ai_token: None,
            prime_network: None,
            compute_pool: None,
        }
    }

    pub fn with_compute_registry(mut self) -> Self {
        self.compute_registry = Some(ComputeRegistryContract::new(
            self.wallet,
            "ComputeRegistry.json",
        ));
        self
    }

    pub fn with_ai_token(mut self) -> Self {
        self.ai_token = Some(AIToken::new(self.wallet, "AIToken.json"));
        self
    }

    pub fn with_prime_network(mut self) -> Self {
        self.prime_network = Some(PrimeNetworkContract::new(self.wallet, "PrimeNetwork.json"));
        self
    }

    pub fn with_compute_pool(mut self) -> Self {
        self.compute_pool = Some(ComputePool::new(self.wallet, "ComputePool.json"));
        self
    }

    // TODO: This is not ideal yet - now you have to init all contracts all the time
    pub fn build(self) -> Result<Contracts, ContractError> {
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
        })
    }
}
