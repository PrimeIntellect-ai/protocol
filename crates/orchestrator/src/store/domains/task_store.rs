use crate::store::core::RedisStore;
use redis::Commands;
use shared::models::task::Task;
use std::sync::Arc;

const TASK_KEY_PREFIX: &str = "orchestrator:task:";
const TASK_LIST_KEY: &str = "orchestrator:tasks";
const CURRENT_TASK_KEY: &str = "orchestrator:task:current";

pub struct TaskStore {
    redis: Arc<RedisStore>,
}

impl TaskStore {
    pub fn new(redis: Arc<RedisStore>) -> Self {
        Self { redis }
    }

    pub fn get_current_task(&self) -> Option<Task> {
        let mut con = self.redis.client.get_connection().unwrap();
        let has_key: bool = con.exists(CURRENT_TASK_KEY).unwrap();
        if !has_key {
            return None;
        }
        let task: Task = con.get(CURRENT_TASK_KEY).unwrap();
        Some(task)
    }

    pub fn add_task(&self, task: Task) {
        let mut con = self.redis.client.get_connection().unwrap();

        // Store the task with its ID as key
        let task_key = format!("{}{}", TASK_KEY_PREFIX, task.id);
        let _: () = con.set(&task_key, task.clone()).unwrap();

        // Add task ID to list of all tasks
        let _: () = con.rpush(TASK_LIST_KEY, task.id.to_string()).unwrap();

        // Set as current task
        let _: () = con.set(CURRENT_TASK_KEY, task).unwrap();
    }

    pub fn get_all_tasks(&self) -> Vec<Task> {
        let mut con = self.redis.client.get_connection().unwrap();

        // Get all task IDs
        let task_ids: Vec<String> = con.lrange(TASK_LIST_KEY, 0, -1).unwrap();

        // Get each task by ID and collect into vector
        let mut tasks: Vec<Task> = task_ids
            .iter()
            .map(|id| {
                let task_key = format!("{}{}", TASK_KEY_PREFIX, id);
                con.get(&task_key).unwrap()
            })
            .collect();

        tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        tasks
    }

    pub fn delete_current_task(&self) {
        let mut con = self.redis.client.get_connection().unwrap();
        let _: () = con.del(CURRENT_TASK_KEY).unwrap();
    }
}
