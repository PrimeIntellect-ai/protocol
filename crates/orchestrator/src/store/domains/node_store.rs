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

const ORCHESTRATOR_BASE_KEY: &str = "orchestrator:node:";
pub struct NodeStore {
    redis: Arc<RedisStore>,
}

impl NodeStore {
    pub fn new(redis: Arc<RedisStore>) -> Self {
        Self { redis }
    }

    pub async fn get_nodes(&self) -> Result<Vec<OrchestratorNode>> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;
        let keys: Vec<String> = con.keys(format!("{}*", ORCHESTRATOR_BASE_KEY)).await?;

        if keys.is_empty() {
            return Ok(Vec::new());
        }

        let node_strings: Vec<String> = redis::pipe().get(&keys).query_async(&mut con).await?;

        let mut nodes: Vec<OrchestratorNode> = Vec::new();
        for node_string in node_strings {
            let node: OrchestratorNode = OrchestratorNode::from_string(&node_string);
            nodes.push(node);
        }

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
        let _: () = con
            .set(
                format!("{}:{}", ORCHESTRATOR_BASE_KEY, node.address),
                node.to_string(),
            )
            .await?;
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
        let keys: Vec<String> = con.keys(format!("{}:*", ORCHESTRATOR_BASE_KEY)).await?;
        let mut nodes: Vec<OrchestratorNode> = Vec::new();

        for key in keys {
            if let Ok(node_string) = con.get::<_, String>(&key).await {
                if let Ok(node) = serde_json::from_str::<OrchestratorNode>(&node_string) {
                    if matches!(node.status, NodeStatus::Discovered) {
                        nodes.push(node);
                    }
                }
            }
        }

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
                        if task_details.is_some() {
                            node.task_details = task_details.clone();
                        }
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
}

#[cfg(test)]
mod tests {
    use crate::api::tests::helper::create_test_app_state;
    use crate::models::node::NodeStatus;
    use crate::models::node::OrchestratorNode;
    use alloy::primitives::Address;
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
}
