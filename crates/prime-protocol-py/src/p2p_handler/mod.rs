use anyhow::{Context, Result};
use p2p::{IncomingMessage, Keypair, Node, NodeBuilder, OutgoingMessage, PeerId, Protocols};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::{
    mpsc::{Receiver, Sender},
    RwLock,
};
use tokio_util::sync::CancellationToken;

use crate::constants::{MESSAGE_QUEUE_CHANNEL_SIZE, P2P_CHANNEL_SIZE};

pub(crate) mod auth;
pub(crate) mod common;
pub(crate) mod message_processor;

pub use common::send_message_with_auth;

// Type alias for the complex return type of Service::new
type ServiceNewResult = Result<(
    Service,
    Sender<Message>,
    Receiver<Message>,
    Arc<RwLock<HashMap<String, String>>>,
)>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageType {
    General {
        data: Vec<u8>,
    },
    AuthenticationInitiation {
        challenge: String,
    },
    AuthenticationResponse {
        challenge: String,
        signature: String,
    },
    AuthenticationSolution {
        signature: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    pub message_type: MessageType,
    pub peer_id: String,
    pub multiaddrs: Vec<String>,
    pub sender_address: Option<String>, // Ethereum address of the sender
    #[serde(skip)]
    pub response_tx: Option<tokio::sync::oneshot::Sender<p2p::Response>>, // For sending responses to auth requests
}

pub struct Service {
    p2p_rx: Receiver<Message>,
    outbound_message_tx: Sender<OutgoingMessage>,
    incoming_message_rx: Receiver<IncomingMessage>,
    message_queue_tx: Sender<Message>,
    pub node: Node,
    cancellation_token: CancellationToken,
    wallet_address: Option<String>, // Our wallet address for authentication
    // Map peer_id to their wallet address after authentication
    authenticated_peers: Arc<RwLock<HashMap<String, String>>>,
}

impl Service {
    pub(crate) fn new(
        keypair: Keypair,
        port: u16,
        cancellation_token: CancellationToken,
        wallet_address: Option<String>,
    ) -> ServiceNewResult {
        // Channels for application <-> P2P service communication
        let (p2p_tx, p2p_rx) = tokio::sync::mpsc::channel(P2P_CHANNEL_SIZE);
        let (message_queue_tx, message_queue_rx) =
            tokio::sync::mpsc::channel(MESSAGE_QUEUE_CHANNEL_SIZE);

        let protocols = Protocols::new().with_general().with_authentication();

        let listen_addr = format!("/ip4/0.0.0.0/tcp/{}", port)
            .parse()
            .context("Failed to parse listen address")?;

        let (node, incoming_messages_rx, outgoing_messages_tx) = NodeBuilder::new()
            .with_keypair(keypair)
            .with_port(port)
            .with_listen_addr(listen_addr)
            .with_protocols(protocols)
            .with_cancellation_token(cancellation_token.clone())
            .try_build()
            .context("Failed to create P2P node")?;

        let authenticated_peers = Arc::new(RwLock::new(HashMap::new()));

        Ok((
            Self {
                p2p_rx,
                outbound_message_tx: outgoing_messages_tx,
                incoming_message_rx: incoming_messages_rx,
                message_queue_tx,
                node,
                cancellation_token,
                wallet_address,
                authenticated_peers: authenticated_peers.clone(),
            },
            p2p_tx,
            message_queue_rx,
            authenticated_peers,
        ))
    }

    async fn handle_outgoing_message(
        mut message: Message,
        outgoing_message_tx: &Sender<OutgoingMessage>,
        wallet_address: &Option<String>,
    ) -> Result<()> {
        // Add our wallet address if not already set
        if message.sender_address.is_none() {
            message.sender_address = wallet_address.clone();
        }

        let req = match &message.message_type {
            MessageType::General { data } => {
                p2p::Request::General(p2p::GeneralRequest { data: data.clone() })
            }
            MessageType::AuthenticationInitiation { challenge } => p2p::Request::Authentication(
                p2p::AuthenticationRequest::Initiation(p2p::AuthenticationInitiationRequest {
                    message: challenge.clone(),
                }),
            ),
            MessageType::AuthenticationResponse {
                challenge: _,
                signature: _,
            } => {
                // This message type should not be sent as a request
                // It should be handled via response channels
                log::error!(
                    "AuthenticationResponse should be sent via response channel, not as a request"
                );
                return Ok(());
            }
            MessageType::AuthenticationSolution { signature } => p2p::Request::Authentication(
                p2p::AuthenticationRequest::Solution(p2p::AuthenticationSolutionRequest {
                    signature: signature.clone(),
                }),
            ),
        };

        let peer_id = PeerId::from_str(&message.peer_id).context("Failed to parse peer ID")?;

        let multiaddrs = message
            .multiaddrs
            .iter()
            .map(|addr| addr.parse())
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to parse multiaddresses")?;

        log::debug!(
            "Sending message to peer: {}, multiaddrs: {:?}",
            peer_id,
            multiaddrs
        );

        outgoing_message_tx
            .send(req.into_outgoing_message(peer_id, multiaddrs))
            .await
            .context("Failed to send outgoing message")?;

        Ok(())
    }

    pub(crate) async fn run(self) -> Result<()> {
        // Extract all necessary fields before the async move block
        let node = self.node;
        let node_cancel_token = self.cancellation_token.clone();
        let mut p2p_rx = self.p2p_rx;
        let outbound_message_tx = self.outbound_message_tx;
        let mut incoming_message_rx = self.incoming_message_rx;
        let message_queue_tx = self.message_queue_tx;
        let cancellation_token = self.cancellation_token;
        let wallet_address = self.wallet_address;
        let authenticated_peers = self.authenticated_peers;

        // Start the P2P node in a separate task
        let node_handle = tokio::spawn(async move {
            tokio::select! {
                result = node.run() => {
                    if let Err(e) = result {
                        log::error!("P2P node error: {}", e);
                    }
                }
                _ = node_cancel_token.cancelled() => {
                    log::info!("P2P node shutdown requested");
                }
            }
        });

        log::info!("P2P service started");

        // Main event loop
        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => {
                    log::info!("P2P service shutdown requested");
                    break;
                }

                // Handle outgoing messages from application
                Some(message) = p2p_rx.recv() => {
                    if let Err(e) = Self::handle_outgoing_message(message, &outbound_message_tx, &wallet_address).await {
                        log::error!("Failed to handle outgoing message: {}", e);
                    }
                }

                // Handle incoming messages from network
                Some(incoming) = incoming_message_rx.recv() => {
                    let peer_id = incoming.peer;
                    match incoming.message {
                        p2p::Libp2pIncomingMessage::Request {
                            request_id,
                            request,
                            channel,
                        } => {
                            if let Err(e) = Self::handle_incoming_request_static(
                                &message_queue_tx,
                                &outbound_message_tx,
                                &authenticated_peers,
                                peer_id,
                                request_id,
                                request,
                                channel
                            ).await {
                                log::error!("Failed to handle incoming request: {}", e);
                            }
                        }
                        p2p::Libp2pIncomingMessage::Response {
                            request_id,
                            response,
                        } => {
                            if let Err(e) = Self::handle_incoming_response_static(
                                &message_queue_tx,
                                peer_id,
                                request_id,
                                response
                            ).await {
                                log::error!("Failed to handle incoming response: {}", e);
                            }
                        }
                    }
                }
            }
        }

        // Wait for the node task to complete
        if let Err(e) = node_handle.await {
            log::error!("P2P node task error: {}", e);
        }

        log::info!("P2P service shutdown complete");
        Ok(())
    }

    async fn handle_incoming_request_static<T>(
        message_queue_tx: &Sender<Message>,
        outbound_message_tx: &Sender<OutgoingMessage>,
        authenticated_peers: &Arc<RwLock<HashMap<String, String>>>,
        peer_id: PeerId,
        request_id: T,
        request: p2p::Request,
        channel: p2p::ResponseChannel,
    ) -> Result<()>
    where
        T: std::fmt::Debug,
    {
        log::debug!(
            "Received request from peer: {} (ID: {:?})",
            peer_id,
            request_id
        );

        match request {
            p2p::Request::General(p2p::GeneralRequest { data }) => {
                log::debug!(
                    "Processing GeneralRequest with {} bytes from {}",
                    data.len(),
                    peer_id
                );

                // Check if peer is authenticated
                let sender_address = {
                    let auth_peers = authenticated_peers.read().await;
                    match auth_peers.get(&peer_id.to_string()) {
                        Some(address) => address.clone(),
                        None => {
                            log::warn!("Rejecting message from unauthenticated peer: {}", peer_id);
                            // Send error response
                            let response = p2p::Response::General(p2p::GeneralResponse {
                                data: b"ERROR: Not authenticated".to_vec(),
                            });
                            outbound_message_tx
                                .send(response.into_outgoing_message(channel))
                                .await
                                .context("Failed to send error response")?;
                            return Ok(());
                        }
                    }
                };

                // Forward message to application
                let message = Message {
                    message_type: MessageType::General { data: data.clone() },
                    peer_id: peer_id.to_string(),
                    multiaddrs: vec![], // TODO: Extract multiaddrs from peer info
                    sender_address: Some(sender_address),
                    response_tx: None, // General messages don't need response channels
                };

                if let Err(e) = message_queue_tx.send(message).await {
                    log::error!("Failed to forward message to application: {}", e);
                }

                // Send acknowledgment
                let response = p2p::Response::General(p2p::GeneralResponse {
                    data: b"ACK".to_vec(),
                });

                outbound_message_tx
                    .send(response.into_outgoing_message(channel))
                    .await
                    .context("Failed to send acknowledgment")?;
            }
            p2p::Request::Authentication(auth_req) => {
                log::debug!("Processing Authentication request from {}", peer_id);

                match auth_req {
                    p2p::AuthenticationRequest::Initiation(init_req) => {
                        // Create a one-shot channel for the response
                        let (response_tx, response_rx) = tokio::sync::oneshot::channel();

                        // Forward to application for handling
                        let message = Message {
                            message_type: MessageType::AuthenticationInitiation {
                                challenge: init_req.message,
                            },
                            peer_id: peer_id.to_string(),
                            multiaddrs: vec![],
                            sender_address: None,
                            response_tx: Some(response_tx), // Pass the sender for response
                        };

                        if let Err(e) = message_queue_tx.send(message).await {
                            log::error!("Failed to forward auth initiation to application: {}", e);
                            return Ok(());
                        }

                        // Wait for the response from the message processor
                        match response_rx.await {
                            Ok(response) => {
                                outbound_message_tx
                                    .send(response.into_outgoing_message(channel))
                                    .await
                                    .context("Failed to send auth response")?;
                            }
                            Err(_) => {
                                log::error!("Response channel closed for auth initiation");
                            }
                        }
                    }
                    p2p::AuthenticationRequest::Solution(sol_req) => {
                        // Create a one-shot channel for the response
                        let (response_tx, response_rx) = tokio::sync::oneshot::channel();

                        // Forward to application for handling
                        let message = Message {
                            message_type: MessageType::AuthenticationSolution {
                                signature: sol_req.signature,
                            },
                            peer_id: peer_id.to_string(),
                            multiaddrs: vec![],
                            sender_address: None,
                            response_tx: Some(response_tx), // Pass the sender for response
                        };

                        if let Err(e) = message_queue_tx.send(message).await {
                            log::error!("Failed to forward auth solution to application: {}", e);
                            return Ok(());
                        }

                        // Wait for the response from the message processor
                        match response_rx.await {
                            Ok(response) => {
                                outbound_message_tx
                                    .send(response.into_outgoing_message(channel))
                                    .await
                                    .context("Failed to send auth solution response")?;
                            }
                            Err(_) => {
                                log::error!("Response channel closed for auth solution");
                            }
                        }
                    }
                }
            }
            _ => {
                log::warn!("Received unsupported request type: {:?}", request);
            }
        }

        Ok(())
    }

    async fn handle_incoming_response_static<T>(
        message_queue_tx: &Sender<Message>,
        peer_id: PeerId,
        request_id: T,
        response: p2p::Response,
    ) -> Result<()>
    where
        T: std::fmt::Debug,
    {
        log::debug!(
            "Received response from peer: {} (ID: {:?})",
            peer_id,
            request_id
        );

        match response {
            p2p::Response::General(p2p::GeneralResponse { data }) => {
                log::debug!("General response received: {} bytes", data.len());
            }
            p2p::Response::Authentication(auth_resp) => {
                log::debug!("Authentication response received from {}", peer_id);

                match auth_resp {
                    p2p::AuthenticationResponse::Initiation(init_resp) => {
                        // Forward to application
                        let message = Message {
                            message_type: MessageType::AuthenticationResponse {
                                challenge: init_resp.message,
                                signature: init_resp.signature,
                            },
                            peer_id: peer_id.to_string(),
                            multiaddrs: vec![],
                            sender_address: None,
                            response_tx: None,
                        };

                        if let Err(e) = message_queue_tx.send(message).await {
                            log::error!("Failed to forward auth response to application: {}", e);
                        }
                    }
                    p2p::AuthenticationResponse::Solution(sol_resp) => {
                        log::debug!("Auth solution response: {:?}", sol_resp);
                        // This is handled by the authentication flow
                    }
                }
            }
            _ => {
                log::debug!("Received other response type: {:?}", response);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_message_serialization() {
        let message = Message {
            message_type: MessageType::General {
                data: vec![1, 2, 3, 4],
            },
            peer_id: "12D3KooWExample".to_string(),
            multiaddrs: vec!["/ip4/127.0.0.1/tcp/4001".to_string()],
            sender_address: Some("0x1234567890123456789012345678901234567890".to_string()),
            response_tx: None,
        };

        let json = serde_json::to_string(&message).unwrap();
        let deserialized: Message = serde_json::from_str(&json).unwrap();

        match (&message.message_type, &deserialized.message_type) {
            (MessageType::General { data: data1 }, MessageType::General { data: data2 }) => {
                assert_eq!(data1, data2);
            }
            _ => panic!("Message types don't match"),
        }
        assert_eq!(message.peer_id, deserialized.peer_id);
        assert_eq!(message.multiaddrs, deserialized.multiaddrs);
        assert_eq!(message.sender_address, deserialized.sender_address);
    }
}
