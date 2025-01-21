use crate::store::redis::RedisStore;
use crate::types::Node;
use crate::types::NodeStatus;
use alloy::primitives::utils::keccak256 as keccak;
use alloy::primitives::U256;
use alloy::signers::Signer;
use anyhow::Result;
use hex;
use redis::Commands;
use serde_json::json;
use shared::security::request_signer::sign_request;
use shared::web3::wallet::Wallet;
use std::sync::Arc;
use tokio::time::{interval, Duration};
pub struct NodeInviter<'a> {
    store: Arc<RedisStore>,
    wallet: &'a Wallet,
    pool_id: u32,
    domain_id: u32,
    host: &'a str,
    port: &'a u16,
}

impl<'a> NodeInviter<'a> {
    pub fn new(
        store: Arc<RedisStore>,
        wallet: &'a Wallet,
        pool_id: u32,
        domain_id: u32,
        host: &'a str,
        port: &'a u16,
    ) -> Self {
        Self {
            store,
            wallet,
            pool_id,
            domain_id,
            host,
            port,
        }
    }

    pub async fn run(&self) -> Result<()> {
        let mut interval = interval(Duration::from_secs(10));

        loop {
            interval.tick().await;
            println!("Running NodeInviter to process uninvited nodes...");
            if let Err(e) = self.process_uninvited_nodes().await {
                eprintln!("Error processing uninvited nodes: {}", e);
            }
        }
    }

    async fn _generate_invite(&self, node: Node) -> Result<[u8; 65]> {
        let domain_id: [u8; 32] = U256::from(self.domain_id).to_be_bytes();
        let pool_id: [u8; 32] = U256::from(self.pool_id).to_be_bytes();

        let digest = keccak([&domain_id, &pool_id, node.address.as_slice()].concat());

        let signature = self
            .wallet
            .signer
            .sign_message(digest.as_slice())
            .await?
            .as_bytes()
            .to_owned();

        Ok(signature)
    }

    async fn process_uninvited_nodes(&self) -> Result<()> {
        //let nodes = self.store.get_uninvited_nodes().await?;
        let mut con = self.store.client.get_connection()?;
        let keys: Vec<String> = con.keys("orchestrator:node:*")?;
        let nodes: Vec<Node> = keys
            .iter()
            .filter_map(|key| con.get::<_, String>(key).ok())
            .filter_map(|node_json| serde_json::from_str::<Node>(&node_json).ok())
            .filter(|node| matches!(node.status, NodeStatus::Discovered))
            .collect();

        // TODO: Acquire lock for invite
        for node in nodes {
            let node_to_update = node.clone();
            // TODO: Do this in tokio and outside of this big fn call
            println!("Node: {:?}", node);
            let node_url = format!("http://{}:{}", node.ip_address, node.port);
            let invite_path = "/invite".to_string();
            let invite_url = format!("{}{}", node_url, invite_path);

            let invite_signature = self._generate_invite(node).await?;

            // TODO: Payload to send to node - must clean up with proper node information
            let payload = json!({
                "invite": hex::encode(invite_signature),
                "master_ip": self.host,
                "master_port": self.port,
                "pool_id": self.pool_id,
            });

            let message_signature = sign_request(&invite_path, self.wallet, Some(&payload))
                .await
                .unwrap();

            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                "x-address",
                self.wallet
                    .wallet
                    .default_signer()
                    .address()
                    .to_string()
                    .parse()
                    .unwrap(),
            );
            headers.insert("x-signature", message_signature.parse().unwrap());

            println!("Sending invite to node: {:?}", invite_url);

            match reqwest::Client::new()
                .post(invite_url)
                .json(&payload)
                .headers(headers)
                .send()
                .await
            {
                Ok(response) => {
                    if response.status().is_success() {
                        let response_body = response.text().await?;
                        let parsed_response: serde_json::Value =
                            serde_json::from_str(&response_body)?;
                        println!("Parsed Response: {:?}", parsed_response);

                        let mut updated_node = node_to_update.clone();
                        updated_node.status = NodeStatus::WaitingForHeartbeat;

                        // Create a mutable copy of the node to update its status
                        let redis_key =
                            format!("orchestrator:node:{}", &updated_node.address.to_string());
                        let json_payload = serde_json::to_string(&updated_node)?;
                        println!("Updating node status to WaitingForHeartbeat");
                        println!("Redis value: {:?}", json_payload);
                        let mut con = self.store.client.get_connection()?;
                        let _: () = con.set(&redis_key, json_payload)?;
                    } else {
                        println!("Received non-success status: {:?}", response.status());
                    }
                }
                Err(e) => {
                    println!("Error sending invite to node: {:?}", e);
                }
            }
        }
        Ok(())
    }
}
