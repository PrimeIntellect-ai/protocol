use crate::web3::contracts::constants::addresses::COMPUTE_POOL_ADDRESS;
use crate::web3::contracts::core::contract::Contract;
use crate::web3::contracts::helpers::utils::{get_selector, PrimeCallBuilder};
use crate::web3::contracts::implementations::rewards_distributor_contract::RewardsDistributor;
use crate::web3::contracts::structs::compute_pool::{PoolInfo, PoolStatus};
use crate::web3::contracts::structs::rewards_distributor::{NodeRewards, PoolRewardsSummary};
use crate::web3::wallet::WalletProvider;
use alloy::dyn_abi::DynSolValue;
use alloy::primitives::{Address, FixedBytes, U256};

#[derive(Clone)]
pub struct ComputePool<P: alloy_provider::Provider> {
    pub instance: Contract<P>,
}

impl<P: alloy_provider::Provider> ComputePool<P> {
    pub fn new(provider: P, abi_file_path: &str) -> Self {
        let instance = Contract::new(COMPUTE_POOL_ADDRESS, provider, abi_file_path);
        Self { instance }
    }

    pub async fn get_pool_info(
        &self,
        pool_id: U256,
    ) -> Result<PoolInfo, Box<dyn std::error::Error>> {
        let pool_info_response = self
            .instance
            .instance()
            .function("getComputePool", &[pool_id.into()])?
            .call()
            .await?;

        let pool_info_tuple: &[DynSolValue] =
            pool_info_response.first().unwrap().as_tuple().unwrap();

        // Check if pool exists by looking at creator and compute manager addresses
        if pool_info_tuple[3].as_address().unwrap() == Address::ZERO
            && pool_info_tuple[4].as_address().unwrap() == Address::ZERO
        {
            return Err("Pool does not exist".into());
        }

        let pool_id: U256 = pool_info_tuple[0].as_uint().unwrap().0;
        let domain_id: U256 = pool_info_tuple[1].as_uint().unwrap().0;
        let name: String = pool_info_tuple[2].as_str().unwrap().to_string();
        let creator: Address = pool_info_tuple[3].as_address().unwrap();
        let compute_manager_key: Address = pool_info_tuple[4].as_address().unwrap();
        let creation_time: U256 = pool_info_tuple[5].as_uint().unwrap().0;
        let start_time: U256 = pool_info_tuple[6].as_uint().unwrap().0;
        let end_time: U256 = pool_info_tuple[7].as_uint().unwrap().0;
        let pool_data_uri: String = pool_info_tuple[8].as_str().unwrap().to_string();
        let pool_validation_logic: Address = pool_info_tuple[9].as_address().unwrap();
        let total_compute: U256 = pool_info_tuple[10].as_uint().unwrap().0;
        let compute_limit: U256 = pool_info_tuple[11].as_uint().unwrap().0;
        let status: U256 = pool_info_tuple[12].as_uint().unwrap().0;
        let status: u8 = status.try_into().expect("Failed to convert status to u8");
        let mapped_status = match status {
            0 => PoolStatus::PENDING,
            1 => PoolStatus::ACTIVE,
            2 => PoolStatus::COMPLETED,
            _ => panic!("Unknown status value: {status}"),
        };

        let pool_info = PoolInfo {
            pool_id,
            domain_id,
            pool_name: name,
            creator,
            compute_manager_key,
            creation_time,
            start_time,
            end_time,
            pool_data_uri,
            pool_validation_logic,
            total_compute,
            compute_limit,
            status: mapped_status,
        };
        Ok(pool_info)
    }

    pub async fn is_node_blacklisted(
        &self,
        pool_id: u32,
        node: Address,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let arg_pool_id: U256 = U256::from(pool_id);
        let result = self
            .instance
            .instance()
            .function(
                "isNodeBlacklistedFromPool",
                &[arg_pool_id.into(), node.into()],
            )?
            .call()
            .await?;

        let is_blacklisted = result
            .first()
            .ok_or("Missing blacklist status in response")?
            .as_bool()
            .ok_or("Failed to parse blacklist status as bool")?;

        Ok(is_blacklisted)
    }

    pub async fn get_blacklisted_nodes(
        &self,
        pool_id: u32,
    ) -> Result<Vec<Address>, Box<dyn std::error::Error>> {
        let arg_pool_id: U256 = U256::from(pool_id);
        let result = self
            .instance
            .instance()
            .function("getBlacklistedNodes", &[arg_pool_id.into()])?
            .call()
            .await?;

        let blacklisted_nodes = result
            .first()
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node.as_address().unwrap())
            .collect();

