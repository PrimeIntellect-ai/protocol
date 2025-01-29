use crate::store::core::RedisStore;
use alloy::primitives::Address;
use redis::Commands;
use shared::models::heartbeat::HeartbeatRequest;
use std::str::FromStr;
use std::sync::Arc;

const ORCHESTRATOR_UNHEALTHY_COUNTER_KEY: &str = "orchestrator:unhealthy_counter";
const ORCHESTRATOR_HEARTBEAT_KEY: &str = "orchestrator:heartbeat";

pub struct HeartbeatStore {
    redis: Arc<RedisStore>,
}

impl HeartbeatStore {
    pub fn new(redis: Arc<RedisStore>) -> Self {
        Self { redis }
    }

    pub fn beat(&self, payload: &HeartbeatRequest) {
        let mut con = self.redis.client.get_connection().unwrap();
        let address = Address::from_str(&payload.address).unwrap();
        let key = format!("{}:{}", ORCHESTRATOR_HEARTBEAT_KEY, address);

        // TODO: Metrics temporary rn
        let payload_string = serde_json::to_string(payload).unwrap();
        let _: () = con
            .set_options(
                &key,
                payload_string,
                redis::SetOptions::default().with_expiration(redis::SetExpiry::EX(60)),
            )
            .unwrap();
    }

    pub fn get_all_metrics(&self){
        let mut con = self.redis.client.get_connection().unwrap();
        let key =format!("{}:{}", ORCHESTRATOR_HEARTBEAT_KEY, "*"); 

        let keys: Vec<String> = redis::cmd("KEYS")
        .arg(&key)
        .query(&mut con)
        .unwrap_or_default();

        let key_data : Vec<Option<String>> = redis::cmd("MGET")
        .arg(&keys)
        .query(&mut con)
        .unwrap_or_default();
        println!("key_data {:?}", key_data)
    }

    pub fn get_heartbeat(&self, address: &Address) -> Option<String> {
        let mut con = self.redis.client.get_connection().unwrap();
        let key = format!("{}:{}", ORCHESTRATOR_HEARTBEAT_KEY, address);
        let value: Option<String> = con.get(key).unwrap();
        value
    }

    pub fn get_unhealthy_counter(&self, address: &Address) -> u32 {
        let mut con = self.redis.client.get_connection().unwrap();
        let key = format!("{}:{}", ORCHESTRATOR_UNHEALTHY_COUNTER_KEY, address);
        let value: Option<String> = con.get(key).unwrap();
        match value {
            Some(value) => value.parse::<u32>().unwrap(),
            None => 0,
        }
    }

    #[cfg(test)]
    pub fn set_unhealthy_counter(&self, address: &Address, counter: u32) {
        let mut con = self.redis.client.get_connection().unwrap();
        let key = format!("{}:{}", ORCHESTRATOR_UNHEALTHY_COUNTER_KEY, address);
        let _: () = con.set(key, counter.to_string()).unwrap();
    }

    pub fn increment_unhealthy_counter(&self, address: &Address) {
        let mut con = self.redis.client.get_connection().unwrap();
        let key = format!("{}:{}", ORCHESTRATOR_UNHEALTHY_COUNTER_KEY, address);
        let _: () = con.incr(key, 1).unwrap();
    }

    pub fn clear_unhealthy_counter(&self, address: &Address) {
        let mut con = self.redis.client.get_connection().unwrap();
        let key = format!("{}:{}", ORCHESTRATOR_UNHEALTHY_COUNTER_KEY, address);
        let _: () = con.del(key).unwrap();
    }
}
