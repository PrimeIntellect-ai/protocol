use crate::error::{PrimeProtocolError, Result};
use crate::p2p_handler::auth::AuthenticationManager;
use crate::p2p_handler::{Message, MessageType};
use std::sync::Arc;
use tokio::sync::mpsc::Sender;
use tokio::sync::Mutex;

/// Shared send_message function that handles authentication and message sending
pub async fn send_message_with_auth(
    message: Message,
    auth_manager: &Arc<AuthenticationManager>,
    outbound_tx: &Arc<Mutex<Sender<Message>>>,
) -> Result<()> {
    log::debug!("Sending message to peer: {}", message.peer_id);

    // Check if we're already authenticated with this peer
    if auth_manager.is_authenticated(&message.peer_id).await {
        log::debug!(
            "Already authenticated with peer {}, sending message directly",
            message.peer_id
        );
        return outbound_tx.lock().await.send(message).await.map_err(|e| {
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
        is_sender_validator: false,
        is_sender_pool_owner: false,
        response_tx: None,
    };

    outbound_tx
        .lock()
        .await
        .send(auth_message)
        .await
        .map_err(|e| {
            PrimeProtocolError::InvalidConfig(format!("Failed to send auth message: {}", e))
        })
}
