use crate::models::node::NodeStatus;
use crate::models::node::OrchestratorNode;
use crate::store::core::RedisStore;
use alloy::primitives::Address;
use log::info;
use redis::Commands;
use redis::Value;
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

    pub fn get_nodes(&self) -> Vec<OrchestratorNode> {
        let mut con = self.redis.client.get_connection().unwrap();
        let keys: Vec<String> = con.keys(format!("{}:*", ORCHESTRATOR_BASE_KEY)).unwrap();
        let mut nodes: Vec<OrchestratorNode> = Vec::new();

        for node in keys {
            let node_string: String = con.get(node).unwrap();
            let node: OrchestratorNode = OrchestratorNode::from_string(&node_string);
            nodes.push(node);
        }
        nodes
    }

    pub fn add_node(&self, node: OrchestratorNode) {
        let mut con = self.redis.client.get_connection().unwrap();
        let _: () = con
            .set(
                format!("{}:{}", ORCHESTRATOR_BASE_KEY, node.address),
                node.to_string(),
            )
            .unwrap();
    }

    pub fn get_node(&self, address: &Address) -> Option<OrchestratorNode> {
        let mut con = self.redis.client.get_connection().unwrap();

        let node_string: Option<String> =
            match con.get(format!("{}:{}", ORCHESTRATOR_BASE_KEY, address)) {
                Ok(value) => value,
                Err(_) => return None,
            };

        node_string.map(|node_string| OrchestratorNode::from_string(&node_string))
    }

    pub fn get_uninvited_nodes(&self) -> Vec<OrchestratorNode> {
        let mut con = self.redis.client.get_connection().unwrap();
        let keys: Vec<String> = con.keys(format!("{}:*", ORCHESTRATOR_BASE_KEY)).unwrap();
        let nodes: Vec<OrchestratorNode> = keys
            .iter()
            .filter_map(|key| con.get::<_, String>(key).ok())
            .filter_map(|node_json| serde_json::from_str::<OrchestratorNode>(&node_json).ok())
            .filter(|node| matches!(node.status, NodeStatus::Discovered))
            .collect();
        nodes
    }

    pub fn update_node_status(&self, node_address: &Address, status: NodeStatus) {
        let mut con = self.redis.client.get_connection().unwrap();
        let node_key: String = format!("{}:{}", ORCHESTRATOR_BASE_KEY, node_address);
        let node_string: String = con.get(&node_key).unwrap();
        let mut node: OrchestratorNode = serde_json::from_str(&node_string).unwrap();
        node.status = status;
        let node_string = node.to_string();
        let _: () = con.set(&node_key, node_string).unwrap();
    }

    pub fn update_node_task(
        &self,
        node_address: Address,
        current_task: Option<String>,
        task_state: Option<String>,
    ) {
        let mut con = self.redis.client.get_connection().unwrap();

        let node_value: Value = con
            .get(format!("{}:{}", ORCHESTRATOR_BASE_KEY, node_address))
            .unwrap();
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
                let details = (current_task, task_state);
                match details {
                    (Some(task), Some(task_state)) => {
                        node.task_state = Some(task_state);
                        node.task_id = Some(task);
                    }
                    _ => {
                        node.task_state = None;
                        node.task_id = None;
                    }
                }
                let node_string = node.to_string();
                let node_key = format!("{}:{}", ORCHESTRATOR_BASE_KEY, node_address);
                let _: () = con.set(&node_key, node_string).unwrap();
            }
            _ => {
                info!("Cannot update node");
            }
        }
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
            task_id: None,
            task_state: None,
        };

        let healthy_node = OrchestratorNode {
            address: Address::from_str("0x0000000000000000000000000000000000000002").unwrap(),
            ip_address: "192.168.1.2".to_string(),
            port: 8081,
            status: NodeStatus::Healthy,
            task_id: None,
            task_state: None,
        };

        node_store.add_node(uninvited_node.clone());
        node_store.add_node(healthy_node.clone());

        let uninvited_nodes = node_store.get_uninvited_nodes();
        assert_eq!(uninvited_nodes.len(), 1);
        assert_eq!(uninvited_nodes[0].address, uninvited_node.address);
    }
}
