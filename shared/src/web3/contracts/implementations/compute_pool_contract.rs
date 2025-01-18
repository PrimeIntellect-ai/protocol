use crate::web3::contracts::constants::addresses::COMPUTE_POOL_ADDRESS;
use crate::web3::contracts::core::contract::Contract;
use crate::web3::contracts::structs::compute_pool::{PoolInfo, PoolStatus};
use crate::web3::wallet::Wallet;
use alloy::dyn_abi::DynSolValue;
use alloy::primitives::{Address, FixedBytes, Uint, U256, U8};

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
        // TODO: If we cannot find a cleaner version for this parsing
        // I'll fix this in the lib myself -_-
        // abi_encode looks promising
        println!("Pool info tuple: {:?}", pool_info_tuple);

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
        let status: u8 = u8::try_from(pool_info_tuple[11].as_uint().unwrap().0).unwrap();
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
            status: mapped_status,
        };
        println!("Pool info: {:?}", pool_info);
        Ok(pool_info)
    }

    pub async fn join_compute_pool(
        &self,
        pool_id: U256,
        provider_address: Address,
        nodes: Vec<Address>,
        signatures: Vec<FixedBytes<65>>,
    ) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
        println!("Joining compute pool");

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
        println!("Address: {:?}", address);
        println!("Signatures: {:?}", signatures);

        let result = self
            .instance
            .instance()
            .function(
                "joinComputePool",
                &[pool_id.into(), provider_address.into(), address, signatures],
            )?
            .send()
            .await?
            .watch()
            .await?;
        println!("Result: {:?}", result);
        Ok(result)
    }

    pub async fn create_compute_pool(
        &self,
        domain_id: U256,
        compute_manager_key: Address,
        pool_name: String,
        pool_data_uri: String,
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
