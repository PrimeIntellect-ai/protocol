use crate::models::node::NodeStatus;
use crate::models::node::OrchestratorNode;
use crate::p2p::client::P2PClient;
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
use shared::models::invite::InviteRequest;
use shared::web3::wallet::Wallet;
use std::sync::Arc;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tokio::time::{interval, Duration};

// Timeout constants
const DEFAULT_INVITE_CONCURRENT_COUNT: usize = 32; // Max concurrent count of nodes being invited

pub struct NodeInviter<'a> {
    wallet: Wallet,
    pool_id: u32,
    domain_id: u32,
    host: Option<&'a str>,
    port: Option<&'a u16>,
    url: Option<&'a str>,
    store_context: Arc<StoreContext>,
    heartbeats: Arc<LoopHeartbeats>,
    p2p_client: Arc<P2PClient>,
}

impl<'a> NodeInviter<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        wallet: Wallet,
        pool_id: u32,
        domain_id: u32,
        host: Option<&'a str>,
        port: Option<&'a u16>,
        url: Option<&'a str>,
        store_context: Arc<StoreContext>,
        heartbeats: Arc<LoopHeartbeats>,
        p2p_client: Arc<P2PClient>,
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
            p2p_client,
        }
    }

    pub async fn run(&self) -> Result<()> {
        let mut interval = interval(Duration::from_secs(10));

        loop {
            interval.tick().await;
            debug!("Running NodeInviter to process uninvited nodes...");
            if let Err(e) = self.process_uninvited_nodes().await {
                error!("Error processing uninvited nodes: {e}");
            }
            self.heartbeats.update_inviter();
        }
    }

    async fn _generate_invite(
        &self,
        node: &OrchestratorNode,
        nonce: [u8; 32],
        expiration: [u8; 32],
    ) -> Result<[u8; 65]> {
        let domain_id: [u8; 32] = U256::from(self.domain_id).to_be_bytes();
        let pool_id: [u8; 32] = U256::from(self.pool_id).to_be_bytes();

        let digest = keccak(
            [
                &domain_id,
                &pool_id,
                node.address.as_slice(),
                &nonce,
                &expiration,
            ]
            .concat(),
        );

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
        if node.worker_p2p_id.is_none() || node.worker_p2p_addresses.is_none() {
            return Err(anyhow::anyhow!("Node does not have p2p information"));
        }
        let p2p_id = node.worker_p2p_id.as_ref().unwrap();
        let p2p_addresses = node.worker_p2p_addresses.as_ref().unwrap();

        // Generate random nonce and expiration
        let nonce: [u8; 32] = rand::random();
        let expiration: [u8; 32] = U256::from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| anyhow::anyhow!("System time error: {}", e))?
                .as_secs()
                + 1000,
        )
        .to_be_bytes();

        let invite_signature = self._generate_invite(node, nonce, expiration).await?;
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
                .map_err(|e| anyhow::anyhow!("System time error: {}", e))?
                .as_secs(),
            expiration,
            nonce,
        };

        info!("Sending invite to node: {p2p_id}");

        match self
            .p2p_client
            .invite_worker(node.address, p2p_id, p2p_addresses, payload)
            .await
        {
            Ok(_) => {
                info!("Successfully invited node");
                if let Err(e) = self
                    .store_context
                    .node_store
                    .update_node_status(&node.address, NodeStatus::WaitingForHeartbeat)
                    .await
                {
                    error!("Error updating node status: {e}");
                }
                if let Err(e) = self
                    .store_context
                    .heartbeat_store
                    .clear_unhealthy_counter(&node.address)
                    .await
                {
                    error!("Error clearing unhealthy counter: {e}");
                }
                Ok(())
            }
            Err(e) => {
                error!("Error sending invite to node: {e:?}");
                Err(anyhow::anyhow!("Error sending invite to node: {:?}", e))
            }
        }
    }

    async fn process_uninvited_nodes(&self) -> Result<()> {
        let nodes = self.store_context.node_store.get_uninvited_nodes().await?;

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