        Ok(blacklisted_nodes)
    }

    pub async fn is_node_in_pool(
        &self,
        pool_id: u32,
        node: Address,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let arg_pool_id: U256 = U256::from(pool_id);
        let result = self
            .instance
            .instance()
            .function("isNodeInPool", &[arg_pool_id.into(), node.into()])?
            .call()
            .await?;

        let is_in_pool = result
            .first()
            .ok_or("Missing node-in-pool status in response")?
            .as_bool()
            .ok_or("Failed to parse node-in-pool status as bool")?;

        Ok(is_in_pool)
    }

    /// Get the rewards distributor address for a specific pool
    pub async fn get_reward_distributor_address(
        &self,
        pool_id: U256,
    ) -> Result<Address, Box<dyn std::error::Error>> {
        let result = self
            .instance
            .instance()
            .function("getRewardDistributorForPool", &[pool_id.into()])?
            .call()
            .await?;

        let address = result
            .first()
            .ok_or("Missing rewards distributor address in response")?
            .as_address()
            .ok_or("Failed to parse rewards distributor address")?;

        Ok(address)
    }

    /// Calculate rewards for a specific node in a pool
    pub async fn calculate_node_rewards(
        &self,
        pool_id: U256,
        node: Address,
    ) -> Result<(U256, U256), Box<dyn std::error::Error>> {
        let distributor_address = self.get_reward_distributor_address(pool_id).await?;
        let rewards_distributor = RewardsDistributor::new(
            distributor_address,
            self.instance.provider(),
            "rewards_distributor.json",
        );

        rewards_distributor.calculate_rewards(node).await
    }

    /// Get detailed rewards information for a node in a pool
    pub async fn get_node_rewards_details(
        &self,
        pool_id: U256,
        node: Address,
        provider: Address,
    ) -> Result<NodeRewards, Box<dyn std::error::Error>> {
        let distributor_address = self.get_reward_distributor_address(pool_id).await?;
        let rewards_distributor = RewardsDistributor::new(
            distributor_address,
            self.instance.provider(),
            "rewards_distributor.json",
        );

        rewards_distributor
            .get_node_rewards_details(node, provider)
            .await
    }

    /// Calculate rewards for all nodes in a pool
    pub async fn calculate_pool_rewards(
        &self,
        pool_id: U256,
    ) -> Result<PoolRewardsSummary, Box<dyn std::error::Error>> {
        // Get all nodes in the pool
        let nodes = self.get_compute_pool_nodes(pool_id).await?;
        let distributor_address = self.get_reward_distributor_address(pool_id).await?;
        let rewards_distributor = RewardsDistributor::new(
            distributor_address,
            self.instance.provider(),
            "rewards_distributor.json",
        );

        let mut summary = PoolRewardsSummary {
            total_claimable: U256::ZERO,
            total_locked: U256::ZERO,
            active_nodes: 0,
            nodes: nodes.clone(),
            node_rewards: Vec::new(),
        };

        for node in &nodes {
            let (claimable, locked) = rewards_distributor.calculate_rewards(*node).await?;
            let node_info = rewards_distributor.get_node_info(*node).await?;

            let node_rewards = NodeRewards {
                claimable_tokens: claimable,
                locked_tokens: locked,
                total_rewards: claimable + locked,
                is_active: node_info.is_active,
                provider: Address::ZERO, // This would need to be fetched from compute registry
            };

            summary.total_claimable += claimable;
            summary.total_locked += locked;
            if node_info.is_active {
                summary.active_nodes += 1;
            }
            summary.node_rewards.push(node_rewards);
        }

        Ok(summary)
    }

    /// Check if a node has claimable rewards in a pool
    pub async fn has_claimable_rewards(
        &self,
        pool_id: U256,
        node: Address,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let distributor_address = self.get_reward_distributor_address(pool_id).await?;
        let rewards_distributor = RewardsDistributor::new(
            distributor_address,
            self.instance.provider(),
            "rewards_distributor.json",
        );

        rewards_distributor.has_claimable_rewards(node).await
    }

    /// Get the reward rate for a specific pool
    pub async fn get_pool_reward_rate(
        &self,
        pool_id: U256,
    ) -> Result<U256, Box<dyn std::error::Error>> {
        let distributor_address = self.get_reward_distributor_address(pool_id).await?;
        let rewards_distributor = RewardsDistributor::new(
            distributor_address,
            self.instance.provider(),
            "rewards_distributor.json",
        );

        rewards_distributor.get_reward_rate().await
    }

    /// Get all nodes in a compute pool
    pub async fn get_compute_pool_nodes(
        &self,
        pool_id: U256,
    ) -> Result<Vec<Address>, Box<dyn std::error::Error>> {
        let result = self
            .instance
            .instance()
            .function("getComputePoolNodes", &[pool_id.into()])?
            .call()
            .await?;

        let nodes_array = result
            .first()
            .ok_or("Missing nodes array in response")?
            .as_array()
            .ok_or("Failed to parse nodes as array")?;

        let mut nodes = Vec::new();
        for node_value in nodes_array {
            let node_address = node_value
                .as_address()
                .ok_or("Failed to parse node address")?;
            nodes.push(node_address);
        }

        Ok(nodes)
    }
}

