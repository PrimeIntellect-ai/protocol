use crate::web3::contracts::core::contract::Contract;
use crate::web3::wallet::Wallet;
use alloy::primitives::{Address, FixedBytes, U256};
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

    pub async fn get_work_keys(&self, pool_id: U256) -> Result<(), Error> {
        let work_keys  = self.instance
            .instance()
            .function("getWorkKeys", &[pool_id.into()])?
            .call()
            .await;

        println!("work_keys: {:?}", work_keys);
        Ok(())
    }

}
