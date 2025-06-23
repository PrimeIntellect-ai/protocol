use crate::models::node::NodeStatus;
use crate::models::node::OrchestratorNode;
use crate::store::core::RedisStore;
use alloy::primitives::Address;
use anyhow::Result;
use chrono::{DateTime, Utc};
use log::info;
use redis::AsyncCommands;
use shared::models::heartbeat::TaskDetails;
use shared::models::node::NodeLocation;
use shared::models::task::TaskState;
use std::collections::HashMap;
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
    // convert orchestrator node to redis hash fields
    fn node_to_hash_fields(node: &OrchestratorNode) -> Result<Vec<(String, String)>> {
        let mut fields = vec![
            ("address".to_string(), node.address.to_string()),
            ("ip_address".to_string(), node.ip_address.clone()),
            ("port".to_string(), node.port.to_string()),
        ];

        let status_json = serde_json::to_string(&node.status)
            .map_err(|e| anyhow::anyhow!("Failed to serialize status: {}", e))?;
        fields.push(("status".to_string(), status_json));

        if let Some(task_id) = &node.task_id {
            fields.push(("task_id".to_string(), task_id.clone()));
        }
        if let Some(task_state) = &node.task_state {
            let task_state_json = serde_json::to_string(task_state)
                .map_err(|e| anyhow::anyhow!("Failed to serialize task_state: {}", e))?;
            fields.push(("task_state".to_string(), task_state_json));
        }
        if let Some(task_details) = &node.task_details {
            let task_details_json = serde_json::to_string(task_details)
                .map_err(|e| anyhow::anyhow!("Failed to serialize task_details: {}", e))?;
            fields.push(("task_details".to_string(), task_details_json));
        }
        if let Some(version) = &node.version {
            fields.push(("version".to_string(), version.clone()));
        }
        if let Some(p2p_id) = &node.p2p_id {
            fields.push(("p2p_id".to_string(), p2p_id.clone()));
        }
        if let Some(last_status_change) = &node.last_status_change {
            fields.push((
                "last_status_change".to_string(),
                last_status_change.to_rfc3339(),
            ));
        }
        if let Some(first_seen) = &node.first_seen {
            fields.push(("first_seen".to_string(), first_seen.to_rfc3339()));
        }
        if let Some(compute_specs) = &node.compute_specs {
            let compute_specs_json = serde_json::to_string(compute_specs)
                .map_err(|e| anyhow::anyhow!("Failed to serialize compute_specs: {}", e))?;
            fields.push(("compute_specs".to_string(), compute_specs_json));
        }
        if let Some(worker_p2p_id) = &node.worker_p2p_id {
            fields.push(("worker_p2p_id".to_string(), worker_p2p_id.clone()));
        }
        if let Some(worker_p2p_addresses) = &node.worker_p2p_addresses {
            let worker_p2p_addresses_json = serde_json::to_string(worker_p2p_addresses)
                .map_err(|e| anyhow::anyhow!("Failed to serialize worker_p2p_addresses: {}", e))?;
            fields.push((
                "worker_p2p_addresses".to_string(),
                worker_p2p_addresses_json,
            ));
        }
        if let Some(location) = &node.location {
            let location_json = serde_json::to_string(location)
                .map_err(|e| anyhow::anyhow!("Failed to serialize location: {}", e))?;
            fields.push(("location".to_string(), location_json));
        }
        if let Some(balance) = &node.balance {
            fields.push(("balance".to_string(), balance.to_string()));
        }
        Ok(fields)
    }

    // Helper method to convert Redis hash fields to OrchestratorNode
    fn hash_fields_to_node(fields: HashMap<String, String>) -> Result<OrchestratorNode> {
        let address = fields
            .get("address")
            .ok_or_else(|| anyhow::anyhow!("Missing address field"))?
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid address format"))?;

        let ip_address = fields
            .get("ip_address")
            .ok_or_else(|| anyhow::anyhow!("Missing ip_address field"))?
            .clone();

        let port = fields
            .get("port")
            .ok_or_else(|| anyhow::anyhow!("Missing port field"))?
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid port format"))?;

        let status = fields
            .get("status")
            .ok_or_else(|| anyhow::anyhow!("Missing status field"))
            .and_then(|s| {
                serde_json::from_str(s).map_err(|e| anyhow::anyhow!("Invalid status format: {}", e))
            })?;

        let task_id = fields.get("task_id").cloned();
        let task_state = fields
            .get("task_state")
            .and_then(|s| serde_json::from_str(s).ok());
        let task_details = fields
            .get("task_details")
            .and_then(|s| serde_json::from_str(s).ok());
        let version = fields.get("version").cloned();
        let p2p_id = fields.get("p2p_id").cloned();
        let last_status_change = fields
            .get("last_status_change")
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));
        let first_seen = fields
            .get("first_seen")
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));
        let compute_specs = fields
            .get("compute_specs")
            .and_then(|s| serde_json::from_str(s).ok());
        let worker_p2p_id = fields.get("worker_p2p_id").cloned();
        let worker_p2p_addresses = fields
            .get("worker_p2p_addresses")
            .and_then(|s| serde_json::from_str(s).ok());
        let location = fields
            .get("location")
            .and_then(|s| serde_json::from_str(s).ok());

        let balance = fields
            .get("balance")
            .and_then(|s| {
                Some(
                    s.parse::<alloy::primitives::U256>()
                        .map_err(|_| anyhow::anyhow!("invalid balance format")),
                )
            })
            .transpose()?;

        Ok(OrchestratorNode {
            address,
            ip_address,
            port,
            status,
            task_id,
            task_state,
            task_details,
            version,
            p2p_id,
            last_status_change,
            first_seen,
            compute_specs,
            worker_p2p_id,
            worker_p2p_addresses,
            location,
            balance,
        })
    }

    pub async fn get_nodes(&self) -> Result<Vec<OrchestratorNode>> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;

        let addresses: Vec<String> = con.smembers(ORCHESTRATOR_NODE_INDEX).await?;

        if addresses.is_empty() {
            return Ok(Vec::new());
        }

        let mut nodes = Vec::new();

        // Use pipeline for efficient bulk hash retrieval
        let mut pipe = redis::pipe();
        for address in &addresses {
            let key = format!("{}:{}", ORCHESTRATOR_BASE_KEY, address);
            pipe.hgetall(&key);
        }

        let hash_results: Vec<HashMap<String, String>> = pipe.query_async(&mut con).await?;

        for fields in hash_results {
            if !fields.is_empty() {
                match Self::hash_fields_to_node(fields) {
                    Ok(node) => nodes.push(node),
                    Err(e) => {
                        info!("Failed to deserialize node: {}", e);
                    }
                }
            }
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

        // Always use hash format for new nodes
        let fields = Self::node_to_hash_fields(&node)?;
        let key = format!("{}:{}", ORCHESTRATOR_BASE_KEY, node.address);

        // Use Redis transaction (MULTI/EXEC) to ensure atomic execution of both operations
        let mut pipe = redis::pipe();
        pipe.atomic()
            .sadd(ORCHESTRATOR_NODE_INDEX, node.address.to_string())
            .hset_multiple(&key, &fields);

        let _: () = pipe.query_async(&mut con).await?;
        Ok(())
    }

    pub async fn get_node(&self, address: &Address) -> Result<Option<OrchestratorNode>> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;
        let key = format!("{}:{}", ORCHESTRATOR_BASE_KEY, address);

        let fields: HashMap<String, String> = con.hgetall(&key).await?;

        if fields.is_empty() {
            return Ok(None);
        }

        match Self::hash_fields_to_node(fields) {
            Ok(node) => Ok(Some(node)),
            Err(e) => {
                info!("Failed to deserialize node {}: {}", address, e);
                Ok(None)
            }
        }
    }

    pub async fn get_uninvited_nodes(&self) -> Result<Vec<OrchestratorNode>> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;

        let addresses: Vec<String> = con.smembers(ORCHESTRATOR_NODE_INDEX).await?;

        if addresses.is_empty() {
            return Ok(Vec::new());
        }

        let mut nodes = Vec::new();

        // Use pipeline for efficient bulk hash retrieval
        let mut pipe = redis::pipe();
        for address in &addresses {
            let key = format!("{}:{}", ORCHESTRATOR_BASE_KEY, address);
            pipe.hgetall(&key);
        }

        let hash_results: Vec<HashMap<String, String>> = pipe.query_async(&mut con).await?;

        for fields in hash_results {
            if !fields.is_empty() {
                match Self::hash_fields_to_node(fields) {
                    Ok(node) if matches!(node.status, NodeStatus::Discovered) => {
                        nodes.push(node);
                    }
                    Ok(_) => {} // Node exists but not in Discovered status
                    Err(e) => {
                        info!("Failed to deserialize node: {}", e);
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
        let node_key = format!("{}:{}", ORCHESTRATOR_BASE_KEY, node_address);

        // Update only the specific fields we need to change
        let status_json = serde_json::to_string(&status)?;
        let last_status_change = chrono::Utc::now().to_rfc3339();

        let mut pipe = redis::pipe();
        pipe.atomic().hset(&node_key, "status", status_json).hset(
            &node_key,
            "last_status_change",
            last_status_change,
        );

        let _: () = pipe.query_async(&mut con).await?;
        Ok(())
    }

    pub async fn update_node_version(&self, node_address: &Address, version: &str) -> Result<()> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;
        let node_key = format!("{}:{}", ORCHESTRATOR_BASE_KEY, node_address);

        // Update only the version field
        let _: () = con.hset(&node_key, "version", version).await?;
        Ok(())
    }

    pub async fn update_node_location(
        &self,
        node_address: &Address,
        location: &NodeLocation,
    ) -> Result<()> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;
        let node_key = format!("{}:{}", ORCHESTRATOR_BASE_KEY, node_address);

        // Update only the location field
        let location_json = serde_json::to_string(location)?;
        let _: () = con.hset(&node_key, "location", location_json).await?;
        Ok(())
    }

    pub async fn update_node_p2p_id(&self, node_address: &Address, p2p_id: &str) -> Result<()> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;
        let node_key = format!("{}:{}", ORCHESTRATOR_BASE_KEY, node_address);

        // Update only the p2p_id field
        let _: () = con.hset(&node_key, "p2p_id", p2p_id).await?;
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
        let node_key = format!("{}:{}", ORCHESTRATOR_BASE_KEY, node_address);

        // Build the update pipeline based on what fields need to be updated
        let mut pipe = redis::pipe();
        pipe.atomic();

        match (current_task, task_state, task_details) {
            (Some(task), Some(state), details) => {
                // Update task-related fields
                pipe.hset(&node_key, "task_id", task);

                let task_state_enum = TaskState::from(state.as_str());
                let task_state_json = serde_json::to_string(&task_state_enum)?;
                pipe.hset(&node_key, "task_state", task_state_json);

                if let Some(details) = details {
                    let details_json = serde_json::to_string(&details)?;
                    pipe.hset(&node_key, "task_details", details_json);
                } else {
                    pipe.hdel(&node_key, "task_details");
                }
            }
            _ => {
                // Clear all task-related fields
                pipe.hdel(&node_key, vec!["task_id", "task_state", "task_details"]);
            }
        }

        let _: () = pipe.query_async(&mut con).await?;
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