impl ComputePool<WalletProvider> {
    pub fn build_join_compute_pool_call(
        &self,
        pool_id: U256,
        provider_address: Address,
        nodes: Vec<Address>,
        nonces: Vec<[u8; 32]>,
        expirations: Vec<[u8; 32]>,
        signatures: Vec<FixedBytes<65>>,
    ) -> Result<PrimeCallBuilder<'_, alloy::json_abi::Function>, Box<dyn std::error::Error>> {
        let join_compute_pool_selector =
            get_selector("joinComputePool(uint256,address,address[],uint256[],uint256[],bytes[])");
        let address = DynSolValue::from(
            nodes
                .iter()
                .map(|addr| DynSolValue::from(*addr))
                .collect::<Vec<_>>(),
        );
        let nonces = DynSolValue::from(
            nonces
                .iter()
                .map(|nonce| DynSolValue::from(U256::from_be_bytes(*nonce)))
                .collect::<Vec<_>>(),
        );
        let expirations = DynSolValue::from(
            expirations
                .iter()
                .map(|exp| DynSolValue::from(U256::from_be_bytes(*exp)))
                .collect::<Vec<_>>(),
        );
        let signatures = DynSolValue::from(
            signatures
                .iter()
                .map(|sig| DynSolValue::Bytes(sig.to_vec()))
                .collect::<Vec<_>>(),
        );
        let call = self.instance.instance().function_from_selector(
            &join_compute_pool_selector,
            &[
                pool_id.into(),
                provider_address.into(),
                address,
                nonces,
                expirations,
                signatures,
            ],
        )?;
        Ok(call)
    }

    pub async fn leave_compute_pool(
        &self,
        pool_id: U256,
        provider_address: Address,
        node: Address,
    ) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
        let leave_compute_pool_selector = get_selector("leaveComputePool(uint256,address,address)");

        let result = self
            .instance
            .instance()
            .function_from_selector(
                &leave_compute_pool_selector,
                &[pool_id.into(), provider_address.into(), node.into()],
            )?
            .send()
            .await?
            .watch()
            .await?;
        Ok(result)
    }

    pub async fn eject_node(
        &self,
        pool_id: u32,
        node: Address,
    ) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
        let arg_pool_id: U256 = U256::from(pool_id);

        let result = self
            .instance
            .instance()
            .function("ejectNode", &[arg_pool_id.into(), node.into()])?
            .send()
            .await?
            .watch()
            .await?;
        println!("Result: {result:?}");
        Ok(result)
    }

    pub fn build_work_submission_call(
        &self,
        pool_id: U256,
        node: Address,
        data: Vec<u8>,
        work_units: U256,
    ) -> Result<PrimeCallBuilder<'_, alloy::json_abi::Function>, Box<dyn std::error::Error>> {
        // Extract the work key from the first 32 bytes
        // Create a new data vector with work key and work units (set to 1)
        let mut submit_data = Vec::with_capacity(64);
        submit_data.extend_from_slice(&data[0..32]); // Work key
        submit_data.extend_from_slice(&work_units.to_be_bytes::<32>());

        let call = self.instance.instance().function(
            "submitWork",
            &[pool_id.into(), node.into(), submit_data.into()],
        )?;
        Ok(call)
    }

    pub async fn blacklist_node(
        &self,
        pool_id: u32,
        node: Address,
    ) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
        println!("Blacklisting node");

        let arg_pool_id: U256 = U256::from(pool_id);

        let result = self
            .instance
            .instance()
            .function("blacklistNode", &[arg_pool_id.into(), node.into()])?
            .send()
            .await?
            .watch()
            .await?;
        println!("Result: {result:?}");
        Ok(result)
    }

    pub async fn create_compute_pool(
        &self,
        domain_id: U256,
        compute_manager_key: Address,
        pool_name: String,
        pool_data_uri: String,
        compute_limit: U256,
    ) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
        let result = self
            .instance
            .instance()
            .function(
                "createComputePool",
                &[
                    domain_id.into(),
                    compute_manager_key.into(),
                    pool_name.into(),
                    pool_data_uri.into(),
                    compute_limit.into(),
                ],
            )?
            .send()
            .await?
            .watch()
            .await?;
        Ok(result)
    }

    pub async fn start_compute_pool(
        &self,
        pool_id: U256,
    ) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
        let result = self
            .instance
            .instance()
            .function("startComputePool", &[pool_id.into()])?
            .send()
            .await?
            .watch()
            .await?;
        Ok(result)
    }
}
