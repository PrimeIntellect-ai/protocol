use crate::constants::{DEFAULT_FUNDING_RETRY_COUNT, MESSAGE_QUEUE_TIMEOUT, P2P_SHUTDOWN_TIMEOUT};
use crate::error::{PrimeProtocolError, Result};
use crate::p2p_handler::auth::AuthenticationManager;
use crate::p2p_handler::message_processor::{MessageProcessor, MessageProcessorConfig};
use crate::worker::blockchain::{BlockchainConfig, BlockchainService};
use crate::worker::p2p_handler::{Message, MessageType, Service as P2PService};
use alloy::primitives::Address;
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
    // Validator and pool owner info
    validator_addresses: Option<Arc<std::collections::HashSet<Address>>>,
    pool_owner_address: Option<Address>,
    compute_manager_address: Option<Address>,
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
            validator_addresses: None,
            pool_owner_address: None,
            compute_manager_address: None,
        })
    }

    /// Start the worker client asynchronously
    pub async fn start_async(&mut self) -> Result<()> {
        log::info!("Starting WorkerClient");

        self.initialize_blockchain().await?;
        self.initialize_validator_and_pool_info().await?;
        self.initialize_auth_manager()?;
        self.start_p2p_service().await?;
        self.start_message_processor().await?;

        log::info!("WorkerClient started successfully");
        Ok(())
    }

    /// Stop the worker client asynchronously
    pub async fn stop_async(&mut self) -> Result<()> {
        log::info!("Stopping worker client...");
        self.cancellation_token.cancel();

        // Stop message processor
        if let Some(handle) = self.message_processor_handle.take() {
            handle.abort();
        }

        // Give P2P service time to shutdown gracefully
        tokio::time::sleep(P2P_SHUTDOWN_TIMEOUT).await;

        // Stop P2P service
        if let Some(handle) = self.p2p_state.handle.take() {
            let _ = tokio::time::timeout(P2P_SHUTDOWN_TIMEOUT, handle).await;
        }

        log::info!("Worker client stopped");
        Ok(())
    }

    /// Get the peer ID of this node
    pub fn get_peer_id(&self) -> Option<PeerId> {
        self.p2p_state.peer_id
    }

    /// Get the next message from the P2P network
    pub async fn get_next_message(&self) -> Option<Message> {
        let rx = self.user_message_rx.as_ref()?;

        match tokio::time::timeout(MESSAGE_QUEUE_TIMEOUT, rx.lock().await.recv()).await {
            Ok(Some(message)) => {
                // Check if it's an invite and process it automatically
                if let MessageType::General { ref data } = message.message_type {
                    if let Ok(invite) = serde_json::from_slice::<p2p::InviteRequest>(data) {
                        log::info!("Received invite from peer: {}", message.peer_id);

                        // Check if invite has expired
                        if let Ok(true) = operations::invite::worker::is_invite_expired(&invite) {
                            log::warn!("Received expired invite from peer: {}", message.peer_id);
                            return Some(message); // Return it so user can see the expired invite
                        }

                        // Verify pool ID matches
                        if invite.pool_id != self.config.compute_pool_id as u32 {
                            log::warn!(
                                "Received invite for wrong pool: expected {}, got {}",
                                self.config.compute_pool_id,
                                invite.pool_id
                            );
                            return Some(message); // Return it so user can see the wrong pool invite
                        }

                        // Process the invite automatically
                        if let Some(blockchain_service) = &self.blockchain_service {
                            match blockchain_service
                                .join_compute_pool_with_invite(&invite)
                                .await
                            {
                                Ok(()) => {
                                    log::info!(
                                        "Successfully processed invite and joined compute pool"
                                    );
                                    // Don't return the invite message since we handled it
                                    return None;
                                }
                                Err(e) => {
                                    log::error!("Failed to process invite: {}", e);
                                    // Return the message so user knows about the failed invite
                                }
                            }
                        }
                    }
                }
                Some(message)
            }
            Ok(None) => None,
            Err(_) => None, // Timeout
        }
    }

    /// Send a message to a peer
    pub async fn send_message(&self, message: Message) -> Result<()> {
        let auth_manager = self.auth_manager.as_ref().ok_or_else(|| {
            PrimeProtocolError::InvalidConfig("Authentication manager not initialized".to_string())
        })?;

        let tx = self.p2p_state.outbound_tx.as_ref().ok_or_else(|| {
            PrimeProtocolError::InvalidConfig("P2P service not initialized".to_string())
        })?;

        crate::p2p_handler::send_message_with_auth(message, auth_manager, tx).await
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

    async fn initialize_validator_and_pool_info(&mut self) -> Result<()> {
        let blockchain_service = self.blockchain_service.as_ref().ok_or_else(|| {
            PrimeProtocolError::InvalidConfig("Blockchain service not initialized".to_string())
        })?;

        let validator_addresses = blockchain_service.get_validator_addresses().await;
        self.validator_addresses = Some(validator_addresses);

        let pool_owner_address = blockchain_service.get_pool_owner_address().await;
        self.pool_owner_address = pool_owner_address;

        let compute_manager_address = blockchain_service.get_compute_manager_address().await;
        self.compute_manager_address = compute_manager_address;

        log::info!(
            "Validator addresses: {:?}",
            self.validator_addresses.as_ref().map(|s| s.len())
        );
        log::info!("Pool owner address: {:?}", self.pool_owner_address);
        log::info!(
            "Compute manager address: {:?}",
            self.compute_manager_address
        );

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
        // Build the message processor configuration
        let config = self.build_message_processor_config()?;

        // Create and spawn the message processor
        let message_processor = MessageProcessor::from_config(config);
        self.message_processor_handle = Some(message_processor.spawn());

        log::info!("Message processor started");
        Ok(())
    }

    /// Build configuration for the message processor
    /// This method is public to allow reuse in other crates
    pub fn build_message_processor_config(&self) -> Result<MessageProcessorConfig> {
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

        let validator_addresses = self
            .validator_addresses
            .as_ref()
            .ok_or_else(|| {
                PrimeProtocolError::InvalidConfig("Validator addresses not initialized".to_string())
            })?
            .clone();

        let pool_owner_address = self.pool_owner_address.ok_or_else(|| {
            PrimeProtocolError::InvalidConfig("Pool owner address not initialized".to_string())
        })?;

        let compute_manager_address = self.compute_manager_address;

        Ok(MessageProcessorConfig {
            auth_manager,
            message_queue_rx,
            user_message_tx,
            outbound_tx,
            authenticated_peers,
            cancellation_token: self.cancellation_token.clone(),
            validator_addresses,
            pool_owner_address: Some(pool_owner_address),
            compute_manager_address,
        })
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
