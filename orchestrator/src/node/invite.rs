use crate::models::node::Node;
use crate::models::node::NodeStatus;
use crate::store::core::StoreContext;
use alloy::primitives::utils::keccak256 as keccak;
use alloy::primitives::U256;
use alloy::signers::Signer;
use anyhow::Result;
use hex;
use log::{debug, error, info, warn};
use shared::models::invite::InviteRequest;
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
            debug!("Running NodeInviter to process uninvited nodes...");
            if let Err(e) = self.process_uninvited_nodes().await {
                error!("Error processing uninvited nodes: {}", e);
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

    async fn _send_invite(&self, node: Node) -> Result<(), anyhow::Error> {
        let node_to_update = node.clone();
        let node_url = format!("http://{}:{}", node.ip_address, node.port);
        let invite_path = "/invite".to_string();
        let invite_url = format!("{}{}", node_url, invite_path);

        let invite_signature = self._generate_invite(node).await?;

        let payload = InviteRequest {
            invite: hex::encode(invite_signature),
            pool_id: self.pool_id,
            master_ip: self.host.to_string(),
            master_port: *self.port,
        };
        let payload_json = serde_json::to_value(&payload).unwrap();

        let message_signature = sign_request(&invite_path, self.wallet, Some(&payload_json))
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

        info!("Sending invite to node: {:?}", invite_url);

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
                    info!("Successfully invited node");
                    self.store_context
                        .node_store
                        .update_node_status(&node.address, NodeStatus::WaitingForHeartbeat);
                    Ok(())
                } else {
                    warn!("Received non-success status: {:?}", response.status());
                    Err(anyhow::anyhow!(
                        "Received non-success status: {:?}",
                        response.status()
                    ))
                }
            }
            Err(e) => {
                error!("Error sending invite to node: {:?}", e);
                Err(anyhow::anyhow!("Error sending invite to node: {:?}", e))
            }
        }
    }

    async fn process_uninvited_nodes(&self) -> Result<()> {
        let nodes = self.store_context.node_store.get_uninvited_nodes();
        for node in nodes {
            // TODO: Eventually and carefully move this to tokio
            self._send_invite(node).await?;
        }

        Ok(())
    }
}
