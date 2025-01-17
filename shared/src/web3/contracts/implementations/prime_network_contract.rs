use crate::web3::contracts::constants::addresses::PRIME_NETWORK_ADDRESS;
use crate::web3::contracts::core::contract::Contract;
use crate::web3::wallet::Wallet;
use alloy::dyn_abi::DynSolValue;
use alloy::primitives::{Address, FixedBytes, U256};

pub struct PrimeNetworkContract {
    pub instance: Contract,
}

impl PrimeNetworkContract {
    pub fn new(wallet: &Wallet, abi_file_path: &str) -> Self {
        let instance = Contract::new(PRIME_NETWORK_ADDRESS, wallet, abi_file_path);
        Self { instance }
    }

    pub async fn register_provider(
        &self,
        stake: U256,
    ) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
        let register_tx = self
            .instance
            .instance()
            .function("registerProvider", &[stake.into()])?
            .send()
            .await?
            .watch()
            .await?;

        Ok(register_tx)
    }

    pub async fn add_compute_node(
        &self,
        node_address: Address,
        compute_units: U256,
        signature: Vec<u8>,
    ) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
        let add_node_tx = self
            .instance
            .instance()
            .function(
                "addComputeNode",
                &[
                    node_address.into(),
                    "ipfs://nodekey/".to_string().into(),
                    compute_units.into(),
                    DynSolValue::Bytes(signature.to_vec()),
                ],
            )?
            .send()
            .await?
            .watch()
            .await?;

        Ok(add_node_tx)
    }

    pub async fn validate_node(
        &self,
        provider_address: Address,
        node_address: Address,
    ) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
        let validate_node_tx = self.instance.instance().function("validateNode", &[provider_address.into(), node_address.into()])?.send().await?.watch().await?;
        Ok(validate_node_tx)
    }
}
