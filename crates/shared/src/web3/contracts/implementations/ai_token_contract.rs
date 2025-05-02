use crate::web3::contracts::constants::addresses::{AI_TOKEN_ADDRESS, PRIME_NETWORK_ADDRESS};
use crate::web3::contracts::core::contract::Contract;
use crate::web3::contracts::helpers::utils::PrimeCallBuilder;
use crate::web3::wallet::Wallet;
use alloy::primitives::{Address, FixedBytes, U256};

#[derive(Clone)]
pub struct AIToken {
    pub instance: Contract,
}

impl AIToken {
    pub fn new(wallet: &Wallet, abi_file_path: &str) -> Self {
        let instance = Contract::new(AI_TOKEN_ADDRESS, wallet, abi_file_path);
        Self { instance }
    }

    pub async fn balance_of(&self, account: Address) -> Result<U256, Box<dyn std::error::Error>> {
        let balance: U256 = self
            .instance
            .instance()
            .function("balanceOf", &[account.into()])?
            .call()
            .await?
            .into_iter()
            .next()
            .map(|value| value.as_uint().unwrap_or_default())
            .unwrap_or_default()
            .0;
        Ok(balance)
    }

    /// Approves the specified amount of tokens to be spent by the PRIME network address.
    ///
    /// # Parameters
    /// - `amount`: The amount of tokens to approve for spending.
    ///
    /// # Returns
    /// - `Result<(), Box<dyn std::error::Error>>`: Returns `Ok(())` if the approval transaction is successful,
    ///   or an error if the transaction fails.
    pub async fn approve(
        &self,
        amount: U256,
    ) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
        let tx = self
            .instance
            .instance()
            .function("approve", &[PRIME_NETWORK_ADDRESS.into(), amount.into()])?
            .send()
            .await?
            .watch()
            .await?;

        Ok(tx)
    }

    pub fn build_mint_call(
        &self,
        to: Address,
        amount: U256,
    ) -> Result<PrimeCallBuilder<'_>, Box<dyn std::error::Error>> {
        let call = self
            .instance
            .instance()
            .function("mint", &[to.into(), amount.into()])?;

        Ok(call)
    }

    pub async fn mint(
        &self,
        to: Address,
        amount: U256,
    ) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
        let call = self.build_mint_call(to, amount)?;
        let tx = call.send().await?.watch().await?;
        Ok(tx)
    }
}
