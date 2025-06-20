use crate::models::node::NodeStatus;
use crate::models::node::OrchestratorNode;
use crate::store::core::RedisStore;
use alloy::primitives::Address;
use anyhow::Result;
use log::info;
use redis::AsyncCommands;
use redis::Value;
use shared::models::heartbeat::TaskDetails;
use shared::models::task::TaskState;
use std::sync::Arc;

const ORCHESTRATOR_BASE_KEY: &str = "orchestrator:node";
const ORCHESTRATOR_NODE_INDEX: &str = "orchestrator:node_index";

pub struct NodeStore {
    redis: Arc<RedisStore>,
}

impl NodeStore {
    pub fn new(redis: Arc<RedisStore>) -> Self {
        Self { redis }
    }

    pub async fn get_nodes(&self) -> Result<Vec<OrchestratorNode>> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;

        let addresses: Vec<String> = con.smembers(ORCHESTRATOR_NODE_INDEX).await?;

        if addresses.is_empty() {
            return Ok(Vec::new());
        }

        let keys: Vec<String> = addresses
            .iter()
            .map(|addr| format!("{}:{}", ORCHESTRATOR_BASE_KEY, addr))
            .collect();

        let node_values: Vec<Option<String>> = con.mget(&keys).await?;

        let mut nodes: Vec<OrchestratorNode> = node_values
            .into_iter()
            .filter_map(|value| value.map(|s| OrchestratorNode::from_string(&s)))
            .collect();

        nodes.sort_by(|a, b| match (&a.status, &b.status) {
            (NodeStatus::Healthy, NodeStatus::Healthy) => std::cmp::Ordering::Equal,
            (NodeStatus::Healthy, _) => std::cmp::Ordering::Less,
            (_, NodeStatus::Healthy) => std::cmp::Ordering::Greater,
            (NodeStatus::Discovered, NodeStatus::Discovered) => std::cmp::Ordering::Equal,
            (NodeStatus::Discovered, _) => std::cmp::Ordering::Less,
            (_, NodeStatus::Discovered) => std::cmp::Ordering::Greater,
            (NodeStatus::Dead, NodeStatus::Dead) => std::cmp::Ordering::Equal,
            (NodeStatus::Dead, _) => std::cmp::Ordering::Greater,
            (_, NodeStatus::Dead) => std::cmp::Ordering::Less,
            _ => std::cmp::Ordering::Equal,
        });

