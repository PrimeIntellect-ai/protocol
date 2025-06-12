use anyhow::Result;
use iroh::{Endpoint, NodeAddr, NodeId, SecretKey};
use log::{debug, info};
use std::str::FromStr;
use std::time::Duration;

use crate::p2p::{P2PMessage, P2PRequest, P2PResponse, PRIME_P2P_PROTOCOL};

pub struct P2PClient {
    endpoint: Endpoint,
    node_id: NodeId,
}

impl P2PClient {
    pub async fn new() -> Result<Self> {
        let mut rng = rand_v8::thread_rng();
        let secret_key = SecretKey::generate(&mut rng);
        let node_id = secret_key.public();

        let endpoint = Endpoint::builder()
            .secret_key(secret_key)
            .alpns(vec![PRIME_P2P_PROTOCOL.to_vec()])
            .bind()
            .await?;

        info!("P2P client initialized with node ID: {}", node_id);

        Ok(Self { endpoint, node_id })
    }

    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    pub async fn send_request(
        &self,
        target_p2p_id: &str,
        target_addresses: &[String],
        message: P2PMessage,
        timeout_secs: u64,
    ) -> Result<P2PMessage> {
        let timeout_duration = Duration::from_secs(timeout_secs);

        tokio::time::timeout(timeout_duration, async {
            self.send_request_inner(target_p2p_id, target_addresses, message)
                .await
        })
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "P2P request to {} timed out after {}s",
                target_p2p_id,
                timeout_secs
            )
        })?
    }

    async fn send_request_inner(
        &self,
        target_p2p_id: &str,
        target_addresses: &[String],
        message: P2PMessage,
    ) -> Result<P2PMessage> {
        // Parse target node ID
        let node_id = NodeId::from_str(target_p2p_id)?;

        let mut socket_addrs = Vec::new();
        for addr in target_addresses {
            if let Ok(socket_addr) = addr.parse() {
                socket_addrs.push(socket_addr);
            }
        }

        if socket_addrs.is_empty() {
            return Err(anyhow::anyhow!(
                "No valid addresses provided for target node"
            ));
        }

        // Create node address
        let node_addr = NodeAddr::new(node_id).with_direct_addresses(socket_addrs);

        debug!("Connecting to P2P node: {}", target_p2p_id);

        // Connect to the target node
        let connection = self.endpoint.connect(node_addr, PRIME_P2P_PROTOCOL).await?;

        // Open bidirectional stream
        let (mut send, mut recv) = connection.open_bi().await?;

        // Create and serialize request
        let request = P2PRequest::new(message);
        let request_bytes = serde_json::to_vec(&request)?;

        // Send request with length prefix
        send.write_all(&(request_bytes.len() as u32).to_be_bytes())
            .await?;
        send.write_all(&request_bytes).await?;
        send.finish()?;

        // Read response length
        let mut response_len_bytes = [0u8; 4];
        recv.read_exact(&mut response_len_bytes).await?;
        let response_len = u32::from_be_bytes(response_len_bytes) as usize;

        // Read response data
        let mut response_bytes = vec![0u8; response_len];
        recv.read_exact(&mut response_bytes).await?;

        // Parse response
        let response: P2PResponse = serde_json::from_slice(&response_bytes)?;

        // Close connection
        connection.close(0u32.into(), b"request complete");
        tokio::time::sleep(Duration::from_millis(50)).await;

        Ok(response.message)
    }

    /// Shutdown the P2P client gracefully
    pub async fn shutdown(self) -> Result<()> {
        info!("Shutting down P2P client with node ID: {}", self.node_id);
        self.endpoint.close().await;
        Ok(())
    }
}

impl Drop for P2PClient {
    fn drop(&mut self) {
        debug!("P2P client dropped for node ID: {}", self.node_id);
    }
}
