use crate::error::Result;
use crate::p2p_handler::auth::AuthenticationManager;
use crate::p2p_handler::{Message, MessageType};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::{
    mpsc::{Receiver, Sender},
    Mutex, RwLock,
};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Configuration for creating a MessageProcessor
pub struct MessageProcessorConfig {
    pub auth_manager: Arc<AuthenticationManager>,
    pub message_queue_rx: Arc<Mutex<Receiver<Message>>>,
    pub user_message_tx: Sender<Message>,
    pub outbound_tx: Arc<Mutex<Sender<Message>>>,
    pub authenticated_peers: Arc<RwLock<HashMap<String, String>>>,
    pub cancellation_token: CancellationToken,
}

/// Handles processing of incoming P2P messages
pub struct MessageProcessor {
    auth_manager: Arc<AuthenticationManager>,
    message_queue_rx: Arc<Mutex<Receiver<Message>>>,
    user_message_tx: Sender<Message>,
    outbound_tx: Arc<Mutex<Sender<Message>>>,
    authenticated_peers: Arc<RwLock<HashMap<String, String>>>,
    cancellation_token: CancellationToken,
}

impl MessageProcessor {
    pub fn new(
        auth_manager: Arc<AuthenticationManager>,
        message_queue_rx: Arc<Mutex<Receiver<Message>>>,
        user_message_tx: Sender<Message>,
        outbound_tx: Arc<Mutex<Sender<Message>>>,
        authenticated_peers: Arc<RwLock<HashMap<String, String>>>,
        cancellation_token: CancellationToken,
    ) -> Self {
        Self {
            auth_manager,
            message_queue_rx,
            user_message_tx,
            outbound_tx,
            authenticated_peers,
            cancellation_token,
        }
    }

    /// Create a MessageProcessor from a config struct
    pub fn from_config(config: MessageProcessorConfig) -> Self {
        Self::new(
            config.auth_manager,
            config.message_queue_rx,
            config.user_message_tx,
            config.outbound_tx,
            config.authenticated_peers,
            config.cancellation_token,
        )
    }

    /// Start the message processor as a background task
    /// Returns a JoinHandle that can be used to await or abort the task
    pub fn spawn(self) -> JoinHandle<()> {
        tokio::task::spawn(self.run())
    }

    /// Run the message processing loop
    pub async fn run(self) {
        loop {
            tokio::select! {
                _ = self.cancellation_token.cancelled() => {
                    log::info!("Message processor shutting down");
                    break;
                }
                message_result = async {
                    let mut rx = self.message_queue_rx.lock().await;
                    tokio::time::timeout(
                        crate::constants::MESSAGE_QUEUE_TIMEOUT,
                        rx.recv()
                    ).await
                } => {
                    let message = match message_result {
                        Ok(Some(msg)) => msg,
                        Ok(None) => {
                            log::debug!("Message queue closed");
                            break;
                        }
                        Err(_) => continue, // Timeout, continue loop
                    };

                    log::debug!("Received message: {:?}", message);

                    if let Err(e) = self.process_message(message).await {
                        log::error!("Failed to process message: {}", e);
                    }
                }
            }
        }
    }

    /// Process a single message
    async fn process_message(&self, message: Message) -> Result<()> {
        let Message {
            message_type,
            peer_id,
            multiaddrs,
            sender_address,
            response_tx,
        } = message;

        match message_type {
            MessageType::AuthenticationInitiation { challenge } => {
                if let Some(tx) = response_tx {
                    self.handle_auth_initiation_with_response(
                        peer_id,
                        multiaddrs,
                        sender_address,
                        challenge,
                        tx,
                    )
                    .await
                } else {
                    log::error!("AuthenticationInitiation received without response tx");
                    Ok(())
                }
            }
            MessageType::AuthenticationResponse {
                challenge,
                signature,
            } => {
                let msg = Message {
                    message_type: MessageType::AuthenticationResponse {
                        challenge: challenge.clone(),
                        signature: signature.clone(),
                    },
                    peer_id,
                    multiaddrs,
                    sender_address,
                    response_tx: None,
                };
                self.handle_auth_response(msg, challenge, signature).await
            }
            MessageType::AuthenticationSolution { signature } => {
                if let Some(tx) = response_tx {
                    self.handle_auth_solution_with_response(
                        peer_id,
                        multiaddrs,
                        sender_address,
                        signature,
                        tx,
                    )
                    .await
                } else {
                    log::error!("AuthenticationSolution received without response tx");
                    Ok(())
                }
            }
            MessageType::AuthenticationComplete => {
                // Authentication has been acknowledged by the peer
                self.handle_auth_complete(peer_id).await
            }
            MessageType::General { data } => {
                // Forward general messages to user
                let msg = Message {
                    message_type: MessageType::General { data },
                    peer_id,
                    multiaddrs,
                    sender_address,
                    response_tx: None,
                };
                self.user_message_tx.send(msg).await.map_err(|e| {
                    crate::error::PrimeProtocolError::InvalidConfig(format!(
                        "Failed to forward message to user: {}",
                        e
                    ))
                })
            }
        }
    }

