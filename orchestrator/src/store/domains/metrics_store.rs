use crate::store::core::RedisStore;
use alloy::primitives::Address;
use redis::Commands;
use shared::models::heartbeat::HeartbeatRequest;
use std::str::FromStr;
use std::sync::Arc;

const ORCHESTRATOR_METRICS_STORE: &str = "orchestrator:metrics";

pub struct MetricsStore{
    redis: Arc<RedisStore>,
}

impl MetricsStore{
    pub fn new(redis: Arc<RedisStore>) -> Self {
        Self { redis }
    }

    pub fn store_metrics(){

    }

    pub fn get_metrics(){

    }

    pub fn get_metrics_for_task(){
        
    }
}

