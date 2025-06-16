use alloy::primitives::{Address, U256};

#[derive(Debug, Clone, PartialEq)]
pub struct NodeRewards {
    pub claimable_tokens: U256,
    pub locked_tokens: U256,
    pub total_rewards: U256,
    pub is_active: bool,
    pub provider: Address,
}

#[derive(Debug, Clone)]
pub struct NodeBucketInfo {
    pub last_24h: U256,
    pub total_all: U256,
    pub last_claimed: U256,
    pub is_active: bool,
}

#[derive(Debug, Clone)]
pub struct PoolRewardsSummary {
    pub total_claimable: U256,
    pub total_locked: U256,
    pub active_nodes: u32,
    pub nodes: Vec<Address>,
    pub node_rewards: Vec<NodeRewards>,
}

#[derive(Debug, Clone)]
pub struct RewardsDistributorInfo {
    pub pool_id: U256,
    pub reward_rate: U256,
    pub reward_token: Address,
    pub total_rewards_distributed: U256,
    pub is_active: bool,
}
