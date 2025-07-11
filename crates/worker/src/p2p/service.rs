use crate::console::Console;
use crate::docker::DockerService;
use crate::operations::heartbeat::service::HeartbeatService;
use crate::state::system_state::SystemState;
use alloy::primitives::{Address, FixedBytes, U256};
use anyhow::Result;
use dashmap::DashMap;
use iroh::endpoint::Incoming;
use iroh::{Endpoint, RelayMode, SecretKey};
use lazy_static::lazy_static;
use log::{debug, error, info, warn};
use rand_v8::Rng;
use shared::models::challenge::calc_matrix;
use shared::models::invite::InviteRequest;
use shared::p2p::messages::MAX_MESSAGE_SIZE;
use shared::p2p::messages::{P2PMessage, P2PRequest, P2PResponse};
use shared::p2p::protocol::PRIME_P2P_PROTOCOL;
use shared::security::request_signer::sign_message;
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::contracts::helpers::utils::retry_call;
use shared::web3::contracts::structs::compute_pool::PoolStatus;
use shared::web3::wallet::{Wallet, WalletProvider};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio_util::sync::CancellationToken;

lazy_static! {
    static ref NONCE_CACHE: DashMap<String, SystemTime> = DashMap::new();
}

#[derive(Clone)]
pub(crate) struct P2PContext {
    pub docker_service: Arc<DockerService>,
    pub heartbeat_service: Arc<HeartbeatService>,
    pub system_state: Arc<SystemState>,
    pub contracts: Contracts<WalletProvider>,
    pub node_wallet: Wallet,
    pub provider_wallet: Wallet,
}

#[derive(Clone)]
pub(crate) struct P2PService {
    endpoint: Endpoint,
    secret_key: SecretKey,
    node_id: String,
    listening_addrs: Vec<String>,
    cancellation_token: CancellationToken,
    context: Option<P2PContext>,
    allowed_addresses: Vec<Address>,
    wallet: Wallet,
}

enum EndpointLoopResult {
    Shutdown,
    EndpointClosed,
}

impl P2PService {
    /// Create a new P2P service with a unique worker identity
    pub(crate) async fn new(
        worker_p2p_seed: Option<u64>,
        cancellation_token: CancellationToken,
        context: Option<P2PContext>,
        wallet: Wallet,
        allowed_addresses: Vec<Address>,
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
        info!("Starting P2P service with node ID: {node_id}");

        // Create the endpoint
        let endpoint = Endpoint::builder()
            .secret_key(secret_key.clone())
            .alpns(vec![PRIME_P2P_PROTOCOL.to_vec()])
            .discovery_n0()
            .relay_mode(RelayMode::Default)
            .bind()
            .await?;

        // Get listening addresses
        let node_addr = endpoint.node_addr().await?;
        let listening_addrs = node_addr
            .direct_addresses
            .iter()
            .map(|addr| addr.to_string())
            .collect::<Vec<_>>();

        info!("P2P service listening on: {listening_addrs:?}");

        Ok(Self {
            endpoint,
            secret_key,
            node_id,
            listening_addrs,
            cancellation_token,
            context,
            allowed_addresses,
            wallet,
        })
    }

    /// Get the P2P node ID
    pub(crate) fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Get the listening addresses
    pub(crate) fn listening_addresses(&self) -> &[String] {
        &self.listening_addrs
    }

    /// Recreate the endpoint with the same identity
    async fn recreate_endpoint(&self) -> Result<Endpoint> {
        info!("Recreating P2P endpoint with node ID: {}", self.node_id);

        let endpoint = Endpoint::builder()
            .secret_key(self.secret_key.clone())
            .alpns(vec![PRIME_P2P_PROTOCOL.to_vec()])
            .discovery_n0()
            .relay_mode(RelayMode::Default)
            .bind()
            .await?;

        let node_addr = endpoint.node_addr().await?;
        let listening_addrs = node_addr
            .direct_addresses
            .iter()
            .map(|addr| addr.to_string())
            .collect::<Vec<_>>();

        info!("P2P endpoint recreated, listening on: {listening_addrs:?}");
        Ok(endpoint)
    }
    /// Start accepting incoming connections with automatic recovery
    pub(crate) fn start(&self) -> Result<()> {
        let service = Arc::new(self.clone());
        let cancellation_token = self.cancellation_token.clone();

        tokio::spawn(async move {
            service.run_with_recovery(cancellation_token).await;
        });

        Ok(())
    }

