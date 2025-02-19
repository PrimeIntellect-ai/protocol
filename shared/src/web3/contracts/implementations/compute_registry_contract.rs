use super::{
    super::constants::addresses::COMPUTE_REGISTRY_ADDRESS, super::core::contract::Contract,
    super::structs::compute_provider::ComputeProvider,
};
use crate::web3::contracts::helpers::utils::get_selector;
use crate::web3::wallet::Wallet;
use alloy::dyn_abi::DynSolValue;
use alloy::primitives::Address;

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
    ) -> Result<ComputeProvider, Box<dyn std::error::Error>> {
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

    pub async fn get_node(
        &self,
        #[allow(unused_variables)] provider_address: Address,
        node_address: Address,
    ) -> Result<(bool, bool), Box<dyn std::error::Error>> {
        let get_node_selector = get_selector("getNode(address,address)");

        let node_response = self
            .instance
            .instance()
            .function_from_selector(
                &get_node_selector,
                &[provider_address.into(), node_address.into()],
            )?
            .call()
            .await;

        // TODO: This should be cleaned up - either we add additional check if this is actually the no-exist error or work on the contract response
        match node_response {
            Ok(response) => {
                if let Some(_node_data) = response.first() {
                    let node_tuple = _node_data.as_tuple().unwrap();
                    let active = node_tuple[5].as_bool().unwrap();
                    let validated = node_tuple[6].as_bool().unwrap();
                    Ok((active, validated))
                } else {
                    println!("Node is not registered. Proceeding to add the node.");
                    Err("Node is not registered".into())
                }
            }
            Err(_) => Err("Node is not registered".into()),
        }
    }
}