    async fn handle_auth_initiation_with_response(
        &self,
        peer_id: String,
        _multiaddrs: Vec<String>,
        _sender_address: Option<String>,
        challenge: String,
        response_tx: tokio::sync::oneshot::Sender<p2p::Response>,
    ) -> Result<()> {
        let (our_challenge, signature) = self
            .auth_manager
            .handle_auth_initiation(&peer_id, &challenge)
            .await?;

        // Send authentication response via the one-shot channel
        let response = p2p::AuthenticationInitiationResponse {
            message: our_challenge,
            signature,
        };

        response_tx.send(response.into()).map_err(|_| {
            crate::error::PrimeProtocolError::InvalidConfig(
                "Failed to send auth response: receiver dropped".to_string(),
            )
        })
    }

    async fn handle_auth_response(
        &self,
        message: Message,
        challenge: String,
        signature: String,
    ) -> Result<()> {
        let our_signature = self
            .auth_manager
            .handle_auth_response(&message.peer_id, &challenge, &signature)
            .await?;

        // Extract and store the peer's wallet address from their signature
        if let Ok(parsed_signature) = alloy::primitives::Signature::from_str(&signature) {
            if let Ok(recovered_address) = parsed_signature.recover_address_from_msg(&challenge) {
                self.authenticated_peers
                    .write()
                    .await
                    .insert(message.peer_id.clone(), recovered_address.to_string());
            }
        }

        // Send authentication solution
        let solution = Message {
            message_type: MessageType::AuthenticationSolution {
                signature: our_signature,
            },
            peer_id: message.peer_id.clone(),
            multiaddrs: message.multiaddrs,
            sender_address: Some(self.auth_manager.wallet_address()),
            response_tx: None,
        };

        self.outbound_tx
            .lock()
            .await
            .send(solution)
            .await
            .map_err(|e| {
                crate::error::PrimeProtocolError::InvalidConfig(format!(
                    "Failed to send auth solution: {}",
                    e
                ))
            })?;

        Ok(())
    }

    async fn handle_auth_solution_with_response(
        &self,
        peer_id: String,
        _multiaddrs: Vec<String>,
        _sender_address: Option<String>,
        signature: String,
        response_tx: tokio::sync::oneshot::Sender<p2p::Response>,
    ) -> Result<()> {
        let (peer_address, queued_messages) = self
            .auth_manager
            .handle_auth_solution(&peer_id, &signature)
            .await?;

        // Store the peer's wallet address for future message handling
        self.authenticated_peers
            .write()
            .await
            .insert(peer_id.clone(), peer_address.to_string());

        // Send any queued messages now that we're authenticated
        for msg in queued_messages {
            self.outbound_tx.lock().await.send(msg).await.map_err(|e| {
                crate::error::PrimeProtocolError::InvalidConfig(format!(
                    "Failed to send queued message: {}",
                    e
                ))
            })?;
        }

        // Send the response
        let response = p2p::AuthenticationSolutionResponse::Granted;
        response_tx.send(response.into()).map_err(|_| {
            crate::error::PrimeProtocolError::InvalidConfig(
                "Failed to send auth solution response: receiver dropped".to_string(),
            )
        })
    }

    async fn handle_auth_complete(&self, peer_id: String) -> Result<()> {
        log::info!("Authentication complete for peer: {}", peer_id);

        // Mark peer as authenticated
        self.auth_manager.mark_authenticated(peer_id.clone()).await;

        // Get and send any queued message
        if let Some(queued_message) = self.auth_manager.handle_auth_acknowledgment(&peer_id).await {
            log::debug!("Sending queued message to peer {}", peer_id);
            self.outbound_tx
                .lock()
                .await
                .send(queued_message)
                .await
                .map_err(|e| {
                    crate::error::PrimeProtocolError::InvalidConfig(format!(
                        "Failed to send queued message: {}",
                        e
                    ))
                })?;
        }

        Ok(())
    }
}
