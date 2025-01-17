use crate::store::redis::RedisStore;
use crate::types::Node;
use alloy::primitives::hex;
use alloy::signers::Signer;
use anyhow::Error;
use anyhow::Result;
use redis::Commands;
use serde_json;
use shared::web3::wallet::Wallet;
use std::time::Duration;
use tokio::time::interval;

pub struct DiscoveryMonitor {
    store: RedisStore,
    coordinator_wallet: Wallet,
    compute_pool_id: u32,
}

impl DiscoveryMonitor {
    pub fn new(store: RedisStore, coordinator_wallet: Wallet, compute_pool_id: &u32) -> Self {
        Self {
            store,
            coordinator_wallet,
            compute_pool_id: compute_pool_id.clone(),
        }
    }

    pub async fn run(&self) -> Result<()> {
        let mut interval = interval(Duration::from_secs(30));

        loop {
            interval.tick().await;
            println!("Running DiscoveryMonitor to get nodes...");
            if let Err(e) = self.get_nodes().await {
                println!("Error getting nodes: {}", e);
            }
        }
    }

    async fn _generate_signature(
        &self,
        message: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let signature = self
            .coordinator_wallet
            .signer
            .sign_message(message.as_bytes())
            .await?
            .as_bytes();
        Ok(format!("0x{}", hex::encode(signature)))
    }

    pub async fn fetch_nodes_from_discovery(&self) -> Result<Vec<Node>, Error> {
        let discovery_url = "http://localhost:8089";
        let discovery_route = format!("/api/nodes/pool/{}", self.compute_pool_id);
        let address = self
            .coordinator_wallet
            .wallet
            .default_signer()
            .address()
            .to_string();

        let signature = self._generate_signature(&discovery_route).await.unwrap();

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-address", address.parse().unwrap());
        headers.insert("x-signature", signature.parse().unwrap());

        println!("Fetching nodes from: {}{}", discovery_url, discovery_route);
        let response = reqwest::Client::new()
            .get(format!("{}{}", discovery_url, discovery_route))
            .headers(headers)
            .send()
            .await?;

        let response_text: String = response.text().await?;
        println!("Response: {:?}", response_text);
        let parsed_response: serde_json::Value = serde_json::from_str(&response_text)?;
        let nodes: Vec<Node> = serde_json::from_value(parsed_response["data"].clone())?;
        println!("Nodes: {:?}", nodes);
        Ok(nodes)
    }

    pub async fn get_nodes(&self) -> Result<Vec<Node>, Error> {
        let mut con = self.store.client.get_connection()?;
        let nodes = self.fetch_nodes_from_discovery().await?;
        for node in &nodes {
            println!("Node: {:?}", node.address);
            let key = format!("orchestrator:node:{}", node.address);
            let exists: Option<String> = con.get(&key)?;
            if exists.is_some() {
                println!("Node {} already exists in the database.", node.address);
                continue; // Skip processing this node
            } else {
                // TODO: Only temp - make nice
                con.set(&key, serde_json::to_string(node)?)?;
            }
        }
        Ok(nodes)
    }
}