        Ok(nodes)
    }
    pub async fn add_node(&self, node: OrchestratorNode) -> Result<()> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;

        // Use Redis transaction (MULTI/EXEC) to ensure atomic execution of both operations
        let mut pipe = redis::pipe();
        pipe.atomic()
            .sadd(ORCHESTRATOR_NODE_INDEX, node.address.to_string())
            .set(
                format!("{}:{}", ORCHESTRATOR_BASE_KEY, node.address),
                node.to_string(),
            );

        let _: () = pipe.query_async(&mut con).await?;
        Ok(())
    }

    pub async fn get_node(&self, address: &Address) -> Result<Option<OrchestratorNode>> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;

        let node_string: Option<String> = match con
            .get::<_, Option<String>>(format!("{}:{}", ORCHESTRATOR_BASE_KEY, address))
            .await
        {
            Ok(value) => value,
            Err(_) => return Ok(None),
        };

        Ok(node_string.map(|node_string| OrchestratorNode::from_string(&node_string)))
    }

    pub async fn get_uninvited_nodes(&self) -> Result<Vec<OrchestratorNode>> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;

        let addresses: Vec<String> = con.smembers(ORCHESTRATOR_NODE_INDEX).await?;

        if addresses.is_empty() {
            return Ok(Vec::new());
        }

        let keys: Vec<String> = addresses
            .iter()
            .map(|addr| format!("{}:{}", ORCHESTRATOR_BASE_KEY, addr))
            .collect();

        let node_values: Vec<Option<String>> = con.mget(&keys).await?;

        let nodes: Vec<OrchestratorNode> = node_values
            .into_iter()
            .filter_map(|value| {
                value.and_then(|s| {
                    serde_json::from_str::<OrchestratorNode>(&s)
                        .ok()
                        .filter(|node| matches!(node.status, NodeStatus::Discovered))
                })
            })
            .collect();

        Ok(nodes)
    }

    pub async fn update_node_status(
        &self,
        node_address: &Address,
        status: NodeStatus,
    ) -> Result<()> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;
        let node_key: String = format!("{}:{}", ORCHESTRATOR_BASE_KEY, node_address);
        let node_string: String = con.get(&node_key).await?;
        let mut node: OrchestratorNode = serde_json::from_str(&node_string)?;
        node.status = status;
        node.last_status_change = Some(chrono::Utc::now());
        let node_string = node.to_string();
        let _: () = con.set(&node_key, node_string).await?;
        Ok(())
    }

    pub async fn update_node_version(&self, node_address: &Address, version: &str) -> Result<()> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;
        let node_key: String = format!("{}:{}", ORCHESTRATOR_BASE_KEY, node_address);
        let node_string: String = con.get(&node_key).await?;
        let mut node: OrchestratorNode = serde_json::from_str(&node_string)?;
        node.version = Some(version.to_string());
        let node_string = node.to_string();
        let _: () = con.set(&node_key, node_string).await?;
        Ok(())
    }

    pub async fn update_node_p2p_id(&self, node_address: &Address, p2p_id: &str) -> Result<()> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;
        let node_key: String = format!("{}:{}", ORCHESTRATOR_BASE_KEY, node_address);
        let node_string: String = con.get(&node_key).await?;
        let mut node: OrchestratorNode = serde_json::from_str(&node_string)?;
        node.p2p_id = Some(p2p_id.to_string());
        let node_string = node.to_string();
        let _: () = con.set(&node_key, node_string).await?;
        Ok(())
    }

    pub async fn update_node_task(
        &self,
        node_address: Address,
        current_task: Option<String>,
        task_state: Option<String>,
        task_details: Option<TaskDetails>,
    ) -> Result<()> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;

        let node_value: Value = con
            .get(format!("{}:{}", ORCHESTRATOR_BASE_KEY, node_address))
            .await?;

        match node_value {
            Value::BulkString(node_string) => {
                // TODO: Use from redis value
                let mut node: OrchestratorNode = serde_json::from_slice(&node_string)
                    .map_err(|_| {
                        redis::RedisError::from((
                            redis::ErrorKind::TypeError,
                            "Failed to deserialize Node from string",
                            format!("Invalid JSON string: {:?}", node_string),
                        ))
                    })
                    .unwrap();
                let task_state = task_state.map(|state| TaskState::from(state.as_str()));
                let details = (current_task, task_state, task_details);
                match details {
                    (Some(task), Some(task_state), task_details) => {
                        node.task_state = Some(task_state);
                        node.task_id = Some(task);
                        node.task_details = task_details;
                    }
                    _ => {
                        node.task_state = None;
                        node.task_id = None;
                        node.task_details = None;
                    }
                }
                let node_string = node.to_string();
                let node_key = format!("{}:{}", ORCHESTRATOR_BASE_KEY, node_address);
                let _: () = con.set(&node_key, node_string).await?;
            }
            _ => {
                info!("Cannot update node");
            }
        }
        Ok(())
    }
    pub async fn migrate_existing_nodes(&self) -> Result<()> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;

        let mut cursor = 0;
        loop {
            let (new_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .cursor_arg(cursor)
                .arg("MATCH")
                .arg(format!("{}:*", ORCHESTRATOR_BASE_KEY))
                .arg("COUNT")
                .arg(100)
                .query_async(&mut con)
                .await?;

            for key in keys {
                if let Some(address) = key.strip_prefix(&format!("{}:", ORCHESTRATOR_BASE_KEY)) {
                    let _: () = con.sadd(ORCHESTRATOR_NODE_INDEX, address).await?;
                }
            }

            if new_cursor == 0 {
                break;
            }
            cursor = new_cursor;
        }

        // Handle legacy keys with double colons (orchestrator:node::address)
        let mut cursor = 0;
        loop {
            let (new_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .cursor_arg(cursor)
                .arg("MATCH")
                .arg("orchestrator:node::*")
                .arg("COUNT")
                .arg(100)
                .query_async(&mut con)
                .await?;

            for key in keys {
                if let Some(address) = key.strip_prefix("orchestrator:node::") {
                    // Add to index
                    let _: () = con.sadd(ORCHESTRATOR_NODE_INDEX, address).await?;

                    // Get the value from the old key
                    let value: Option<String> = con.get(&key).await?;
                    if let Some(value) = value {
                        // Set the value with the correct key format
                        let new_key = format!("{}:{}", ORCHESTRATOR_BASE_KEY, address);
                        let _: () = con.set(&new_key, value).await?;

                        // Delete the old key with double colons
                        let _: () = con.del(&key).await?;
                    }
                }
            }

            if new_cursor == 0 {
                break;
            }
            cursor = new_cursor;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::api::tests::helper::create_test_app_state;
    use crate::models::node::NodeStatus;
    use crate::models::node::OrchestratorNode;
    use alloy::primitives::Address;
    use redis::AsyncCommands;
    use std::str::FromStr;

    #[tokio::test]
    async fn test_get_uninvited_nodes() {
        let app_state = create_test_app_state().await;
        let node_store = &app_state.store_context.node_store;

        let uninvited_node = OrchestratorNode {
            address: Address::from_str("0x0000000000000000000000000000000000000001").unwrap(),
            ip_address: "192.168.1.1".to_string(),
            port: 8080,
            status: NodeStatus::Discovered,
            ..Default::default()
        };

        let healthy_node = OrchestratorNode {
            address: Address::from_str("0x0000000000000000000000000000000000000002").unwrap(),
            ip_address: "192.168.1.2".to_string(),
            port: 8081,
            status: NodeStatus::Healthy,
            ..Default::default()
        };

        node_store.add_node(uninvited_node.clone()).await.unwrap();
        node_store.add_node(healthy_node.clone()).await.unwrap();

        let uninvited_nodes = node_store.get_uninvited_nodes().await.unwrap();
        assert_eq!(uninvited_nodes.len(), 1);
        assert_eq!(uninvited_nodes[0].address, uninvited_node.address);
    }

    #[tokio::test]
    async fn test_node_sorting() {
        let app_state = create_test_app_state().await;
        let node_store = &app_state.store_context.node_store;

        let nodes = vec![
            OrchestratorNode {
                address: Address::from_str("0x0000000000000000000000000000000000000003").unwrap(),
                ip_address: "192.168.1.3".to_string(),
                port: 8082,
                status: NodeStatus::Dead,
                ..Default::default()
            },
            OrchestratorNode {
                address: Address::from_str("0x0000000000000000000000000000000000000002").unwrap(),
                ip_address: "192.168.1.2".to_string(),
                port: 8081,
                status: NodeStatus::Discovered,
                ..Default::default()
            },
            OrchestratorNode {
                address: Address::from_str("0x0000000000000000000000000000000000000001").unwrap(),
                ip_address: "192.168.1.1".to_string(),
                port: 8080,
                status: NodeStatus::Healthy,
                ..Default::default()
            },
        ];
        for node in nodes {
            node_store.add_node(node).await.unwrap();
        }

        let nodes = node_store.get_nodes().await.unwrap();
        assert_eq!(nodes.len(), 3);
        assert_eq!(
            nodes[0].address,
            Address::from_str("0x0000000000000000000000000000000000000001").unwrap()
        );
        assert_eq!(
            nodes[1].address,
            Address::from_str("0x0000000000000000000000000000000000000002").unwrap()
        );
        assert_eq!(
            nodes[2].address,
            Address::from_str("0x0000000000000000000000000000000000000003").unwrap()
        );
    }

    #[tokio::test]
    async fn test_migration_handles_double_colon_keys() {
        let app_state = create_test_app_state().await;
        let node_store = &app_state.store_context.node_store;
        let mut con = app_state
            .store_context
            .node_store
            .redis
            .client
            .get_multiplexed_async_connection()
            .await
            .unwrap();

        let test_address = "0x66295E2B4A78d1Cb57Db16Ac0260024900A5BA9B";
        let test_node = OrchestratorNode {
            address: Address::from_str(test_address).unwrap(),
            ip_address: "192.168.1.1".to_string(),
            port: 8080,
            status: NodeStatus::Healthy,
            ..Default::default()
        };

        // Manually create a legacy key with double colons
        let legacy_key = format!("orchestrator:node::{}", test_address);
        let _: () = con.set(&legacy_key, test_node.to_string()).await.unwrap();

        // Verify the legacy key exists
        let exists: bool = con.exists(&legacy_key).await.unwrap();
        assert!(exists);

        // Run migration
        node_store.migrate_existing_nodes().await.unwrap();

        // Verify the legacy key is removed
        let legacy_exists: bool = con.exists(&legacy_key).await.unwrap();
        assert!(!legacy_exists);

        // Verify the correct key exists
        let correct_key = format!("orchestrator:node:{}", test_address);
        let correct_exists: bool = con.exists(&correct_key).await.unwrap();
        assert!(correct_exists);

        // Verify the node is in the index
        let in_index: bool = con
            .sismember("orchestrator:node_index", test_address)
            .await
            .unwrap();
        assert!(in_index);

        // Verify we can retrieve the node correctly
        let retrieved_node = node_store
            .get_node(&Address::from_str(test_address).unwrap())
            .await
            .unwrap();
        assert!(retrieved_node.is_some());
        assert_eq!(retrieved_node.unwrap().address, test_node.address);
    }
}
