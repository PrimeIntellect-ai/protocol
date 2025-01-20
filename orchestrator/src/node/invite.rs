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
use shared::web3::wallet::Wallet;
use tokio::time::{interval, Duration};

pub struct NodeInviter<'a> {
    store: RedisStore,
    wallet: &'a Wallet,
    pool_id: u32,
    domain_id: u32,
}

impl<'a> NodeInviter<'a> {
    pub fn new(store: RedisStore, wallet: &'a Wallet, pool_id: u32, domain_id: u32) -> Self {
        Self {
            store,
            wallet,
            pool_id,
            domain_id,
        }
    }

    pub async fn run(&self) -> Result<()> {
        let mut interval = interval(Duration::from_secs(15));

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
            .try_into()
            .unwrap();

        Ok(signature)
    }

    async fn _generate_msg_signature(
        &self,
        message: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let signature = self
            .wallet
            .signer
            .sign_message(message.as_bytes())
            .await?
            .as_bytes();
        Ok(format!("0x{}", hex::encode(signature)))
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
                "master_ip": "0.0.0.0",
                "master_port": 8090,
                "pool_id": self.pool_id,
            });

            println!("Invite Payload: {:?}", payload);

            let mut payload_data = payload.clone();
            if let Some(obj) = payload_data.as_object_mut() {
                let sorted_keys: Vec<String> = obj.keys().cloned().collect();
                let sorted_obj: serde_json::Map<String, serde_json::Value> = sorted_keys
                    .into_iter()
                    .map(|key| (key.clone(), obj.remove(&key).unwrap()))
                    .collect();
                *obj = sorted_obj;
            }
            let payload_string = serde_json::to_string(&payload_data).unwrap();
            let message = format!("{}{}", invite_path, payload_string);
            let message_signature = self._generate_msg_signature(&message).await.unwrap();

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
                        con.set(&redis_key, json_payload)?;
                    } else {
                        println!("Received non-success status: {:?}", response.status());
                    }
                }
                Err(e) => {
                    println!("Error sending invite to node: {:?}", e);
                }
            }

            // Create and send invite logic here
            // TODO: Need port for invite
            // invite + signature / call to invite to contract - my own message signature
            // has to respond with 200 to move state from invited to pending
            // TODO: Need to know if node is valid
        }
        Ok(())
    }
}
