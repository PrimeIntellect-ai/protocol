use crate::web3::contracts::constants::addresses::COMPUTE_POOL_ADDRESS;
use crate::web3::contracts::core::contract::Contract;
use crate::web3::contracts::helpers::utils::{get_selector, PrimeCallBuilder};
use crate::web3::contracts::structs::compute_pool::{PoolInfo, PoolStatus};
use crate::web3::wallet::Wallet;
use alloy::dyn_abi::DynSolValue;
use alloy::primitives::{Address, FixedBytes, U256};

#[derive(Clone)]
pub struct ComputePool {
    pub instance: Contract,
}

impl ComputePool {
    pub fn new(wallet: &Wallet, abi_file_path: &str) -> Self {
        let instance = Contract::new(COMPUTE_POOL_ADDRESS, wallet, abi_file_path);
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
            _ => panic!("Unknown status value: {}", status),
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

    pub fn build_join_compute_pool_call(
        &self,
        pool_id: U256,
        provider_address: Address,
        nodes: Vec<Address>,
        signatures: Vec<FixedBytes<65>>,
    ) -> Result<PrimeCallBuilder, Box<dyn std::error::Error>> {
        let join_compute_pool_selector =
            get_selector("joinComputePool(uint256,address,address[],bytes[])");
        let address = DynSolValue::from(
            nodes
                .iter()
                .map(|addr| DynSolValue::from(*addr))
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
            &[pool_id.into(), provider_address.into(), address, signatures],
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

    pub async fn build_work_submission_call(
        &self,
        pool_id: U256,
        node: Address,
        data: Vec<u8>,
    ) -> Result<PrimeCallBuilder, Box<dyn std::error::Error>> {
        // Extract the work key from the first 32 bytes
        // Create a new data vector with work key and work units (set to 1)
        let mut submit_data = Vec::with_capacity(64);
        submit_data.extend_from_slice(&data[0..32]); // Work key

        // We leave work units simple for now and only set this to 1 (1 file = 1 work unit)
        let work_units = U256::from(1);
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
        println!("Result: {:?}", result);
        Ok(result)
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
        Ok(result.first().unwrap().as_bool().unwrap())
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

    pub async fn eject_node(
        &self,
        pool_id: u32,
        node: Address,
    ) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
        println!("Ejecting node");

        let arg_pool_id: U256 = U256::from(pool_id);

        let result = self
            .instance
            .instance()
            .function("ejectNode", &[arg_pool_id.into(), node.into()])?
            .send()
            .await?
            .watch()
            .await?;
        println!("Result: {:?}", result);
        Ok(result)
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
        Ok(result.first().unwrap().as_bool().unwrap())
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
