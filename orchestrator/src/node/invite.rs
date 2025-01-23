use crate::store::context::StoreContext;
use crate::types::Node;
use crate::types::NodeStatus;
use alloy::primitives::utils::keccak256 as keccak;
use alloy::primitives::U256;
use alloy::signers::Signer;
use anyhow::Result;
use hex;
use serde_json::json;
use shared::security::request_signer::sign_request;
use shared::web3::wallet::Wallet;
use std::sync::Arc;
use tokio::time::{interval, Duration};

pub struct NodeInviter<'a> {
    wallet: &'a Wallet,
    pool_id: u32,
    domain_id: u32,
    host: &'a str,
    port: &'a u16,
    store_context: Arc<StoreContext>,
}

impl<'a> NodeInviter<'a> {
    pub fn new(
        wallet: &'a Wallet,
        pool_id: u32,
        domain_id: u32,
        host: &'a str,
        port: &'a u16,
        store_context: Arc<StoreContext>,
    ) -> Self {
        Self {
            wallet,
            pool_id,
            domain_id,
            host,
            port,
            store_context,
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
        let nodes = self.store_context.node_store.get_uninvited_nodes();

        // TODO: Acquire lock for invite
        for node in nodes {
            let node_to_update = node.clone();
            // TODO: Do this in tokio and outside of this big fn call
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
                        let node = node_to_update.clone();
                        println!("Updating node status to WaitingForHeartbeat");
                        self.store_context
                            .node_store
                            .update_node_status(&node.address, NodeStatus::WaitingForHeartbeat);
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
