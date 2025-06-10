use crate::store::core::RedisStore;
use alloy::primitives::Address;
use anyhow::{anyhow, Result};
use redis::AsyncCommands;
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

    pub async fn beat(&self, payload: &HeartbeatRequest) -> Result<()> {
        let address =
            Address::from_str(&payload.address).map_err(|_| anyhow!("Failed to parse address"))?;
        let key = format!("{}:{}", ORCHESTRATOR_HEARTBEAT_KEY, address);

        let payload_string =
            serde_json::to_string(payload).map_err(|_| anyhow!("Failed to serialize payload"))?;
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;
        con.set_options::<_, _, ()>(
            &key,
            payload_string,
            redis::SetOptions::default().with_expiration(redis::SetExpiry::EX(60)),
        )
        .await
        .map_err(|_| anyhow!("Failed to set options"))?;

        Ok(())
    }

    pub async fn get_heartbeat(&self, address: &Address) -> Result<Option<HeartbeatRequest>> {
        let mut con = self
            .redis
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|_| anyhow!("Failed to get connection"))?;
        let key = format!("{}:{}", ORCHESTRATOR_HEARTBEAT_KEY, address);
        let value: Option<String> = con
            .get(key)
            .await
            .map_err(|_| anyhow!("Failed to get value"))?;
        Ok(value.and_then(|v| serde_json::from_str(&v).ok()))
    }

    pub async fn get_unhealthy_counter(&self, address: &Address) -> Result<u32> {
        let mut con = self
            .redis
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|_| anyhow!("Failed to get connection"))?;
        let key = format!("{}:{}", ORCHESTRATOR_UNHEALTHY_COUNTER_KEY, address);
        let value: Option<String> = con
            .get(key)
            .await
            .map_err(|_| anyhow!("Failed to get value"))?;
        match value {
            Some(value) => value
                .parse::<u32>()
                .map_err(|_| anyhow!("Failed to parse counter value")),
            None => Ok(0),
        }
    }

    #[cfg(test)]
    pub async fn set_unhealthy_counter(&self, address: &Address, counter: u32) -> Result<()> {
        let mut con = self
            .redis
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|_| anyhow!("Failed to get connection"))?;
        let key = format!("{}:{}", ORCHESTRATOR_UNHEALTHY_COUNTER_KEY, address);
        con.set(key, counter.to_string())
            .await
            .map_err(|_| anyhow!("Failed to set value"))
    }

    pub async fn increment_unhealthy_counter(&self, address: &Address) -> Result<()> {
        let mut con = self
            .redis
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|_| anyhow!("Failed to get connection"))?;
        let key = format!("{}:{}", ORCHESTRATOR_UNHEALTHY_COUNTER_KEY, address);
        con.incr(key, 1)
            .await
            .map_err(|_| anyhow!("Failed to increment value"))
    }

    pub async fn clear_unhealthy_counter(&self, address: &Address) -> Result<()> {
        let mut con = self
            .redis
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|_| anyhow!("Failed to get connection"))?;
        let key = format!("{}:{}", ORCHESTRATOR_UNHEALTHY_COUNTER_KEY, address);
        con.del(key)
            .await
            .map_err(|_| anyhow!("Failed to delete value"))
    }
}
