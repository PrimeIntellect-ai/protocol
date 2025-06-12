use crate::console::Console;
use crate::docker::DockerService;
use crate::operations::heartbeat::service::HeartbeatService;
use crate::state::system_state::SystemState;
use alloy::primitives::{FixedBytes, U256};
use anyhow::Result;
use iroh::endpoint::Incoming;
use iroh::{Endpoint, SecretKey};
use log::{debug, error, info, warn};
use shared::models::challenge::calc_matrix;
use shared::models::invite::InviteRequest;
use shared::p2p::{P2PMessage, P2PRequest, P2PResponse, PRIME_P2P_PROTOCOL};
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::contracts::helpers::utils::retry_call;
use shared::web3::contracts::structs::compute_pool::PoolStatus;
use shared::web3::wallet::{Wallet, WalletProvider};
use std::sync::Arc;
use std::time::SystemTime;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct P2PContext {
    pub docker_service: Arc<DockerService>,
    pub heartbeat_service: Arc<HeartbeatService>,
    pub system_state: Arc<SystemState>,
    pub contracts: Contracts<WalletProvider>,
    pub node_wallet: Wallet,
    pub provider_wallet: Wallet,
}

pub struct P2PService {
    endpoint: Endpoint,
    node_id: String,
    listening_addrs: Vec<String>,
    cancellation_token: CancellationToken,
    context: P2PContext,
}

impl P2PService {
    /// Create a new P2P service with a unique worker identity
    pub async fn new(
        worker_p2p_seed: Option<u64>,
        cancellation_token: CancellationToken,
        context: P2PContext,
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
            context,
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
        let context = self.context.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancellation_token.cancelled() => {
                        info!("P2P service shutting down");
                        break;
                    }
                    incoming = endpoint.accept() => {
                        if let Some(incoming) = incoming {
                            tokio::spawn(Self::handle_connection(incoming, context.clone()));
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
    async fn handle_connection(incoming: Incoming, context: P2PContext) {
        match incoming.await {
            Ok(connection) => {
                info!("Accepted connection");
                match connection.accept_bi().await {
                    Ok((send, recv)) => {
                        if let Err(e) = Self::handle_stream(send, recv, context).await {
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
        context: P2PContext,
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
            P2PMessage::Invite(invite) => {
                let (status, error) = Self::handle_invite(invite, &context).await;
                P2PResponse::new(request.id, P2PMessage::InviteResponse { status, error })
            }
            P2PMessage::GetTaskLogs => {
                let logs = context.docker_service.get_logs().await;
                let response_logs = logs
                    .map(|log_string| vec![log_string])
                    .map_err(|e| e.to_string());
                P2PResponse::new(
                    request.id,
                    P2PMessage::GetTaskLogsResponse {
                        logs: response_logs,
                    },
                )
            }
            P2PMessage::RestartTask => {
                let result = context.docker_service.restart_task().await;
                let response_result = result.map_err(|e| e.to_string());
                P2PResponse::new(
                    request.id,
                    P2PMessage::RestartTaskResponse {
                        result: response_result,
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

    async fn handle_invite(
        invite: InviteRequest,
        context: &P2PContext,
    ) -> (String, Option<String>) {
        if context.system_state.is_running().await {
            return (
                "error".to_string(),
                Some("Heartbeat is currently running and in a compute pool".to_string()),
            );
        }
        if let Some(pool_id) = context.system_state.compute_pool_id.clone() {
            if invite.pool_id.to_string() != pool_id {
                return ("error".to_string(), Some("Invalid pool ID".to_string()));
            }
        }

        let invite_bytes = match hex::decode(&invite.invite) {
            Ok(bytes) => bytes,
            Err(err) => {
                error!("Failed to decode invite hex string: {:?}", err);
                return (
                    "error".to_string(),
                    Some("Invalid invite format".to_string()),
                );
            }
        };

        if invite_bytes.len() < 65 {
            return (
                "error".to_string(),
                Some("Invite data is too short".to_string()),
            );
        }

        let contracts = &context.contracts;
        let wallet = &context.node_wallet;
        let pool_id = U256::from(invite.pool_id);

        let bytes_array: [u8; 65] = match invite_bytes[..65].try_into() {
            Ok(array) => array,
            Err(_) => {
                error!("Failed to convert invite bytes to fixed-size array");
                return (
                    "error".to_string(),
                    Some("Invalid invite signature format".to_string()),
                );
            }
        };

        let provider_address = context.provider_wallet.wallet.default_signer().address();

        let pool_info = match contracts.compute_pool.get_pool_info(pool_id).await {
            Ok(info) => info,
            Err(err) => {
                error!("Failed to get pool info: {:?}", err);
                return (
                    "error".to_string(),
                    Some("Failed to get pool information".to_string()),
                );
            }
        };

        if let PoolStatus::PENDING = pool_info.status {
            Console::user_error("Pool is pending - Invite is invalid");
            return (
                "error".to_string(),
                Some("Pool is pending - Invite is invalid".to_string()),
            );
        }

        let node_address = vec![wallet.wallet.default_signer().address()];
        let signatures = vec![FixedBytes::from(&bytes_array)];
        let nonces = vec![invite.nonce];
        let expirations = vec![invite.expiration];
        let call = match contracts.compute_pool.build_join_compute_pool_call(
            pool_id,
            provider_address,
            node_address,
            nonces,
            expirations,
            signatures,
        ) {
            Ok(call) => call,
            Err(err) => {
                error!("Failed to build join compute pool call: {:?}", err);
                return (
                    "error".to_string(),
                    Some("Failed to build join compute pool call".to_string()),
                );
            }
        };

        let provider = &context.provider_wallet.provider;
        match retry_call(call, 3, provider.clone(), None).await {
            Ok(result) => {
                Console::success(&format!("Successfully joined compute pool: {}", result));
            }
            Err(err) => {
                error!("Failed to join compute pool: {:?}", err);
                return (
                    "error".to_string(),
                    Some(format!("Failed to join compute pool: {}", err)),
                );
            }
        }
        let endpoint = if let Some(url) = &invite.master_url {
            format!("{}/heartbeat", url)
        } else {
            match (&invite.master_ip, &invite.master_port) {
                (Some(ip), Some(port)) => format!("http://{}:{}/heartbeat", ip, port),
                _ => {
                    error!("Missing master IP or port in invite request");
                    return (
                        "error".to_string(),
                        Some("Missing master IP or port".to_string()),
                    );
                }
            }
        };

        if let Err(err) = context.heartbeat_service.start(endpoint).await {
            error!("Failed to start heartbeat service: {:?}", err);
            return (
                "error".to_string(),
                Some("Failed to start heartbeat service".to_string()),
            );
        }

        ("ok".to_string(), None)
    }
}
