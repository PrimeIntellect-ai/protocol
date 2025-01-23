use crate::store::heartbeat_store::HeartbeatStore;
use crate::store::node_store::NodeStore;
use crate::store::redis::RedisStore;
use crate::store::task_store::TaskStore;
use std::sync::Arc;

pub struct StoreContext {
    pub node_store: Arc<NodeStore>,
    pub heartbeat_store: Arc<HeartbeatStore>,
    pub task_store: Arc<TaskStore>,
}

impl StoreContext {
    pub fn new(store: Arc<RedisStore>) -> Self {
        Self {
            node_store: Arc::new(NodeStore::new(store.clone())),
            heartbeat_store: Arc::new(HeartbeatStore::new(store.clone())),
            task_store: Arc::new(TaskStore::new(store.clone())),
        }
    }
}
