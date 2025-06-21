use crate::store::redis::RedisStore;
use anyhow::Error;
use log::error;
use redis::AsyncCommands;
use shared::models::node::{DiscoveryNode, Node};

pub struct NodeStore {
    redis_store: RedisStore,
}

impl NodeStore {
    pub fn new(redis_store: RedisStore) -> Self {
        Self { redis_store }
    }

    async fn get_connection(&self) -> Result<redis::aio::MultiplexedConnection, redis::RedisError> {
        self.redis_store
            .client
            .get_multiplexed_async_connection()
            .await
    }

    pub async fn get_node(&self, address: String) -> Result<Option<DiscoveryNode>, Error> {
        let key = format!("node:{}", address);
        let mut con = self.get_connection().await?;
        let node: Option<String> = con.get(&key).await?;
        let node = match node {
            Some(node) => serde_json::from_str(&node),
            None => Ok(None),
        }?;
        Ok(node)
    }

    pub async fn get_active_node_by_ip(&self, ip: String) -> Result<Option<DiscoveryNode>, Error> {
        let mut con = self.get_connection().await?;
        let node_ids: Vec<String> = con.smembers("node:ids").await?;

        if node_ids.is_empty() {
            return Ok(None);
        }

        let node_keys: Vec<String> = node_ids.iter().map(|id| format!("node:{}", id)).collect();
        let serialized_nodes: Vec<String> =
            redis::pipe().get(&node_keys).query_async(&mut con).await?;

        for serialized_node in serialized_nodes {
            let deserialized_node: DiscoveryNode = serde_json::from_str(&serialized_node)?;
            if deserialized_node.ip_address == ip && deserialized_node.is_active {
                return Ok(Some(deserialized_node));
            }
        }
        Ok(None)
    }

    pub async fn count_active_nodes_by_ip(&self, ip: String) -> Result<u32, Error> {
        let mut con = self.get_connection().await?;
        let node_ids: Vec<String> = con.smembers("node:ids").await?;

        if node_ids.is_empty() {
            return Ok(0);
        }

        let node_keys: Vec<String> = node_ids.iter().map(|id| format!("node:{}", id)).collect();

        let mut count = 0;
        for key in node_keys {
            let serialized_node: Option<String> = con.get(&key).await?;
            if let Some(serialized_node) = serialized_node {
                let deserialized_node: DiscoveryNode = serde_json::from_str(&serialized_node)?;
                if deserialized_node.ip_address == ip && deserialized_node.is_active {
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    pub async fn register_node(&self, node: Node) -> Result<(), Error> {
        let address = node.id.clone();
        let key = format!("node:{}", address);

        let mut con = self.get_connection().await?;

        if con.exists(&key).await? {
            let existing_node = self.get_node(address.clone()).await?;
            if let Some(existing_node) = existing_node {
                let updated_node = existing_node.with_updated_node(node);
                self.update_node(updated_node).await?;
            }
        } else {
            let discovery_node = DiscoveryNode::from(node);
            let serialized_node = serde_json::to_string(&discovery_node)?;

            let _: () = redis::pipe()
                .atomic()
                .set(&key, serialized_node)
                .sadd("node:ids", &address)
                .query_async(&mut con)
                .await?;
        }
        Ok(())
    }

    pub async fn update_node(&self, node: DiscoveryNode) -> Result<(), Error> {
        let mut con = self.get_connection().await?;
        let address = node.id.clone();
        let key = format!("node:{}", address);
        let serialized_node = serde_json::to_string(&node)?;

        let _: () = redis::pipe()
            .atomic()
            .set(&key, serialized_node)
            .sadd("node:ids", &address)
            .query_async(&mut con)
            .await?;

        Ok(())
    }

    pub async fn get_nodes(&self) -> Result<Vec<DiscoveryNode>, Error> {
        let mut con = self.get_connection().await?;
        let node_ids: Vec<String> = con.smembers("node:ids").await?;

        if node_ids.is_empty() {
            return Ok(Vec::new());
        }

        let node_keys: Vec<String> = node_ids.iter().map(|id| format!("node:{}", id)).collect();

        let mut pipe = redis::pipe();
        for key in &node_keys {
            pipe.get(key);
        }

        let serialized_nodes: Result<Vec<String>, redis::RedisError> =
            pipe.query_async(&mut con).await;

        let serialized_nodes = match serialized_nodes {
            Ok(nodes) => nodes,
            Err(e) => {
                error!("Error querying nodes from Redis: {}", e);
                return Err(e.into());
            }
        };

        let nodes_vec: Result<Vec<DiscoveryNode>, _> = serialized_nodes
            .into_iter()
            .map(|serialized_node| serde_json::from_str(&serialized_node))
            .collect();
        let mut nodes_vec = nodes_vec?;

        nodes_vec.sort_by(|a, b| {
            let a_time = a.last_updated.or(a.created_at);
            let b_time = b.last_updated.or(b.created_at);
            b_time.cmp(&a_time)
        });
        Ok(nodes_vec)
    }

    pub async fn get_node_by_id(&self, node_id: &str) -> Result<Option<DiscoveryNode>, Error> {
        let mut con = self.get_connection().await?;
        let key = format!("node:{}", node_id);

        let serialized_node: Option<String> = con.get(&key).await?;

        let serialized_node = match serialized_node {
            Some(node_str) => serde_json::from_str(&node_str),
            None => Ok(None),
        }?;

        Ok(serialized_node)
    }
}
