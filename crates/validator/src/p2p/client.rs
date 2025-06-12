use anyhow::Result;
use iroh::{Endpoint, NodeAddr, NodeId, SecretKey};
use log::{debug, info};
use rand_v8::Rng;
use shared::p2p::{P2PMessage, P2PRequest, P2PResponse, PRIME_P2P_PROTOCOL};
use std::str::FromStr;
use std::time::SystemTime;

pub struct P2PClient {
    endpoint: Endpoint,
}

impl P2PClient {
    /// Create a new P2P client for the validator
    pub async fn new() -> Result<Self> {
        // Generate a new secret key for the validator
        let mut rng = rand_v8::thread_rng();
        let secret_key = SecretKey::generate(&mut rng);

        let endpoint = Endpoint::builder()
            .secret_key(secret_key)
            .alpns(vec![PRIME_P2P_PROTOCOL.to_vec()])
            .bind()
            .await?;

        Ok(Self { endpoint })
    }

    /// Send a ping to a worker and wait for pong response
    pub async fn ping_worker(
        &self,
        worker_p2p_id: &str,
        worker_addresses: &[String],
    ) -> Result<u64> {
        // Parse the worker node ID
        let node_id = NodeId::from_str(worker_p2p_id)?;

        // Parse addresses
        let mut socket_addrs = Vec::new();
        for addr in worker_addresses {
            if let Ok(socket_addr) = addr.parse() {
                socket_addrs.push(socket_addr);
            }
        }

        // Create node address
        let node_addr = NodeAddr::new(node_id).with_direct_addresses(socket_addrs);

        debug!("Connecting to worker P2P node: {}", worker_p2p_id);

        // Connect to the worker
        let connection = self.endpoint.connect(node_addr, PRIME_P2P_PROTOCOL).await?;

        // Open a bidirectional stream
        let (mut send, mut recv) = connection.open_bi().await?;

        let nonce = rand_v8::thread_rng().gen::<u64>();
        let ping_request = P2PRequest::new(P2PMessage::Ping {
            timestamp: SystemTime::now(),
            nonce,
        });

        // Serialize and send the request
        let request_bytes = serde_json::to_vec(&ping_request)?;
        send.write_all(&(request_bytes.len() as u32).to_be_bytes())
            .await?;
        send.write_all(&request_bytes).await?;

        send.finish()?;

        // Read the response
        let mut response_len_bytes = [0u8; 4];
        recv.read_exact(&mut response_len_bytes).await?;
        let response_len = u32::from_be_bytes(response_len_bytes) as usize;

        let mut response_bytes = vec![0u8; response_len];
        recv.read_exact(&mut response_bytes).await?;

        let response: P2PResponse = serde_json::from_slice(&response_bytes)?;

        // Verify the response
        let result = match response.message {
            P2PMessage::Pong {
                nonce: returned_nonce,
                ..
            } => {
                if returned_nonce == nonce {
                    info!(
                        "Received valid pong from worker {} with nonce: {}",
                        worker_p2p_id, nonce
                    );
                    Ok(nonce)
                } else {
                    Err(anyhow::anyhow!("Invalid nonce in pong response"))
                }
            }
            _ => Err(anyhow::anyhow!("Unexpected response type")),
        };

        connection.close(0u32.into(), b"ping complete");
        // Give the close frame time to be sent
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        result
    }

    /// Shutdown the P2P client
    pub async fn shutdown(self) -> Result<()> {
        self.endpoint.close().await;
        Ok(())
    }
}
