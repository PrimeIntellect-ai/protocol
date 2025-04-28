use crate::models::node::NodeStatus;
use crate::models::node::OrchestratorNode;
use crate::store::core::StoreContext;
use crate::utils::loop_heartbeats::LoopHeartbeats;
use alloy::primitives::utils::keccak256 as keccak;
use alloy::primitives::U256;
use alloy::signers::Signer;
use anyhow::Result;
use futures::stream;
use futures::StreamExt;
use hex;
use log::{debug, error, info, warn};
use reqwest::Client;
use shared::models::invite::InviteRequest;
use shared::security::request_signer::sign_request;
use shared::web3::wallet::Wallet;
use std::sync::Arc;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tokio::time::{interval, Duration};

// Timeout constants
const REQUEST_TIMEOUT: u64 = 15; // 15 seconds for HTTP requests
const CONNECTION_TIMEOUT: u64 = 10; // 10 seconds for establishing connections
const DEFAULT_INVITE_CONCURRENT_COUNT: usize = 32; // Max concurrent count of nodes being invited

pub struct NodeInviter<'a> {
    wallet: &'a Wallet,
    pool_id: u32,
    domain_id: u32,
    host: Option<&'a str>,
    port: Option<&'a u16>,
    url: Option<&'a str>,
    store_context: Arc<StoreContext>,
    client: Client,
    heartbeats: Arc<LoopHeartbeats>,
}

impl<'a> NodeInviter<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        wallet: &'a Wallet,
        pool_id: u32,
        domain_id: u32,
        host: Option<&'a str>,
        port: Option<&'a u16>,
        url: Option<&'a str>,
        store_context: Arc<StoreContext>,
        heartbeats: Arc<LoopHeartbeats>,
    ) -> Self {
        Self {
            wallet,
            pool_id,
            domain_id,
            host,
            port,
            url,
            store_context,
            heartbeats,
            client: Client::builder()
                .timeout(Duration::from_secs(REQUEST_TIMEOUT))
                .connect_timeout(Duration::from_secs(CONNECTION_TIMEOUT))
                .build()
                .unwrap(),
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
            self.heartbeats.update_inviter();
        }
    }

    async fn _generate_invite(&self, node: &OrchestratorNode) -> Result<[u8; 65]> {
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

    async fn _send_invite(&self, node: &OrchestratorNode) -> Result<(), anyhow::Error> {
        let node_url = format!("http://{}:{}", node.ip_address, node.port);
        let invite_path = "/invite".to_string();
        let invite_url = format!("{}{}", node_url, invite_path);

        let invite_signature = self._generate_invite(node).await?;
        let payload = InviteRequest {
            invite: hex::encode(invite_signature),
            pool_id: self.pool_id,
            master_url: self.url.map(|u| u.to_string()),
            master_ip: if self.url.is_none() {
                self.host.map(|h| h.to_string())
            } else {
                None
            },
            master_port: if self.url.is_none() {
                self.port.copied()
            } else {
                None
            },
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
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

        match self
            .client
            .post(invite_url)
            .json(&payload)
            .headers(headers)
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status();

                if status.is_success() {
                    info!("Successfully invited node");
                    self.store_context
                        .node_store
                        .update_node_status(&node.address, NodeStatus::WaitingForHeartbeat);
                    self.store_context
                        .heartbeat_store
                        .clear_unhealthy_counter(&node.address);
                    Ok(())
                } else {
                    let response_text = match response.text().await {
                        Ok(text) => text,
                        Err(e) => format!("Failed to get response text: {}", e),
                    };
                    warn!(
                        "Received non-success status: {:?}. Response: {}",
                        status, response_text
                    );
                    Err(anyhow::anyhow!(
                        "Received non-success status: {:?}. Response: {}",
                        status,
                        response_text
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

        let invited_nodes = stream::iter(nodes.into_iter().map(|node| async move {
            info!("Processing node {:?}", node.address);
            match self._send_invite(&node).await {
                Ok(_) => {
                    info!("Successfully processed node {:?}", node.address);
                    Ok(())
                }
                Err(e) => {
                    error!("Failed to process node {:?}: {}", node.address, e);
                    Err((node, e))
                }
            }
        }))
        .buffer_unordered(DEFAULT_INVITE_CONCURRENT_COUNT)
        .collect::<Vec<_>>()
        .await;

        let failed_nodes: Vec<_> = invited_nodes.into_iter().filter_map(Result::err).collect();
        if !failed_nodes.is_empty() {
            warn!(
                "Failed to process {} nodes: {:?}",
                failed_nodes.len(),
                failed_nodes
                    .iter()
                    .map(|(node, _)| node.address)
                    .collect::<Vec<_>>()
            );
        }

        Ok(())
    }
}
