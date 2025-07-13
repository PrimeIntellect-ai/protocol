use crate::constants::{DEFAULT_FUNDING_RETRY_COUNT, MESSAGE_QUEUE_TIMEOUT, P2P_SHUTDOWN_TIMEOUT};
use crate::error::{PrimeProtocolError, Result};
use crate::p2p_handler::auth::AuthenticationManager;
use crate::p2p_handler::message_processor::MessageProcessor;
use crate::worker::blockchain::{BlockchainConfig, BlockchainService};
use crate::worker::p2p_handler::{Message, MessageType, Service as P2PService};
use p2p::{Keypair, PeerId};
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use url::Url;

/// Core worker client that handles P2P networking and blockchain operations
pub struct WorkerClientCore {
    config: ClientConfig,
    blockchain_service: Option<BlockchainService>,
    p2p_state: P2PState,
    auth_manager: Option<Arc<AuthenticationManager>>,
    cancellation_token: CancellationToken,
    // Message processing
    user_message_tx: Option<Sender<Message>>,
    user_message_rx: Option<Arc<Mutex<Receiver<Message>>>>,
    message_processor_handle: Option<JoinHandle<()>>,
}

/// Configuration for the worker client
struct ClientConfig {
    rpc_url: String,
    compute_pool_id: u64,
    private_key_provider: Option<String>,
    private_key_node: Option<String>,
    auto_accept_transactions: bool,
    funding_retry_count: u32,
    p2p_port: u16,
}

/// P2P networking state
struct P2PState {
    keypair: Keypair,
    peer_id: Option<PeerId>,
    outbound_tx: Option<Arc<Mutex<Sender<Message>>>>,
    message_queue_rx: Option<Arc<Mutex<Receiver<Message>>>>,
    handle: Option<JoinHandle<anyhow::Result<()>>>,
    authenticated_peers:
        Option<Arc<tokio::sync::RwLock<std::collections::HashMap<String, String>>>>,
}

