use crate::store::redis::RedisStore;
use crate::types::Node;
use crate::types::NodeStatus;
use alloy::primitives::Address;
use anyhow::Error;
use anyhow::Result;
use redis::Commands;
use serde_json;
use shared::security::request_signer::sign_request;
use shared::web3::wallet::Wallet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;

pub struct DiscoveryMonitor<'b> {
    store: Arc<RedisStore>,
    coordinator_wallet: &'b Wallet,
    compute_pool_id: u32,
    interval_s: u64,
    discovery_url: String,
}

impl<'b> DiscoveryMonitor<'b> {
    pub fn new(
        store: Arc<RedisStore>,
        coordinator_wallet: &'b Wallet,
        compute_pool_id: u32,
        interval_s: u64,
        discovery_url: String,
    ) -> Self {
        Self {
            store,
            coordinator_wallet,
            compute_pool_id,
            interval_s,
            discovery_url,
        }
    }

    pub async fn run(&self) -> Result<()> {
        let mut interval = interval(Duration::from_secs(self.interval_s));

        loop {
            interval.tick().await;
            println!("Running DiscoveryMonitor to get nodes...");
            if let Err(e) = self.get_nodes().await {
                println!("Error getting nodes: {}", e);
            }
        }
    }

    pub async fn fetch_nodes_from_discovery(&self) -> Result<Vec<Node>, Error> {
        let discovery_route = format!("/api/nodes/pool/{}", self.compute_pool_id);
        let address = self.coordinator_wallet.address().to_string();

        let signature = sign_request(&discovery_route, self.coordinator_wallet, None)
            .await
            .unwrap();

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-address", address.parse().unwrap());
        headers.insert("x-signature", signature.parse().unwrap());

        let response = reqwest::Client::new()
            .get(format!("{}{}", self.discovery_url, discovery_route))
            .headers(headers)
            .send()
            .await?;

        let response_text: String = response.text().await?;
        let parsed_response: serde_json::Value = serde_json::from_str(&response_text)?;
        let nodes: Vec<Node> = parsed_response["data"]
            .as_array()
            .ok_or_else(|| Error::msg("Invalid data format"))?
            .iter()
            .filter(|node_json| node_json["isValidated"].as_bool() == Some(true))
            .map(|node_json| {
                let id = node_json["id"]
                    .as_str()
                    .ok_or_else(|| Error::msg("Missing id"))?;
                let ip_address = node_json["ipAddress"]
                    .as_str()
                    .ok_or_else(|| Error::msg("Missing ipAddress"))?;
                let port: u16 = node_json["port"]
                    .as_u64()
                    .ok_or_else(|| Error::msg("Missing port"))?
                    as u16;
                let address: Address =
                    Address::from_str(id).map_err(|_| Error::msg("Invalid address format"))?;

                Ok(Node {
                    address,
                    ip_address: ip_address.to_string(),
                    port,
                    status: NodeStatus::Discovered,
                })
            })
            .collect::<Result<Vec<Node>, anyhow::Error>>()?;
        println!("Nodes: {:?}", nodes);
        Ok(nodes)
    }

    pub async fn get_nodes(&self) -> Result<Vec<Node>, Error> {
        let mut con = self.store.client.get_connection()?;
        let nodes = self.fetch_nodes_from_discovery().await?;
        for node in &nodes {
            let exists: Option<String> = con.get(node.orchestrator_key())?;
            if exists.is_some() {
                continue;
            }

            let _: () = con.set(node.orchestrator_key(), node.to_string())?;
        }
        Ok(nodes)
    }
}
