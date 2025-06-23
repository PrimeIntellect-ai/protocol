use crate::events::TaskObserver;
use crate::store::core::RedisStore;
use anyhow::Result;
use futures::future;
use log::error;
use redis::AsyncCommands;
use shared::models::task::Task;
use std::sync::Arc;
use tokio::sync::Mutex;

const TASK_KEY_PREFIX: &str = "orchestrator:task:";
const TASK_LIST_KEY: &str = "orchestrator:tasks";
const TASK_NAME_INDEX_KEY: &str = "orchestrator:task_names";

pub struct TaskStore {
    redis: Arc<RedisStore>,
    observers: Arc<Mutex<Vec<Arc<dyn TaskObserver>>>>,
}

impl TaskStore {
    pub fn new(redis: Arc<RedisStore>) -> Self {
        Self {
            redis,
            observers: Arc::new(Mutex::new(vec![])),
        }
    }

    pub async fn add_observer(&self, observer: Arc<dyn TaskObserver>) {
        let mut observers = self.observers.lock().await;
        observers.push(observer);
    }

    pub async fn add_task(&self, task: Task) -> Result<()> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;

        // Store the task with its ID as key
        let task_key = format!("{}{}", TASK_KEY_PREFIX, task.id);
        let _: () = con.set(&task_key, task.clone()).await?;

        // Add task ID to list of all tasks
        let _: () = con.rpush(TASK_LIST_KEY, task.id.to_string()).await?;

        // Add task name to set for fast lookup
        let _: () = con.sadd(TASK_NAME_INDEX_KEY, &task.name).await?;

        // Notify observers synchronously
        let observers = self.observers.lock().await.clone();
        for observer in observers.iter() {
            if let Err(e) = observer.on_task_created(&task) {
                error!("Error notifying observer: {e}");
            }
        }

        Ok(())
    }

    pub async fn get_all_tasks(&self) -> Result<Vec<Task>> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;

        // Get all task IDs
        let task_ids: Vec<String> = con.lrange(TASK_LIST_KEY, 0, -1).await?;

        // Get each task by ID in parallel
        let task_futures: Vec<_> = task_ids
            .into_iter()
            .map(|id| {
                let task_key = format!("{TASK_KEY_PREFIX}{id}");
                let mut con = con.clone();
                async move {
                    let task: Option<Task> = con.get(&task_key).await.ok().flatten();
                    task
                }
            })
            .collect();

        let task_results = future::join_all(task_futures).await;
        let mut tasks: Vec<Task> = task_results.into_iter().flatten().collect();

        tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        Ok(tasks)
    }

    pub async fn delete_task(&self, id: String) -> Result<()> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;

        let task = self.get_task(&id).await?;

        // Delete task from individual storage
        let task_key = format!("{TASK_KEY_PREFIX}{id}");
        let _: () = con.del(&task_key).await?;

        // Remove task ID from list
        let _: () = con.lrem(TASK_LIST_KEY, 0, id).await?;

        // Remove task name from set if task exists
        if let Some(ref task) = task {
            let _: () = con.srem(TASK_NAME_INDEX_KEY, &task.name).await?;
        }

        // Notify observers synchronously
        let observers = self.observers.lock().await.clone();
        for observer in observers.iter() {
            if let Err(e) = observer.on_task_deleted(task.clone()) {
                error!("Error notifying observer: {e}");
            }
        }

        Ok(())
    }

    pub async fn delete_all_tasks(&self) -> Result<()> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;

        // Get all tasks first for observer notifications
        let tasks = self.get_all_tasks().await?;

        // Delete all individual task keys
        for task in &tasks {
            let task_key = format!("{}{}", TASK_KEY_PREFIX, task.id);
            let _: () = con.del(&task_key).await?;
        }

        // Clear the task list
        let _: () = con.del(TASK_LIST_KEY).await?;

        // Notify observers synchronously
        let observers = self.observers.lock().await.clone();
        for task in tasks {
            for observer in observers.iter() {
                if let Err(e) = observer.on_task_deleted(Some(task.clone())) {
                    error!("Error notifying observer: {e}");
                }
            }
        }

        Ok(())
    }

    pub async fn get_task(&self, id: &str) -> Result<Option<Task>> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;
        let task_key = format!("{TASK_KEY_PREFIX}{id}");
        let task: Option<Task> = con.get(&task_key).await?;
        Ok(task)
    }

    pub async fn task_name_exists(&self, name: &str) -> Result<bool> {
        let mut con = self.redis.client.get_multiplexed_async_connection().await?;
        let exists: bool = con.sismember(TASK_NAME_INDEX_KEY, name).await?;
        Ok(exists)
    }
}
