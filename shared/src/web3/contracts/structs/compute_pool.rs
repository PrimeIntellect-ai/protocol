use alloy::primitives::Address;
use alloy::primitives::U256;

#[derive(Debug)]
pub enum PoolStatus {
    PENDING,
    ACTIVE,
    COMPLETED,
}

#[derive(Debug)]
pub struct PoolInfo {
    pub pool_id: U256,
    pub domain_id: U256,
    pub pool_name: String,
    pub creator: Address,
    pub compute_manager_key: Address,
    pub creation_time: U256,
    pub start_time: U256,
    pub end_time: U256,
    pub pool_data_uri: String,
    pub pool_validation_logic: Address,
    pub total_compute: U256,
    pub status: PoolStatus,
}
