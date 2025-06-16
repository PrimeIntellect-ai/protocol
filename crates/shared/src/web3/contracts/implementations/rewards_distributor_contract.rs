use crate::web3::contracts::core::contract::Contract;
use crate::web3::contracts::structs::rewards_distributor::{NodeBucketInfo, NodeRewards};
use crate::web3::wallet::WalletProvider;
use alloy::primitives::{Address, U256};

#[derive(Clone)]
pub struct RewardsDistributor<P: alloy_provider::Provider> {
    pub instance: Contract<P>,
}

impl<P: alloy_provider::Provider> RewardsDistributor<P> {
    pub fn new(rewards_distributor_address: Address, provider: P, abi_file_path: &str) -> Self {
        let instance = Contract::new(rewards_distributor_address, provider, abi_file_path);
        Self { instance }
    }

    /// Calculate rewards for a specific node
    /// Returns (claimable_tokens, locked_tokens)
    pub async fn calculate_rewards(
        &self,
        node: Address,
    ) -> Result<(U256, U256), Box<dyn std::error::Error>> {
        let result = self
            .instance
            .instance()
            .function("calculateRewards", &[node.into()])?
            .call()
            .await?;

        let claimable = result
            .first()
            .ok_or("Missing claimable rewards in response")?
            .as_uint()
            .ok_or("Failed to parse claimable rewards as uint")?
            .0;

        let locked = result
            .get(1)
            .ok_or("Missing locked rewards in response")?
            .as_uint()
            .ok_or("Failed to parse locked rewards as uint")?
            .0;

        Ok((claimable, locked))
    }

    /// Get detailed node information including bucket data
    pub async fn get_node_info(
        &self,
        node: Address,
    ) -> Result<NodeBucketInfo, Box<dyn std::error::Error>> {
        let result = self
            .instance
            .instance()
            .function("nodeInfo", &[node.into()])?
            .call()
            .await?;

        let last_24h = result
            .first()
            .ok_or("Missing last_24h in nodeInfo response")?
            .as_uint()
            .ok_or("Failed to parse last_24h as uint")?
            .0;

        let total_all = result
            .get(1)
            .ok_or("Missing total_all in nodeInfo response")?
            .as_uint()
            .ok_or("Failed to parse total_all as uint")?
            .0;

        let last_claimed = result
            .get(2)
            .ok_or("Missing last_claimed in nodeInfo response")?
            .as_uint()
            .ok_or("Failed to parse last_claimed as uint")?
            .0;

        let is_active = result
            .get(3)
            .ok_or("Missing is_active in nodeInfo response")?
            .as_bool()
            .ok_or("Failed to parse is_active as bool")?;

        Ok(NodeBucketInfo {
            last_24h,
            total_all,
            last_claimed,
            is_active,
        })
    }

    /// Get the current reward rate per unit
    pub async fn get_reward_rate(&self) -> Result<U256, Box<dyn std::error::Error>> {
        let result = self
            .instance
            .instance()
            .function("getRewardRate", &[])?
            .call()
            .await?;

        let rate = result
            .first()
            .ok_or("Missing reward rate in response")?
            .as_uint()
            .ok_or("Failed to parse reward rate as uint")?
            .0;

        Ok(rate)
    }

    /// Check if a node has any claimable rewards
    pub async fn has_claimable_rewards(
        &self,
        node: Address,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let (claimable, _) = self.calculate_rewards(node).await?;
        Ok(claimable > U256::ZERO)
    }

    /// Get detailed rewards information for a node
    pub async fn get_node_rewards_details(
        &self,
        node: Address,
        provider: Address,
    ) -> Result<NodeRewards, Box<dyn std::error::Error>> {
        let (claimable, locked) = self.calculate_rewards(node).await?;
        let node_info = self.get_node_info(node).await?;

        Ok(NodeRewards {
            claimable_tokens: claimable,
            locked_tokens: locked,
            total_rewards: claimable + locked,
            is_active: node_info.is_active,
            provider,
        })
    }
}

impl RewardsDistributor<WalletProvider> {
    /// Claim rewards for a node (only callable by node provider)
    pub async fn claim_rewards(
        &self,
        node: Address,
    ) -> Result<alloy::primitives::FixedBytes<32>, Box<dyn std::error::Error>> {
        let result = self
            .instance
            .instance()
            .function("claimRewards", &[node.into()])?
            .send()
            .await?
            .watch()
            .await?;

        Ok(result)
    }

    /// Set the reward rate (only callable by rewards manager)
    pub async fn set_reward_rate(
        &self,
        new_rate: U256,
    ) -> Result<alloy::primitives::FixedBytes<32>, Box<dyn std::error::Error>> {
        let result = self
            .instance
            .instance()
            .function("setRewardRate", &[new_rate.into()])?
            .send()
            .await?
            .watch()
            .await?;

        Ok(result)
    }

    /// Slash pending rewards for a node (only callable by rewards manager or compute pool)
    pub async fn slash_pending_rewards(
        &self,
        node: Address,
    ) -> Result<alloy::primitives::FixedBytes<32>, Box<dyn std::error::Error>> {
        let result = self
            .instance
            .instance()
            .function("slashPendingRewards", &[node.into()])?
            .send()
            .await?
            .watch()
            .await?;

        Ok(result)
    }

    /// Remove specific work units from a node (soft slash)
    pub async fn remove_work(
        &self,
        node: Address,
        work_units: U256,
    ) -> Result<alloy::primitives::FixedBytes<32>, Box<dyn std::error::Error>> {
        let result = self
            .instance
            .instance()
            .function("removeWork", &[node.into(), work_units.into()])?
            .send()
            .await?
            .watch()
            .await?;

        Ok(result)
    }
}
