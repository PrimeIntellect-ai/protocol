use crate::web3::contracts::constants::addresses::STAKE_MANAGER_ADDRESS;
use crate::web3::contracts::core::contract::Contract;
use crate::web3::wallet::Wallet;
use alloy::primitives::{Address, U256};

#[derive(Clone)]
pub struct StakeManagerContract {
    instance: Contract,
}

impl StakeManagerContract {
    pub fn new(wallet: &Wallet, abi_file_path: &str) -> Self {
        let instance = Contract::new(STAKE_MANAGER_ADDRESS, wallet, abi_file_path);
        Self { instance }
    }

    pub async fn get_stake_minimum(&self) -> Result<U256, Box<dyn std::error::Error>> {
        let result = self
            .instance
            .instance()
            .function("getStakeMinimum", &[])?
            .call()
            .await?;

        let minimum: U256 = result
            .into_iter()
            .next()
            .map(|value| value.as_uint().unwrap_or_default())
            .unwrap_or_default()
            .0;
        Ok(minimum)
    }

    pub async fn get_stake(&self, staker: Address) -> Result<U256, Box<dyn std::error::Error>> {
        let result = self
            .instance
            .instance()
            .function("getStake", &[staker.into()])?
            .call()
            .await?;

        Ok(result[0].as_uint().unwrap_or_default().0)
    }

    pub async fn calculate_stake(
        &self,
        compute_units: U256,
        provider_total_compute: U256,
    ) -> Result<U256, Box<dyn std::error::Error>> {
        let min_stake_per_unit = self.get_stake_minimum().await?;
        let total_compute = provider_total_compute + compute_units + U256::from(1);
        let required_stake = total_compute * min_stake_per_unit;
        Ok(required_stake)
    }
}
