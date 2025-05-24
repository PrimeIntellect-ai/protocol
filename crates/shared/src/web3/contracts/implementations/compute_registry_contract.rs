use super::{
    super::constants::addresses::COMPUTE_REGISTRY_ADDRESS, super::core::contract::Contract,
    super::structs::compute_provider::ComputeProvider,
};
use crate::web3::contracts::helpers::utils::get_selector;
use crate::web3::wallet::Wallet;
use alloy::dyn_abi::DynSolValue;
use alloy::primitives::{Address, U256};
use anyhow::Result;

#[derive(Clone)]
pub struct ComputeRegistryContract {
    instance: Contract,
}

impl ComputeRegistryContract {
    pub fn new(wallet: &Wallet, abi_path: &str) -> Self {
        let instance = Contract::new(COMPUTE_REGISTRY_ADDRESS, wallet, abi_path);
        Self { instance }
    }

    pub async fn get_provider(
        &self,
        address: Address,
    ) -> Result<ComputeProvider, Box<dyn std::error::Error + Send + Sync>> {
        let provider_response = self
            .instance
            .instance()
            .function("getProvider", &[address.into()])?
            .call()
            .await?;

        let provider_tuple: &[DynSolValue] = provider_response.first().unwrap().as_tuple().unwrap();
        let provider_address: Address = provider_tuple[0].as_address().unwrap();
        let is_whitelisted: bool = provider_tuple[1].as_bool().unwrap();

        let provider = ComputeProvider {
            provider_address,
            is_whitelisted,
            active_nodes: 0,
            nodes: vec![],
        };
        Ok(provider)
    }

    pub async fn get_provider_total_compute(
        &self,
        address: Address,
    ) -> Result<U256, Box<dyn std::error::Error>> {
        let provider_response = self
            .instance
            .instance()
            .function("getProviderTotalCompute", &[address.into()])?
            .call()
            .await?;

        Ok(U256::from(
            provider_response.first().unwrap().as_uint().unwrap().0,
        ))
    }
    pub async fn get_node(
        &self,
        provider_address: Address,
        node_address: Address,
    ) -> anyhow::Result<(bool, bool)> {
        let get_node_selector = get_selector("getNode(address,address)");

        let node_response = self
            .instance
            .instance()
            .function_from_selector(
                &get_node_selector,
                &[provider_address.into(), node_address.into()],
            )?
            .call()
            .await?;

        if let Some(_node_data) = node_response.first() {
            let node_tuple = _node_data.as_tuple().unwrap();

            // Check that provider and subkey match
            let node_provider = node_tuple[0].as_address().unwrap();
            let node_subkey = node_tuple[1].as_address().unwrap();

            if node_provider != provider_address || node_subkey != node_address {
                return Err(anyhow::anyhow!("Node does not match provider or subkey"));
            }

            let active = node_tuple[5].as_bool().unwrap();
            let validated = node_tuple[6].as_bool().unwrap();
            Ok((active, validated))
        } else {
            Err(anyhow::anyhow!("Node is not registered"))
        }
    }
}
