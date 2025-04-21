use crate::web3::contracts::constants::addresses::PRIME_NETWORK_ADDRESS;
use crate::web3::contracts::core::contract::Contract;
use crate::web3::wallet::Wallet;
use alloy::dyn_abi::DynSolValue;
use alloy::primitives::{keccak256, Address, FixedBytes, U256};
use alloy::providers::Provider;
use anyhow::Error;

#[derive(Clone)]
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

    pub async fn stake(
        &self,
        additional_stake: U256,
    ) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
        let stake_tx = self
            .instance
            .instance()
            .function("increaseStake", &[additional_stake.into()])?
            .send()
            .await?
            .watch()
            .await?;

        Ok(stake_tx)
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

    pub async fn remove_compute_node(
        &self,
        provider_address: Address,
        node_address: Address,
    ) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
        let remove_node_tx = self
            .instance
            .instance()
            .function(
                "removeComputeNode",
                &[provider_address.into(), node_address.into()],
            )?
            .send()
            .await?
            .watch()
            .await?;

        Ok(remove_node_tx)
    }

    pub async fn validate_node(
        &self,
        provider_address: Address,
        node_address: Address,
    ) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
        let validate_node_tx = self
            .instance
            .instance()
            .function(
                "validateNode",
                &[provider_address.into(), node_address.into()],
            )?
            .send()
            .await?
            .watch()
            .await?;
        Ok(validate_node_tx)
    }

    pub async fn create_domain(
        &self,
        domain_name: String,
        validation_logic: Address,
        domain_uri: String,
    ) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
        let create_domain_tx = self
            .instance
            .instance()
            .function(
                "createDomain",
                &[
                    domain_name.into(),
                    validation_logic.into(),
                    domain_uri.into(),
                ],
            )?
            .send()
            .await?
            .watch()
            .await?;

        Ok(create_domain_tx)
    }

    pub async fn update_validation_logic(
        &self,
        domain_id: U256,
        validation_logic: Address,
    ) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
        let update_validation_logic_tx = self
            .instance
            .instance()
            .function(
                "updateDomainValidationLogic",
                &[domain_id.into(), validation_logic.into()],
            )?
            .send()
            .await?
            .watch()
            .await?;

        Ok(update_validation_logic_tx)
    }

    pub async fn set_stake_minimum(
        &self,
        min_stake_amount: U256,
    ) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
        let set_stake_minimum_tx = self
            .instance
            .instance()
            .function("setStakeMinimum", &[min_stake_amount.into()])?
            .send()
            .await?
            .watch()
            .await?;
        Ok(set_stake_minimum_tx)
    }

    pub async fn whitelist_provider(
        &self,
        provider_address: Address,
    ) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
        let whitelist_provider_tx = self
            .instance
            .instance()
            .function("whitelistProvider", &[provider_address.into()])?
            .send()
            .await?
            .watch()
            .await?;

        let receipt = self
            .instance
            .provider()
            .get_transaction_receipt(whitelist_provider_tx)
            .await?;
        println!("Receipt: {:?}", receipt);

        Ok(whitelist_provider_tx)
    }

    pub async fn invalidate_work(
        &self,
        pool_id: U256,
        penalty: U256,
        data: Vec<u8>,
    ) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
        let invalidate_work_tx = self
            .instance
            .instance()
            .function(
                "invalidateWork",
                &[pool_id.into(), penalty.into(), data.into()],
            )?
            .send()
            .await?
            .watch()
            .await?;

        Ok(invalidate_work_tx)
    }

    pub async fn get_validator_role(&self) -> Result<Vec<Address>, Error> {
        let hash = keccak256(b"VALIDATOR_ROLE");
        let value = DynSolValue::FixedBytes(hash, 32);
        let members = self
            .instance
            .instance()
            .function("getRoleMembers", &[value])?
            .call()
            .await?;

        let mut members_vec = Vec::new();
        for member in members {
            if let Some(array) = member.as_array() {
                for address in array {
                    if let Some(addr) = address.as_address() {
                        members_vec.push(addr);
                    } else {
                        return Err(Error::msg("Failed to convert member to address"));
                    }
                }
            } else {
                return Err(Error::msg("Member is not an array"));
            }
        }

        Ok(members_vec)
    }

    pub async fn reclaim_stake(
        &self,
        amount: U256,
    ) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
        let reclaim_tx = self
            .instance
            .instance()
            .function("reclaimStake", &[amount.into()])?
            .send()
            .await?
            .watch()
            .await?;

        Ok(reclaim_tx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web3::wallet::Wallet;
    use url::Url;

    #[tokio::test]
    #[ignore = "This test requires a running blockchain with deployed contracts"]
    async fn test_get_validator_role() {
        // This test requires:
        // 1. A running local blockchain (e.g. anvil or ganache) at http://localhost:8545
        // 2. The PrimeNetwork contract deployed with known address
        // 3. At least one validator role assigned

        let wallet = Wallet::new(
            "0x0000000000000000000000000000000000000000000000000000000000000001",
            Url::parse("http://localhost:8545").unwrap(),
        )
        .unwrap();

        let prime_network_contract = PrimeNetworkContract::new(&wallet, "prime_network.json");
        let validators = prime_network_contract.get_validator_role().await.unwrap();
        assert_eq!(validators.len(), 1, "Expected exactly one validator");
    }
}
