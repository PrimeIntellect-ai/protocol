use crate::models::node::Node;
use crate::store::redis::RedisStore;
use redis::Commands;
use serde::{Deserialize, Serialize};

pub struct NodeStore {
    redis_store: RedisStore,
}

impl NodeStore {
    pub fn new(redis_store: RedisStore) -> Self {
        Self { redis_store }
    }

    fn get_connection(&self) -> redis::Connection {
        self.redis_store.client.get_connection().unwrap()
    }

    pub fn register_node(&self, node: Node) {
        let mut con = self.get_connection();
    }

    pub fn get_nodes(&self) -> Vec<Node> {
        let con = self.get_connection();
        vec![]
    }
}
