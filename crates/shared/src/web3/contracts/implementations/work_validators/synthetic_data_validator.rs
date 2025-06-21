use crate::web3::contracts::core::contract::Contract;
use alloy::{
    dyn_abi::{DynSolValue, Word},
    primitives::{Address, U256},
};
use anyhow::Error;
use serde::Deserialize;
use serde::Serialize;

#[derive(Clone)]
pub struct SyntheticDataWorkValidator<P: alloy_provider::Provider> {
    pub instance: Contract<P>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone, Copy)]
pub struct WorkInfo {
    pub provider: Address,
    pub node_id: Address,
    pub timestamp: u64,
    pub work_units: U256,
}

impl<P: alloy_provider::Provider> SyntheticDataWorkValidator<P> {
    pub fn new(address: Address, provider: P, abi_file_path: &str) -> Self {
        let instance = Contract::new(address, provider, abi_file_path);
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

        let work_keys = array
            .iter()
            .map(|value| {
                let bytes = value
                    .as_fixed_bytes()
                    .ok_or_else(|| Error::msg("Value is not fixed bytes"))?;

                if bytes.0.len() != 32 {
                    return Err(Error::msg(format!(
                        "Expected 32 bytes, got {}",
                        bytes.0.len()
                    )));
                }

                Ok(hex::encode(bytes.0))
            })
            .collect::<Result<Vec<String>, Error>>()?;

        Ok(work_keys)
    }

    pub async fn get_work_info(&self, pool_id: U256, work_key: &str) -> Result<WorkInfo, Error> {
        let work_key_bytes = hex::decode(work_key)?;
        if work_key_bytes.len() != 32 {
            return Err(Error::msg("Work key must be 32 bytes"));
        }

        let fixed_bytes = DynSolValue::FixedBytes(Word::from_slice(&work_key_bytes), 32);

        let result = self
            .instance
            .instance()
            .function("getWorkInfo", &[pool_id.into(), fixed_bytes])?
            .call()
            .await?;

        let tuple = result
            .into_iter()
            .next()
            .ok_or_else(|| Error::msg("No result returned from getWorkInfo"))?;

        let tuple_array = tuple
            .as_tuple()
            .ok_or_else(|| Error::msg("Result is not a tuple"))?;
        if tuple_array.len() != 4 {
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

        let work_units = tuple_array[3]
            .as_uint()
            .ok_or_else(|| Error::msg("Work units is not a uint"))?
            .0;

        Ok(WorkInfo {
            provider,
            node_id,
            timestamp,
            work_units,
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

                if bytes.0.len() != 32 {
                    return Err(Error::msg(format!(
                        "Expected 32 bytes, got {}",
                        bytes.0.len()
                    )));
                }

                Ok(hex::encode(bytes.0))
            })
            .collect::<Result<Vec<String>, Error>>()?;

        Ok(work_keys)
    }
}