    /// Run the P2P service with automatic endpoint recovery
    async fn run_with_recovery(&self, cancellation_token: CancellationToken) {
        let mut endpoint = self.endpoint.clone();
        let mut retry_delay = Duration::from_secs(1);
        const MAX_RETRY_DELAY: Duration = Duration::from_secs(60);

        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => {
                    info!("P2P service shutting down");
                    break;
                }
                result = self.run_endpoint_loop(&endpoint, &cancellation_token) => {
                    match result {
                        EndpointLoopResult::Shutdown => break,
                        EndpointLoopResult::EndpointClosed => {
                            warn!("P2P endpoint closed, attempting recovery in {retry_delay:?}");

                            tokio::select! {
                                _ = cancellation_token.cancelled() => break,
                                _ = tokio::time::sleep(retry_delay) => {}
                            }

                            match self.recreate_endpoint().await {
                                Ok(new_endpoint) => {
                                    info!("P2P endpoint successfully recovered");
                                    endpoint = new_endpoint;
                                    retry_delay = Duration::from_secs(1);
                                }
                                Err(e) => {
                                    error!("Failed to recreate P2P endpoint: {e}");
                                    retry_delay = std::cmp::min(retry_delay * 2, MAX_RETRY_DELAY);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Run the main endpoint acceptance loop
    async fn run_endpoint_loop(
        &self,
        endpoint: &Endpoint,
        cancellation_token: &CancellationToken,
    ) -> EndpointLoopResult {
        let context = self.context.clone();
        let allowed_addresses = self.allowed_addresses.clone();
        let wallet = self.wallet.clone();

        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => {
                    return EndpointLoopResult::Shutdown;
                }
                incoming = endpoint.accept() => {
                    if let Some(incoming) = incoming {
                        tokio::spawn(Self::handle_connection(incoming, context.clone(), allowed_addresses.clone(), wallet.clone()));
                    } else {
                        return EndpointLoopResult::EndpointClosed;
                    }
                }
            }
        }
    }

    /// Handle an incoming connection
    async fn handle_connection(
        incoming: Incoming,
        context: Option<P2PContext>,
        allowed_addresses: Vec<Address>,
        wallet: Wallet,
    ) {
        match incoming.await {
            Ok(connection) => {
                match connection.accept_bi().await {
                    Ok((send, recv)) => {
                        if let Err(e) =
                            Self::handle_stream(send, recv, context, allowed_addresses, wallet)
                                .await
                        {
                            error!("Error handling stream: {e}");
                        }
                        // Wait a bit before closing to ensure client has processed response
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    }
                    Err(e) => {
                        error!("Failed to accept bi-stream: {e}");
                        connection.close(1u32.into(), b"stream error");
                    }
                }
            }
            Err(e) => {
                // Only log as debug for protocol mismatches, which are expected
                if e.to_string()
                    .contains("peer doesn't support any known protocol")
                {
                    debug!("Connection attempt with unsupported protocol: {e}");
                } else {
                    error!("Failed to accept connection: {e}");
                }
            }
        }
    }

    /// Read a message from the stream
    async fn read_message(recv: &mut iroh::endpoint::RecvStream) -> Result<P2PRequest> {
        // Read message length
        let mut msg_len_bytes = [0u8; 4];
        match recv.read_exact(&mut msg_len_bytes).await {
            Ok(_) => {}
            Err(e) => {
                debug!("Stream read ended: {e}");
                return Err(anyhow::anyhow!("Stream closed"));
            }
        }
        let msg_len = u32::from_be_bytes(msg_len_bytes) as usize;

        // Enforce maximum message size
        if msg_len > MAX_MESSAGE_SIZE {
            error!("Message size {msg_len} exceeds maximum allowed size {MAX_MESSAGE_SIZE}");
            return Err(anyhow::anyhow!("Message too large"));
        }

        let mut msg_bytes = vec![0u8; msg_len];
        recv.read_exact(&mut msg_bytes).await?;

        let request: P2PRequest = serde_json::from_slice(&msg_bytes)
            .map_err(|e| anyhow::anyhow!("Failed to deserialize P2P request: {}", e))?;

        debug!("Received P2P request: {request:?}");
        Ok(request)
    }

    async fn write_response(
        send: &mut iroh::endpoint::SendStream,
        response: P2PResponse,
    ) -> Result<()> {
        let response_bytes = serde_json::to_vec(&response)?;

        // Check response size before sending
        if response_bytes.len() > MAX_MESSAGE_SIZE {
            error!(
                "Response size {} exceeds maximum allowed size {}",
                response_bytes.len(),
                MAX_MESSAGE_SIZE
            );
            return Err(anyhow::anyhow!("Response too large"));
        }

        send.write_all(&(response_bytes.len() as u32).to_be_bytes())
            .await?;
        send.write_all(&response_bytes).await?;
        Ok(())
    }

    /// Handle a bidirectional stream
    async fn handle_stream(
        mut send: iroh::endpoint::SendStream,
        mut recv: iroh::endpoint::RecvStream,
        context: Option<P2PContext>,
        allowed_addresses: Vec<Address>,
        wallet: Wallet,
    ) -> Result<()> {
        // Handle multiple messages in sequence
        let mut is_authorized = false;
        let mut current_challenge: Option<String> = None;

        loop {
            let Ok(request) = Self::read_message(&mut recv).await else {
                break;
            };

            // Handle the request
            let response = match request.message {
                P2PMessage::Ping { nonce, .. } => {
                    info!("Received ping with nonce: {nonce}");
                    P2PResponse::new(
                        request.id,
                        P2PMessage::Pong {
                            timestamp: SystemTime::now(),
                            nonce,
                        },
                    )
                }
                P2PMessage::RequestAuthChallenge { message } => {
                    // Generate a fresh cryptographically secure challenge message for this auth attempt
                    let challenge_bytes: [u8; 32] = rand_v8::rngs::OsRng.gen();
                    let challenge_message = hex::encode(challenge_bytes);

                    debug!("Received request auth challenge");
                    let signature = match sign_message(&message, &wallet).await {
                        Ok(signature) => signature,
                        Err(e) => {
                            error!("Failed to sign message: {e}");
                            return Err(anyhow::anyhow!("Failed to sign message: {}", e));
                        }
                    };

                    // Store the challenge message in nonce cache to prevent replay
                    NONCE_CACHE.insert(challenge_message.clone(), SystemTime::now());

                    // Store the current challenge for this connection
                    current_challenge = Some(challenge_message.clone());

                    P2PResponse::new(
                        request.id,
                        P2PMessage::AuthChallenge {
                            message: challenge_message,
                            signed_message: signature,
                        },
                    )
                }
                P2PMessage::AuthSolution { signed_message } => {
                    // Get the challenge message for this connection
                    debug!("Received auth solution");
                    let Some(challenge_message) = &current_challenge else {
                        warn!("No active challenge for auth solution");
                        let response = P2PResponse::new(request.id, P2PMessage::AuthRejected {});
                        Self::write_response(&mut send, response).await?;
                        continue;
                    };

                    // Check if challenge message has been used before (replay attack prevention)
                    if !NONCE_CACHE.contains_key(challenge_message) {
                        warn!("Challenge message not found or expired: {challenge_message}");
                        let response = P2PResponse::new(request.id, P2PMessage::AuthRejected {});
                        Self::write_response(&mut send, response).await?;
                        continue;
                    }

                    // Clean up old nonces (older than 5 minutes)
                    let cutoff_time = SystemTime::now() - Duration::from_secs(300);
                    NONCE_CACHE.retain(|_, &mut timestamp| timestamp > cutoff_time);

                    // Parse the signature
                    let Ok(parsed_signature) =
                        alloy::primitives::Signature::from_str(&signed_message)
                    else {
                        // Handle signature parsing error
                        let response = P2PResponse::new(request.id, P2PMessage::AuthRejected {});
                        Self::write_response(&mut send, response).await?;
                        continue;
                    };

                    // Recover address from the challenge message that the client signed
                    let Ok(recovered_address) =
                        parsed_signature.recover_address_from_msg(challenge_message)
                    else {
                        // Handle address recovery error
                        let response = P2PResponse::new(request.id, P2PMessage::AuthRejected {});
                        Self::write_response(&mut send, response).await?;
                        continue;
                    };

                    // Check if the recovered address is in allowed addresses
                    NONCE_CACHE.remove(challenge_message);
                    current_challenge = None;
                    if allowed_addresses.contains(&recovered_address) {
                        is_authorized = true;
                        P2PResponse::new(request.id, P2PMessage::AuthGranted {})
                    } else {
                        P2PResponse::new(request.id, P2PMessage::AuthRejected {})
                    }
                }
                P2PMessage::HardwareChallenge { challenge, .. } if is_authorized => {
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
                P2PMessage::Invite(invite) if is_authorized => {
                    if let Some(context) = &context {
                        let (status, error) = Self::handle_invite(invite, context).await;
                        P2PResponse::new(request.id, P2PMessage::InviteResponse { status, error })
                    } else {
                        P2PResponse::new(
                            request.id,
                            P2PMessage::InviteResponse {
                                status: "error".to_string(),
                                error: Some("No context".to_string()),
                            },
                        )
                    }
                }
                P2PMessage::GetTaskLogs if is_authorized => {
                    if let Some(context) = &context {
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
                    } else {
                        P2PResponse::new(
                            request.id,
                            P2PMessage::GetTaskLogsResponse { logs: Ok(vec![]) },
                        )
                    }
                }
                P2PMessage::RestartTask if is_authorized => {
                    if let Some(context) = &context {
                        let result = context.docker_service.restart_task().await;
                        let response_result = result.map_err(|e| e.to_string());
                        P2PResponse::new(
                            request.id,
                            P2PMessage::RestartTaskResponse {
                                result: response_result,
                            },
                        )
                    } else {
                        P2PResponse::new(
                            request.id,
                            P2PMessage::RestartTaskResponse { result: Ok(()) },
                        )
                    }
                }
                _ => {
                    warn!("Unexpected message type");
                    continue;
                }
            };

            // Send response
            Self::write_response(&mut send, response).await?;
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
                error!("Failed to decode invite hex string: {err:?}");
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
                error!("Failed to get pool info: {err:?}");
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
                error!("Failed to build join compute pool call: {err:?}");
                return (
                    "error".to_string(),
                    Some("Failed to build join compute pool call".to_string()),
                );
            }
        };
        let provider = &context.provider_wallet.provider;
        match retry_call(call, 3, provider.clone(), None).await {
            Ok(result) => {
                Console::section("WORKER JOINED COMPUTE POOL");
                Console::success(&format!(
                    "Successfully registered on chain with tx: {result}"
                ));
                Console::info(
                    "Status",
                    "Worker is now part of the compute pool and ready to receive tasks",
                );
            }
            Err(err) => {
                error!("Failed to join compute pool: {err:?}");
                return (
                    "error".to_string(),
                    Some(format!("Failed to join compute pool: {err}")),
                );
            }
        }
        let endpoint = if let Some(url) = &invite.master_url {
            format!("{url}/heartbeat")
        } else {
            match (&invite.master_ip, &invite.master_port) {
                (Some(ip), Some(port)) => format!("http://{ip}:{port}/heartbeat"),
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
            error!("Failed to start heartbeat service: {err:?}");
            return (
                "error".to_string(),
                Some("Failed to start heartbeat service".to_string()),
            );
        }

        ("ok".to_string(), None)
    }
}

#[cfg(test)]
mod tests {
    use rand_v8::Rng;
    use serial_test::serial;
    use shared::p2p::P2PClient;
    use url::Url;

    use super::*;

    async fn setup_test_service(
        include_addresses: bool,
    ) -> (P2PService, P2PClient, Address, Address) {
        let validator_wallet = shared::web3::wallet::Wallet::new(
            "0000000000000000000000000000000000000000000000000000000000000001",
            Url::parse("https://mainnet.infura.io/v3/9aa3d95b3bc440fa88ea12eaa4456161").unwrap(),
        )
        .unwrap();
        let worker_wallet = shared::web3::wallet::Wallet::new(
            "0000000000000000000000000000000000000000000000000000000000000002",
            Url::parse("https://mainnet.infura.io/v3/9aa3d95b3bc440fa88ea12eaa4456161").unwrap(),
        )
        .unwrap();
        let validator_wallet_address = validator_wallet.wallet.default_signer().address();
        let worker_wallet_address = worker_wallet.wallet.default_signer().address();
        let service = P2PService::new(
            None,
            CancellationToken::new(),
            None,
            worker_wallet,
            if include_addresses {
                vec![validator_wallet_address]
            } else {
                vec![]
            },
        )
        .await
        .unwrap();
        let client = P2PClient::new(validator_wallet.clone()).await.unwrap();
        (
            service,
            client,
            validator_wallet_address,
            worker_wallet_address,
        )
    }

    #[tokio::test]
    #[serial]
    async fn test_ping() {
        let (service, client, _, worker_wallet_address) = setup_test_service(true).await;
        let node_id = service.node_id().to_string();
        let addresses = service.listening_addresses().to_vec();
        let random_nonce = rand_v8::thread_rng().gen::<u64>();

        tokio::spawn(async move {
            service.start().unwrap();
        });

        let ping = P2PMessage::Ping {
            nonce: random_nonce,
            timestamp: SystemTime::now(),
        };

        let response = client
            .send_request(&node_id, &addresses, worker_wallet_address, ping, 20)
            .await
            .unwrap();

        let P2PMessage::Pong { nonce, .. } = response else {
            panic!("Expected Pong message");
        };
        let response_nonce = nonce;
        assert_eq!(response_nonce, random_nonce);
    }
    #[tokio::test]
    #[serial]
    async fn test_auth_error() {
        let (service, client, _, worker_wallet_address) = setup_test_service(false).await;
        let node_id = service.node_id().to_string();
        let addresses = service.listening_addresses().to_vec();

        tokio::spawn(async move {
            service.start().unwrap();
        });

        let ping = P2PMessage::Ping {
            nonce: rand_v8::thread_rng().gen::<u64>(),
            timestamp: SystemTime::now(),
        };

        // Since we set include_addresses to false, the client's wallet address
        // is not in the allowed_addresses list, so we expect auth to be rejected
        let result = client
            .send_request(&node_id, &addresses, worker_wallet_address, ping, 20)
            .await;

        assert!(
            result.is_err(),
            "Expected auth to be rejected but request succeeded"
        );
    }
}
