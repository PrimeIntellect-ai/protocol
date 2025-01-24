use crate::store::redis::RedisStore;
use redis::Commands;
use shared::models::node::Node;

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
        let address = node.id.clone();
        let key = format!("node:{}", address);
        let serialized_node = serde_json::to_string(&node).unwrap();
        let _: () = con.set(&key, serialized_node).unwrap();
    }

    pub fn get_nodes(&self) -> Vec<Node> {
        let mut con = self.get_connection();
        let nodes: Vec<String> = con.keys("node:*").unwrap();
        let mut nodes_vec = Vec::new();
        for node in nodes {
            let serialized_node: String = con.get(node).unwrap();
            let deserialized_node: Node = serde_json::from_str(&serialized_node).unwrap();
            nodes_vec.push(deserialized_node);
        }
        nodes_vec
    }
}
