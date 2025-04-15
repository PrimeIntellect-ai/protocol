use crate::store::redis::RedisStore;
use anyhow::Error;
use redis::Commands;
use shared::models::node::{DiscoveryNode, Node};

pub struct NodeStore {
    redis_store: RedisStore,
}

impl NodeStore {
    pub fn new(redis_store: RedisStore) -> Self {
        Self { redis_store }
    }

    fn get_connection(&self) -> Result<redis::Connection, redis::RedisError> {
        self.redis_store.client.get_connection()
    }

    pub fn get_node(&self, address: String) -> Result<Option<DiscoveryNode>, Error> {
        let key = format!("node:{}", address);
        let mut con = self.get_connection()?;
        let node: Option<String> = con.get(&key)?;
        let node = match node {
            Some(node) => serde_json::from_str(&node),
            None => Ok(None),
        }?;
        Ok(node)
    }

    pub fn get_active_node_by_ip(&self, ip: String) -> Result<Option<DiscoveryNode>, Error> {
        let mut con = self.get_connection()?;
        let nodes: Vec<String> = con.keys("node:*")?;
        for node in nodes {
            let serialized_node: String = con.get(node)?;
            let deserialized_node: DiscoveryNode = serde_json::from_str(&serialized_node)?;
            if deserialized_node.ip_address == ip && deserialized_node.is_active {
                return Ok(Some(deserialized_node));
            }
        }
        Ok(None)
    }

    pub fn register_node(&self, node: Node) -> Result<(), Error> {
        let address = node.id.clone();
        let key = format!("node:{}", address);

        let mut con = self.get_connection()?;

        if con.exists(&key)? {
            let existing_node = self.get_node(address)?;
            if let Some(existing_node) = existing_node {
                let updated_node = existing_node.with_updated_node(node);
                self.update_node(updated_node)?;
            }
        } else {
            let discovery_node = DiscoveryNode::from(node);
            let serialized_node = serde_json::to_string(&discovery_node)?;
            let _: () = con.set(&key, serialized_node)?;
        }
        Ok(())
    }

    pub fn update_node(&self, node: DiscoveryNode) -> Result<(), Error> {
        let mut con = self.get_connection()?;
        let address = node.id.clone();
        let key = format!("node:{}", address);
        let serialized_node = serde_json::to_string(&node)?;
        let _: () = con.set(&key, serialized_node)?;
        Ok(())
    }

    pub fn get_nodes(&self) -> Result<Vec<DiscoveryNode>, Error> {
        let mut con = self.get_connection()?;
        let nodes: Vec<String> = con.keys("node:*")?;
        let mut nodes_vec = Vec::new();
        for node in nodes {
            let serialized_node: String = con.get(node)?;
            let deserialized_node: DiscoveryNode = serde_json::from_str(&serialized_node)?;
            nodes_vec.push(deserialized_node);
        }
        nodes_vec.sort_by(|a, b| {
            let a_time = a.last_updated.or(a.created_at);
            let b_time = b.last_updated.or(b.created_at);
            b_time.cmp(&a_time)
        });
        Ok(nodes_vec)
    }

    pub fn get_node_by_id(&self, node_id: &str) -> Result<Option<DiscoveryNode>, Error> {
        let mut con = self.get_connection()?;
        let key = format!("node:{}", node_id);

        let serialized_node: Option<String> = con.get(&key)?;

        let serialized_node = match serialized_node {
            Some(node_str) => serde_json::from_str(&node_str),
            None => Ok(None),
        }?;

        Ok(serialized_node)
    }
}
