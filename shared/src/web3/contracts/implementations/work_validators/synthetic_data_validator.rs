use crate::web3::contracts::core::contract::Contract;
use crate::web3::wallet::Wallet;
use alloy::{
    dyn_abi::{DynSolValue, Word},
    primitives::{Address, U256},
};
use anyhow::Error;
use log::debug;
use serde::Deserialize;

#[derive(Clone)]
pub struct SyntheticDataWorkValidator {
    pub instance: Contract,
}

#[derive(Debug, Deserialize)]
pub struct WorkInfo {
    pub provider: Address,
    pub node_id: Address,
    pub timestamp: u64,
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

        let array_value = result
            .into_iter()
            .next()
            .ok_or_else(|| Error::msg("No result returned from getWorkKeys"))?;

        let array = array_value
            .as_array()
            .ok_or_else(|| Error::msg("Result is not an array"))?;

        // Map each value to a hex string
        let work_keys = array
            .iter()
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
                Ok(hex::encode(bytes.0))
            })
            .collect::<Result<Vec<String>, Error>>()?;

        Ok(work_keys)
    }

    pub async fn get_work_info(&self, pool_id: U256, work_key: &str) -> Result<WorkInfo, Error> {
        // Convert work_key from hex string to bytes32
        debug!("Processing work key: {}", work_key);
        let work_key_bytes = hex::decode(work_key)?;
        if work_key_bytes.len() != 32 {
            return Err(Error::msg("Work key must be 32 bytes"));
        }
        debug!("Decoded work key bytes: {:?}", work_key_bytes);

        let fixed_bytes = DynSolValue::FixedBytes(Word::from_slice(&work_key_bytes), 32);

        let result = self
            .instance
            .instance()
            .function("getWorkInfo", &[pool_id.into(), fixed_bytes])?
            .call()
            .await?;
        debug!("Got work info result: {:?}", result);

        let tuple = result
            .into_iter()
            .next()
            .ok_or_else(|| Error::msg("No result returned from getWorkInfo"))?;

        let tuple_array = tuple
            .as_tuple()
            .ok_or_else(|| Error::msg("Result is not a tuple"))?;

        if tuple_array.len() != 3 {
            return Err(Error::msg("Invalid tuple length"));
        }

        let provider = tuple_array[0]
            .as_address()
            .ok_or_else(|| Error::msg("Provider is not an address"))?;

        let node_id = tuple_array[1]
            .as_address()
            .ok_or_else(|| Error::msg("Node ID is not an address"))?;

        let timestamp = u64::try_from(
            tuple_array[2]
                .as_uint()
                .ok_or_else(|| Error::msg("Timestamp is not a uint"))?
                .0,
        )
        .map_err(|_| Error::msg("Timestamp conversion failed"))?;

        Ok(WorkInfo {
            provider,
            node_id,
            timestamp,
        })
    }

    pub async fn get_work_since(
        &self,
        pool_id: U256,
        timestamp: U256,
    ) -> Result<Vec<String>, Error> {
        let result = self
            .instance
            .instance()
            .function("getWorkSince", &[pool_id.into(), timestamp.into()])?
            .call()
            .await?;

        let array_value = result
            .into_iter()
            .next()
            .ok_or_else(|| Error::msg("No result returned from getWorkSince"))?;

        let array = array_value
            .as_array()
            .ok_or_else(|| Error::msg("Result is not an array"))?;

        let work_keys = array
            .iter()
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
                Ok(hex::encode(bytes.0))
            })
            .collect::<Result<Vec<String>, Error>>()?;

        Ok(work_keys)
    }
}
