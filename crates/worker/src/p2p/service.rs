use anyhow::Result;
use iroh::endpoint::Incoming;
use iroh::{Endpoint, SecretKey};
use log::{debug, error, info, warn};
use shared::models::challenge::calc_matrix;
use shared::p2p::{P2PMessage, P2PRequest, P2PResponse, PRIME_P2P_PROTOCOL};
use std::time::SystemTime;
use tokio_util::sync::CancellationToken;

pub struct P2PService {
    endpoint: Endpoint,
    node_id: String,
    listening_addrs: Vec<String>,
    cancellation_token: CancellationToken,
}

impl P2PService {
    /// Create a new P2P service with a unique worker identity
    pub async fn new(
        worker_p2p_seed: Option<u64>,
        cancellation_token: CancellationToken,
    ) -> Result<Self> {
        // Generate or derive the secret key for this worker
        let secret_key = if let Some(seed) = worker_p2p_seed {
            // Derive from seed for deterministic identity
            let mut seed_bytes = [0u8; 32];
            seed_bytes[..8].copy_from_slice(&seed.to_le_bytes());
            SecretKey::from_bytes(&seed_bytes)
        } else {
            let mut rng = rand_v8::thread_rng();
            SecretKey::generate(&mut rng)
        };

        let node_id = secret_key.public().to_string();
        info!("Starting P2P service with node ID: {}", node_id);

        // Create the endpoint
        let endpoint = Endpoint::builder()
            .secret_key(secret_key)
            .alpns(vec![PRIME_P2P_PROTOCOL.to_vec()])
            .bind()
            .await?;

        // Get listening addresses
        let node_addr = endpoint.node_addr().await?;
        let listening_addrs = node_addr
            .direct_addresses
            .iter()
            .map(|addr| addr.to_string())
            .collect::<Vec<_>>();

        info!("P2P service listening on: {:?}", listening_addrs);

        Ok(Self {
            endpoint,
            node_id,
            listening_addrs,
            cancellation_token,
        })
    }

    /// Get the P2P node ID
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Get the listening addresses
    pub fn listening_addresses(&self) -> &[String] {
        &self.listening_addrs
    }
    /// Start accepting incoming connections
    pub async fn start(&self) -> Result<()> {
        let endpoint = self.endpoint.clone();
        let cancellation_token = self.cancellation_token.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancellation_token.cancelled() => {
                        info!("P2P service shutting down");
                        break;
                    }
                    incoming = endpoint.accept() => {
                        if let Some(incoming) = incoming {
                            tokio::spawn(Self::handle_connection(incoming));
                        } else {
                            warn!("P2P endpoint closed");
                            break;
                        }
                    }
                }
            }
        });

        Ok(())
    }

    /// Handle an incoming connection
    async fn handle_connection(incoming: Incoming) {
        match incoming.await {
            Ok(connection) => {
                info!("Accepted connection");
                match connection.accept_bi().await {
                    Ok((send, recv)) => {
                        if let Err(e) = Self::handle_stream(send, recv).await {
                            error!("Error handling stream: {}", e);
                        }
                        // Wait a bit before closing to ensure client has processed response
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                        // Properly close the connection after handling the stream
                        connection.close(0u32.into(), b"stream complete");
                    }
                    Err(e) => {
                        error!("Failed to accept bi-stream: {}", e);
                        connection.close(1u32.into(), b"stream error");
                    }
                }
            }
            Err(e) => {
                // Only log as debug for protocol mismatches, which are expected
                if e.to_string()
                    .contains("peer doesn't support any known protocol")
                {
                    debug!("Connection attempt with unsupported protocol: {}", e);
                } else {
                    error!("Failed to accept connection: {}", e);
                }
            }
        }
    }

    /// Handle a bidirectional stream
    async fn handle_stream(
        mut send: iroh::endpoint::SendStream,
        mut recv: iroh::endpoint::RecvStream,
    ) -> Result<()> {
        let mut msg_len_bytes = [0u8; 4];
        recv.read_exact(&mut msg_len_bytes).await?;
        let msg_len = u32::from_be_bytes(msg_len_bytes) as usize;

        let mut msg_bytes = vec![0u8; msg_len];
        recv.read_exact(&mut msg_bytes).await?;

        let request: P2PRequest = serde_json::from_slice(&msg_bytes)?;
        debug!("Received P2P request: {:?}", request);
        // Handle the request
        let response = match request.message {
            P2PMessage::Ping { nonce, .. } => {
                info!("Received ping with nonce: {}", nonce);
                P2PResponse::new(
                    request.id,
                    P2PMessage::Pong {
                        timestamp: SystemTime::now(),
                        nonce,
                    },
                )
            }
            P2PMessage::HardwareChallenge { challenge, .. } => {
                info!("Received hardware challenge");
                let challenge_response = calc_matrix(&challenge);
                P2PResponse::new(
                    request.id,
                    P2PMessage::HardwareChallengeResponse {
                        response: challenge_response,
                        timestamp: SystemTime::now(),
                    },
                )
            }
            _ => {
                warn!("Unexpected message type");
                return Ok(());
            }
        };

        let response_bytes = serde_json::to_vec(&response)?;
        send.write_all(&(response_bytes.len() as u32).to_be_bytes())
            .await?;
        send.write_all(&response_bytes).await?;
        send.finish()?;

        // Wait for client to close
        match recv.read_to_end(1024).await {
            Ok(_) => debug!("Client closed their send stream gracefully"),
            Err(e) => debug!("Error waiting for client close: {}", e),
        }

        Ok(())
    }
}
