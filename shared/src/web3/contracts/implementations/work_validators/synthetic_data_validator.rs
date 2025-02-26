use crate::web3::contracts::core::contract::Contract;
use crate::web3::wallet::Wallet;
use alloy::primitives::{Address, U256};
use anyhow::Error;

#[derive(Clone)]
pub struct SyntheticDataWorkValidator {
    pub instance: Contract,
}

impl SyntheticDataWorkValidator {
    pub fn new(address: Address, wallet: &Wallet, abi_file_path: &str) -> Self {
        let instance = Contract::new(address, wallet, abi_file_path);
        Self { instance }
    }

    pub async fn get_work_keys(&self, pool_id: U256) -> Result<Vec<String>, Error> {
        let result = self
            .instance
            .instance()
            .function("getWorkKeys", &[pool_id.into()])?
            .call()
            .await?;

        // Get the first (and probably only) value from the result
        let array_value = result
            .into_iter()
            .next()
            .ok_or_else(|| Error::msg("No result returned from getWorkKeys"))?;

        // Convert to array
        let array = array_value
            .as_array()
            .ok_or_else(|| Error::msg("Result is not an array"))?;

        // Map each value to a hex string
        let work_keys = array
            .into_iter()
            .map(|value| {
                let bytes = value
                    .as_fixed_bytes()
                    .ok_or_else(|| Error::msg("Value is not fixed bytes"))?;

                // Ensure we have exactly 32 bytes
                if bytes.0.len() != 32 {
                    return Err(Error::msg(format!(
                        "Expected 32 bytes, got {}",
                        bytes.0.len()
                    )));
                }

                // Convert bytes to string
                Ok(hex::encode(&bytes.0))
            })
            .collect::<Result<Vec<String>, Error>>()?;

        Ok(work_keys)
    }
}
