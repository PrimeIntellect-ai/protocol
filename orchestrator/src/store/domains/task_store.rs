use crate::store::core::RedisStore;
use redis::Commands;
use shared::models::task::Task;
use std::sync::Arc;

const TASK_KEY: &str = "orchestrator:task:current";

pub struct TaskStore {
    redis: Arc<RedisStore>,
}

impl TaskStore {
    pub fn new(redis: Arc<RedisStore>) -> Self {
        Self { redis }
    }

    pub fn get_task(&self) -> Option<Task> {
        let mut con = self.redis.client.get_connection().unwrap();
        let has_key: bool = con.exists(TASK_KEY).unwrap();
        if !has_key {
            return None;
        }
        let task: Task = con.get(TASK_KEY).unwrap();
        Some(task)
    }

    pub fn set_task(&self, task: Task) {
        let mut con = self.redis.client.get_connection().unwrap();
        let _: () = con.set(TASK_KEY, task).unwrap();
    }

    pub fn delete_task(&self) {
        let mut con = self.redis.client.get_connection().unwrap();
        let _: () = con.del(TASK_KEY).unwrap();
    }
}
