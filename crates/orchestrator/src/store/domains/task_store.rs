use crate::events::TaskObserver;
use crate::store::core::RedisStore;
use log::error;
use redis::Commands;
use shared::models::task::Task;
use std::sync::{Arc, Mutex};

const TASK_KEY_PREFIX: &str = "orchestrator:task:";
const TASK_LIST_KEY: &str = "orchestrator:tasks";

pub struct TaskStore {
    redis: Arc<RedisStore>,
    observers: Mutex<Vec<Arc<dyn TaskObserver>>>,
}

impl TaskStore {
    pub fn new(redis: Arc<RedisStore>) -> Self {
        Self {
            redis,
            observers: Mutex::new(vec![]),
        }
    }

    pub fn add_observer(&self, observer: Arc<dyn TaskObserver>) {
        if let Ok(mut observers) = self.observers.lock() {
            observers.push(observer);
        }
    }

    pub fn add_task(&self, task: Task) {
        let mut con = self.redis.client.get_connection().unwrap();

        // Store the task with its ID as key
        let task_key = format!("{}{}", TASK_KEY_PREFIX, task.id);
        let _: () = con.set(&task_key, task.clone()).unwrap();

        // Add task ID to list of all tasks
        let _: () = con.rpush(TASK_LIST_KEY, task.id.to_string()).unwrap();

        // Notify observers synchronously
        let observers = self.observers.lock().unwrap().clone();
        for observer in observers.iter() {
            if let Err(e) = observer.on_task_created(&task) {
                error!("Error notifying observer: {}", e);
            }
        }
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

    pub fn delete_task(&self, id: String) {
        let mut con = self.redis.client.get_connection().unwrap();

        let task = self.get_task(&id);

        // Delete task from individual storage
        let task_key = format!("{}{}", TASK_KEY_PREFIX, id);
        let _: () = con.del(&task_key).unwrap();

        // Remove task ID from list
        let _: () = con.lrem(TASK_LIST_KEY, 0, id).unwrap();

        // Notify observers synchronously
        let observers = self.observers.lock().unwrap().clone();
        for observer in observers.iter() {
            if let Err(e) = observer.on_task_deleted(task.clone()) {
                error!("Error notifying observer: {}", e);
            }
        }
    }

    pub fn get_task(&self, id: &str) -> Option<Task> {
        let mut con = self.redis.client.get_connection().unwrap();
        let task_key = format!("{}{}", TASK_KEY_PREFIX, id);
        let task: Option<Task> = con.get(&task_key).unwrap();
        task
    }
}
