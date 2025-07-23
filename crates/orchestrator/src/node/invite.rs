use crate::models::node::NodeStatus;
use crate::models::node::OrchestratorNode;
use crate::p2p::InviteRequest as InviteRequestWithMetadata;
use crate::store::core::StoreContext;
use crate::utils::loop_heartbeats::LoopHeartbeats;
use anyhow::{bail, Result};
use futures::stream;
use futures::StreamExt;
use log::{debug, error, info, warn};
use operations::invite::{
    admin::{generate_invite_expiration, generate_invite_nonce, generate_invite_signature},
    common::InviteBuilder,
};
use p2p::InviteRequestUrl;
use shared::web3::wallet::Wallet;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;
use tokio::time::{interval, Duration};

// Timeout constants
const DEFAULT_INVITE_CONCURRENT_COUNT: usize = 32; // Max concurrent count of nodes being invited

pub struct NodeInviter {
    wallet: Wallet,
    pool_id: u32,
    domain_id: u32,
    url: InviteRequestUrl,
    store_context: Arc<StoreContext>,
    heartbeats: Arc<LoopHeartbeats>,
    invite_tx: Sender<InviteRequestWithMetadata>,
}

impl NodeInviter {
    #[allow(clippy::too_many_arguments)]
    pub fn new<'a>(
        wallet: Wallet,
        pool_id: u32,
        domain_id: u32,
        host: Option<&'a str>,
        port: Option<&'a u16>,
        url: Option<&'a str>,
        store_context: Arc<StoreContext>,
        heartbeats: Arc<LoopHeartbeats>,
        invite_tx: Sender<InviteRequestWithMetadata>,
    ) -> Result<Self> {
        let url = if let Some(url) = url {
            InviteRequestUrl::MasterUrl(url.to_string())
        } else {
            let Some(host) = host else {
                bail!("either host or url must be provided");
            };

            let Some(port) = port else {
                bail!("either port or url must be provided");
            };

            InviteRequestUrl::MasterIpPort(host.to_string(), *port)
        };

        Ok(Self {
            wallet,
            pool_id,
            domain_id,
            url,
            store_context,
            heartbeats,
            invite_tx,
        })
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

    async fn generate_invite(
        &self,
        node: &OrchestratorNode,
        nonce: [u8; 32],
        expiration: [u8; 32],
    ) -> Result<[u8; 65]> {
        generate_invite_signature(
            &self.wallet,
            self.domain_id,
            self.pool_id,
            node.address,
            nonce,
            expiration,
        )
        .await
    }

    async fn send_invite(&self, node: &OrchestratorNode) -> Result<(), anyhow::Error> {
        if node.worker_p2p_id.is_none() || node.worker_p2p_addresses.is_none() {
            return Err(anyhow::anyhow!("Node does not have p2p information"));
        }
        let p2p_id = node.worker_p2p_id.as_ref().unwrap();
        let p2p_addresses = node.worker_p2p_addresses.as_ref().unwrap();

        // Generate random nonce and expiration
        let nonce = generate_invite_nonce();
        let expiration = generate_invite_expiration(Some(1000))?;

        let invite_signature = self.generate_invite(node, nonce, expiration).await?;

        // Build the invite request using the builder
        let builder = match &self.url {
            InviteRequestUrl::MasterUrl(url) => InviteBuilder::with_url(self.pool_id, url.clone()),
            InviteRequestUrl::MasterIpPort(ip, port) => {
                InviteBuilder::with_ip_port(self.pool_id, ip.clone(), *port)
            }
        };

        let payload = builder.build(invite_signature, nonce, expiration)?;

        info!("Sending invite to node: {p2p_id}");

        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        let invite = InviteRequestWithMetadata {
            worker_wallet_address: node.address,
            worker_p2p_id: p2p_id.clone(),
            worker_addresses: p2p_addresses.clone(),
            invite: payload,
            response_tx,
        };
        self.invite_tx
            .send(invite)
            .await
            .map_err(|_| anyhow::anyhow!("failed to send invite request"))?;
        match response_rx.await {
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
            match self.send_invite(&node).await {
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
