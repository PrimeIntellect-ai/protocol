use crate::store::core::RedisStore;
use crate::store::domains::heartbeat_store::HeartbeatStore;
use crate::store::domains::metrics_store::MetricsStore;
use crate::store::domains::node_store::NodeStore;
use crate::store::domains::task_store::TaskStore;
use std::sync::Arc;

pub struct StoreContext {
    pub node_store: Arc<NodeStore>,
    pub heartbeat_store: Arc<HeartbeatStore>,
    pub task_store: Arc<TaskStore>,
    pub metrics_store: Arc<MetricsStore>,
}

impl StoreContext {
    pub fn new(store: Arc<RedisStore>) -> Self {
        Self {
            node_store: Arc::new(NodeStore::new(store.clone())),
            heartbeat_store: Arc::new(HeartbeatStore::new(store.clone())),
            task_store: Arc::new(TaskStore::new(store.clone())),
            metrics_store: Arc::new(MetricsStore::new(store.clone())),
        }
    }
}
