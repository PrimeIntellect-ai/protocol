use crate::store::redis::RedisStore;
use crate::types::Node;
use anyhow::Result;
use redis::Commands;
use tokio::time::{interval, Duration};

pub struct NodeInviter {
    store: RedisStore,
}

impl NodeInviter {
    pub fn new(store: RedisStore) -> Self {
        Self { store }
    }

    pub async fn run(&self) -> Result<()> {
        let mut interval = interval(Duration::from_secs(30));

        loop {
            interval.tick().await;
            println!("Running NodeInviter to process uninvited nodes...");
            if let Err(e) = self.process_uninvited_nodes().await {
                eprintln!("Error processing uninvited nodes: {}", e);
            }
        }
    }

    async fn process_uninvited_nodes(&self) -> Result<()> {
        //let nodes = self.store.get_uninvited_nodes().await?;
        let mut con = self.store.client.get_connection()?;
        let keys: Vec<String> = con.keys("orchestrator:node:*")?;
        let nodes: Vec<Node> = keys
            .iter()
            .filter_map(|key| con.get::<_, String>(key).ok())
            .filter_map(|node_json| serde_json::from_str::<Node>(&node_json).ok())
            .collect();

        // TODO: Acquire lock for invite
        for node in nodes {
            println!("Node: {:?}", node);
            // Create and send invite logic here
            // TODO: Need port for invite
            // TODO: Need api for invite
            // invite + signature / call to invite to contract - my own message signature
            // has to respond with 200 to move state from invited to pending

            // TODO: Need to know if node is valid
        }
        Ok(())
    }
}
