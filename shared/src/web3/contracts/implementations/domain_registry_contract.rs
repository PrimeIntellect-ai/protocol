use crate::web3::contracts::constants::addresses::DOMAIN_REGISTRY_ADDRESS;
use crate::web3::contracts::core::contract::Contract;
use crate::web3::wallet::Wallet;
use alloy::dyn_abi::DynSolValue;
use alloy::primitives::{Address, U256};
use anyhow::Error;

pub struct Domain {
    pub domain_id: U256,
    pub name: String,
    pub validation_logic: Address,
    pub domain_parameters_uri: String,
}

#[derive(Clone)]
pub struct DomainRegistryContract {
    instance: Contract,
}

impl DomainRegistryContract {
    pub fn new(wallet: &Wallet, abi_file_path: &str) -> Self {
        let instance = Contract::new(DOMAIN_REGISTRY_ADDRESS, wallet, abi_file_path);
        Self { instance }
    }

    pub async fn get_domain(&self, domain_id: u32) -> Result<Domain, Error> {
        let result = self
            .instance
            .instance()
            .function("get", &[U256::from(domain_id).into()])?
            .call()
            .await?;

        let domain_info_tuple: &[DynSolValue] = result
            .first()
            .ok_or_else(|| Error::msg("Failed to get domain info tuple"))?
            .as_tuple()
            .ok_or_else(|| Error::msg("Failed to convert to tuple"))?;

        let domain_id: U256 = domain_info_tuple[0]
            .as_uint()
            .ok_or_else(|| Error::msg("Failed to get domain ID"))?
            .0;
        let name: String = domain_info_tuple[1]
            .as_str()
            .ok_or_else(|| Error::msg("Failed to get domain name"))?
            .to_string();
        let validation_logic: Address = domain_info_tuple[2]
            .as_address()
            .ok_or_else(|| Error::msg("Failed to get validation logic address"))?;
        let domain_parameters_uri: String = domain_info_tuple[3]
            .as_str()
            .ok_or_else(|| Error::msg("Failed to get domain parameters URI"))?
            .to_string();

        Ok(Domain {
            domain_id,
            name,
            validation_logic,
            domain_parameters_uri,
        })
    }
}