impl WorkerClientCore {
    /// Create a new worker client
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        compute_pool_id: u64,
        rpc_url: String,
        private_key_provider: Option<String>,
        private_key_node: Option<String>,
        auto_accept_transactions: Option<bool>,
        funding_retry_count: Option<u32>,
        cancellation_token: CancellationToken,
        p2p_port: u16,
    ) -> Result<Self> {
        // Validate inputs
        if rpc_url.is_empty() {
            return Err(PrimeProtocolError::InvalidConfig(
                "RPC URL cannot be empty".to_string(),
            ));
        }

        Url::parse(&rpc_url)
            .map_err(|_| PrimeProtocolError::InvalidConfig("Invalid RPC URL format".to_string()))?;

        let config = ClientConfig {
            rpc_url,
            compute_pool_id,
            private_key_provider,
            private_key_node,
            auto_accept_transactions: auto_accept_transactions.unwrap_or(true),
            funding_retry_count: funding_retry_count.unwrap_or(DEFAULT_FUNDING_RETRY_COUNT),
            p2p_port,
        };

        let p2p_state = P2PState {
            keypair: Keypair::generate_ed25519(),
            peer_id: None,
            outbound_tx: None,
            message_queue_rx: None,
            handle: None,
            authenticated_peers: None,
        };

        // Create user message channel
        let (user_message_tx, user_message_rx) = tokio::sync::mpsc::channel::<Message>(1000);

        Ok(Self {
            config,
            blockchain_service: None,
            p2p_state,
            auth_manager: None,
            cancellation_token,
            user_message_tx: Some(user_message_tx),
            user_message_rx: Some(Arc::new(Mutex::new(user_message_rx))),
            message_processor_handle: None,
        })
    }

    /// Start the worker client asynchronously
    pub async fn start_async(&mut self) -> Result<()> {
        log::info!("Starting worker client...");

        // Initialize blockchain components
        self.initialize_blockchain().await?;

        // Initialize authentication manager
        self.initialize_auth_manager()?;

        // Start P2P networking
        self.start_p2p_service().await?;

        // Start message processor
        self.start_message_processor().await?;

        log::info!("Worker client started successfully");
        Ok(())
    }

    /// Stop the worker client and clean up resources
    pub async fn stop_async(&mut self) -> Result<()> {
        log::info!("Stopping worker client...");

        // Cancel all background tasks
        self.cancellation_token.cancel();

        // Stop message processor
        if let Some(handle) = self.message_processor_handle.take() {
            handle.abort();
        }

        // Wait for P2P service to shut down
        if let Some(handle) = self.p2p_state.handle.take() {
            match tokio::time::timeout(P2P_SHUTDOWN_TIMEOUT, handle).await {
                Ok(Ok(_)) => log::info!("P2P service shut down gracefully"),
                Ok(Err(e)) => log::error!("P2P service error during shutdown: {:?}", e),
                Err(_) => log::warn!("P2P service shutdown timed out"),
            }
        }

        log::info!("Worker client stopped");
        Ok(())
    }

    /// Get the peer ID of this node
    pub fn get_peer_id(&self) -> Option<PeerId> {
        self.p2p_state.peer_id
    }

    /// Get the next message from the P2P network (only returns general messages)
    pub async fn get_next_message(&self) -> Option<Message> {
        let rx = self.user_message_rx.as_ref()?;

        tokio::time::timeout(MESSAGE_QUEUE_TIMEOUT, rx.lock().await.recv())
            .await
            .ok()
            .flatten()
    }

    /// Send a message to a peer
    pub async fn send_message(&self, message: Message) -> Result<()> {
        log::debug!("Sending message to peer: {}", message.peer_id);

        let auth_manager = self.auth_manager.as_ref().ok_or_else(|| {
            PrimeProtocolError::InvalidConfig("Authentication manager not initialized".to_string())
        })?;

        let tx = self.p2p_state.outbound_tx.as_ref().ok_or_else(|| {
            PrimeProtocolError::InvalidConfig("P2P service not initialized".to_string())
        })?;

        // Check if we're already authenticated with this peer
        if auth_manager.is_authenticated(&message.peer_id).await {
            log::debug!(
                "Already authenticated with peer {}, sending message directly",
                message.peer_id
            );
            return tx.lock().await.send(message).await.map_err(|e| {
                PrimeProtocolError::InvalidConfig(format!("Failed to send message: {}", e))
            });
        }

        // Not authenticated yet, check if we have ongoing authentication
        log::debug!("Not authenticated with peer {}", message.peer_id);

        // Check if there's already an ongoing auth request
        if let Some(role) = auth_manager.get_auth_role(&message.peer_id).await {
            match role.as_str() {
                "initiator" => {
                    return Err(PrimeProtocolError::InvalidConfig(format!(
                        "Already initiated authentication with peer {}",
                        message.peer_id
                    )));
                }
                "responder" => {
                    // We're responding to their auth, queue the message
                    log::debug!(
                        "Queuing message for peer {} (we're responding to their auth)",
                        message.peer_id
                    );
                    return auth_manager
                        .queue_message_as_responder(message.peer_id.clone(), message)
                        .await;
                }
                _ => {}
            }
        }

        // Extract fields we need before moving the message
        let peer_id = message.peer_id.clone();
        let multiaddrs = message.multiaddrs.clone();

        // Start authentication (takes ownership of message)
        let auth_challenge = auth_manager
            .start_authentication(peer_id.clone(), message)
            .await?;

        // Send authentication initiation
        let auth_message = Message {
            message_type: MessageType::AuthenticationInitiation {
                challenge: auth_challenge,
            },
            peer_id,
            multiaddrs,
            sender_address: Some(auth_manager.wallet_address()),
            response_tx: None,
        };

        tx.lock().await.send(auth_message).await.map_err(|e| {
            PrimeProtocolError::InvalidConfig(format!("Failed to send auth message: {}", e))
        })
    }

    // Private helper methods

    async fn initialize_blockchain(&mut self) -> Result<()> {
        let blockchain_config = BlockchainConfig {
            rpc_url: self.config.rpc_url.clone(),
            compute_pool_id: self.config.compute_pool_id,
            private_key_provider: self.get_private_key_provider()?,
            private_key_node: self.get_private_key_node()?,
            auto_accept_transactions: self.config.auto_accept_transactions,
            funding_retry_count: self.config.funding_retry_count,
        };

        // Create blockchain service - wallets are created internally
        let mut blockchain_service = BlockchainService::new(blockchain_config).map_err(|e| {
            PrimeProtocolError::BlockchainError(format!(
                "Failed to create blockchain service: {}",
                e
            ))
        })?;

        blockchain_service.initialize().await.map_err(|e| {
            PrimeProtocolError::BlockchainError(format!("Failed to initialize blockchain: {}", e))
        })?;

        self.blockchain_service = Some(blockchain_service);
        Ok(())
    }

    fn initialize_auth_manager(&mut self) -> Result<()> {
        let blockchain_service = self.blockchain_service.as_ref().ok_or_else(|| {
            PrimeProtocolError::InvalidConfig("Blockchain service not initialized".to_string())
        })?;

        let node_wallet = blockchain_service.node_wallet().ok_or_else(|| {
            PrimeProtocolError::InvalidConfig("Node wallet not initialized".to_string())
        })?;

        self.auth_manager = Some(Arc::new(AuthenticationManager::new(Arc::new(
            node_wallet.clone(),
        ))));
        Ok(())
    }

    async fn start_p2p_service(&mut self) -> Result<()> {
        // Get wallet address from auth manager
        let wallet_address = self.auth_manager.as_ref().map(|auth| auth.wallet_address());

        let (p2p_service, outbound_tx, message_queue_rx, authenticated_peers) = P2PService::new(
            self.p2p_state.keypair.clone(),
            self.config.p2p_port,
            self.cancellation_token.clone(),
            wallet_address,
        )
        .map_err(|e| {
            PrimeProtocolError::InvalidConfig(format!("Failed to create P2P service: {}", e))
        })?;

        self.p2p_state.peer_id = Some(p2p_service.node.peer_id());
        self.p2p_state.outbound_tx = Some(Arc::new(Mutex::new(outbound_tx)));
        self.p2p_state.message_queue_rx = Some(Arc::new(Mutex::new(message_queue_rx)));
        self.p2p_state.authenticated_peers = Some(authenticated_peers);

        log::info!(
            "P2P service initialized with peer ID: {:?}",
            self.p2p_state.peer_id
        );

        self.p2p_state.handle = Some(tokio::task::spawn(p2p_service.run()));
        Ok(())
    }

    async fn start_message_processor(&mut self) -> Result<()> {
        let message_queue_rx = self
            .p2p_state
            .message_queue_rx
            .as_ref()
            .ok_or_else(|| {
                PrimeProtocolError::InvalidConfig("P2P service not initialized".to_string())
            })?
            .clone();

        let user_message_tx = self
            .user_message_tx
            .as_ref()
            .ok_or_else(|| {
                PrimeProtocolError::InvalidConfig(
                    "User message channel not initialized".to_string(),
                )
            })?
            .clone();

        let auth_manager = self
            .auth_manager
            .as_ref()
            .ok_or_else(|| {
                PrimeProtocolError::InvalidConfig(
                    "Authentication manager not initialized".to_string(),
                )
            })?
            .clone();

        let outbound_tx = self
            .p2p_state
            .outbound_tx
            .as_ref()
            .ok_or_else(|| {
                PrimeProtocolError::InvalidConfig("P2P service not initialized".to_string())
            })?
            .clone();

        let authenticated_peers = self
            .p2p_state
            .authenticated_peers
            .as_ref()
            .ok_or_else(|| {
                PrimeProtocolError::InvalidConfig(
                    "Authenticated peers map not initialized".to_string(),
                )
            })?
            .clone();

        let message_processor = MessageProcessor::new(
            auth_manager,
            message_queue_rx,
            user_message_tx,
            outbound_tx,
            authenticated_peers,
            self.cancellation_token.clone(),
        );

        self.message_processor_handle = Some(tokio::task::spawn(message_processor.run()));
        Ok(())
    }

    /// Get the provider's Ethereum address
    pub fn get_provider_address(&self) -> Result<String> {
        let blockchain_service = self.blockchain_service.as_ref().ok_or_else(|| {
            PrimeProtocolError::InvalidConfig("Blockchain service not initialized".to_string())
        })?;
        let provider_wallet = blockchain_service.provider_wallet().ok_or_else(|| {
            PrimeProtocolError::InvalidConfig("Provider wallet not initialized".to_string())
        })?;
        Ok(provider_wallet
            .wallet
            .default_signer()
            .address()
            .to_string())
    }

    // todo: move to blockchain service
    pub fn get_node_address(&self) -> Result<String> {
        let blockchain_service = self.blockchain_service.as_ref().ok_or_else(|| {
            PrimeProtocolError::InvalidConfig("Blockchain service not initialized".to_string())
        })?;
        let node_wallet = blockchain_service.node_wallet().ok_or_else(|| {
            PrimeProtocolError::InvalidConfig("Node wallet not initialized".to_string())
        })?;
        Ok(node_wallet.wallet.default_signer().address().to_string())
    }

    /// Get the compute pool ID
    pub fn get_compute_pool_id(&self) -> u64 {
        self.config.compute_pool_id
    }

    /// Get the listening multiaddresses from the P2P service
    pub async fn get_listening_addresses(&self) -> Vec<String> {
        // For now, return a simple localhost address with the configured port
        // In the future, this could query the actual P2P service for its listen addresses
        vec![format!("/ip4/0.0.0.0/tcp/{}", self.config.p2p_port)]
    }

    /// Upload node information to discovery services
    pub async fn upload_to_discovery(
        &self,
        node_info: &crate::worker::discovery::SimpleNode,
        discovery_urls: &[String],
    ) -> Result<()> {
        let blockchain_service = self.blockchain_service.as_ref().ok_or_else(|| {
            PrimeProtocolError::InvalidConfig("Blockchain service not initialized".to_string())
        })?;

        let node_wallet = blockchain_service.node_wallet().ok_or_else(|| {
            PrimeProtocolError::InvalidConfig("Provider wallet not initialized".to_string())
        })?;

        let node_json = node_info.to_json_value();

        shared::discovery::upload_node_to_discovery(discovery_urls, &node_json, node_wallet)
            .await
            .map_err(|e| {
                PrimeProtocolError::RuntimeError(format!("Failed to upload to discovery: {}", e))
            })
    }

    fn get_private_key_provider(&self) -> Result<String> {
        self.config
            .private_key_provider
            .clone()
            .or_else(|| std::env::var("PRIVATE_KEY_PROVIDER").ok())
            .ok_or_else(|| {
                PrimeProtocolError::InvalidConfig(
                    "PRIVATE_KEY_PROVIDER must be set either as parameter or environment variable"
                        .to_string(),
                )
            })
    }

    fn get_private_key_node(&self) -> Result<String> {
        self.config
            .private_key_node
            .clone()
            .or_else(|| std::env::var("PRIVATE_KEY_NODE").ok())
            .ok_or_else(|| {
                PrimeProtocolError::InvalidConfig(
                    "PRIVATE_KEY_NODE must be set either as parameter or environment variable"
                        .to_string(),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    fn create_test_config() -> (String, String) {
        // Standard Anvil blockchain keys for local testing
        let node_key = "0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6";
        let provider_key = "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a";
        (node_key.to_string(), provider_key.to_string())
    }

    #[test]
    fn test_client_creation() {
        let (node_key, provider_key) = create_test_config();
        let cancellation_token = CancellationToken::new();

        let result = WorkerClientCore::new(
            0,
            "http://localhost:8545".to_string(),
            Some(provider_key),
            Some(node_key),
            None,
            None,
            cancellation_token,
            8000,
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_invalid_rpc_url() {
        let (node_key, provider_key) = create_test_config();
        let cancellation_token = CancellationToken::new();

        let result = WorkerClientCore::new(
            0,
            "invalid-url".to_string(),
            Some(provider_key),
            Some(node_key),
            None,
            None,
            cancellation_token,
            8000,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_empty_rpc_url() {
        let (node_key, provider_key) = create_test_config();
        let cancellation_token = CancellationToken::new();

        let result = WorkerClientCore::new(
            0,
            "".to_string(),
            Some(provider_key),
            Some(node_key),
            None,
            None,
            cancellation_token,
            8000,
        );

        assert!(result.is_err());
    }
}
