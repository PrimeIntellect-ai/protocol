use crate::error::{PrimeProtocolError, Result};
use crate::p2p_handler::Message;
use alloy::primitives::{Address, Signature};
use rand::Rng;
use shared::security::request_signer::sign_message;
use shared::web3::wallet::Wallet;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;

/*
 * Authentication Flow:
 *
 * This module implements a mutual authentication protocol between peers using cryptographic signatures.
 * The flow works as follows:
 *
 * 1. INITIATION: Peer A wants to send a message to Peer B but they're not authenticated
 *    - Peer A generates a random challenge (32 bytes, hex-encoded)
 *    - Peer A stores the challenge and queues the original message
 *    - Peer A sends AuthenticationInitiation{challenge} to Peer B
 *
 * 2. RESPONSE: Peer B receives the initiation
 *    - Peer B signs Peer A's challenge with their private key
 *    - Peer B generates their own challenge for Peer A
 *    - Peer B sends AuthenticationResponse{challenge: B's challenge, signature: signed A's challenge}
 *
 * 3. SOLUTION: Peer A receives the response
 *    - Peer A verifies Peer B's signature against the original challenge to get B's wallet address
 *    - Peer A signs Peer B's challenge with their private key
 *    - Peer A marks Peer B as authenticated
 *    - Peer A sends AuthenticationSolution{signature: signed B's challenge}
 *    - Peer A sends the originally queued message
 *
 * 4. COMPLETION: Peer B receives the solution
 *    - Peer B verifies Peer A's signature against their challenge to get A's wallet address
 *    - Peer B marks Peer A as authenticated
 *    - Both peers can now exchange messages freely
 *
 * Security Properties:
 * - Mutual authentication: Both peers prove they control their private keys
 * - Replay protection: Each challenge is randomly generated
 * - Address verification: Signature recovery provides cryptographic proof of wallet ownership
 */

/// Represents an ongoing authentication challenge with a peer
#[derive(Debug)]
pub struct OngoingAuthChallenge {
    pub peer_wallet_address: Address,
    pub auth_challenge_request_message: String,
    pub outgoing_message: Message,
    pub their_challenge: Option<String>, // The challenge we received from them that we need to sign
}

/// Manages authentication state and operations
pub struct AuthenticationManager {
    /// Track ongoing authentication requests (when we initiate)
    ongoing_auth_requests: Arc<RwLock<HashMap<String, OngoingAuthChallenge>>>,
    /// Track outgoing challenges (when they initiate and we respond)
    outgoing_challenges: Arc<RwLock<HashMap<String, String>>>,
    /// Track peers we're responding to (to prevent initiating auth with them)
    responding_to_peers: Arc<RwLock<HashSet<String>>>,
    /// Queue messages for peers we're authenticating with as responder
    responder_message_queue: Arc<RwLock<HashMap<String, Vec<Message>>>>,
    /// Track authenticated peers
    authenticated_peers: Arc<RwLock<HashSet<String>>>,
    /// Our wallet for signing
    node_wallet: Arc<Wallet>,
}

impl AuthenticationManager {
    pub fn new(node_wallet: Arc<Wallet>) -> Self {
        Self {
            ongoing_auth_requests: Arc::new(RwLock::new(HashMap::new())),
            outgoing_challenges: Arc::new(RwLock::new(HashMap::new())),
            responding_to_peers: Arc::new(RwLock::new(HashSet::new())),
            responder_message_queue: Arc::new(RwLock::new(HashMap::new())),
            authenticated_peers: Arc::new(RwLock::new(HashSet::new())),
            node_wallet,
        }
    }

    /// Check if a peer is already authenticated
    pub async fn is_authenticated(&self, peer_id: &str) -> bool {
        self.authenticated_peers.read().await.contains(peer_id)
    }

    /// Mark a peer as authenticated
    pub async fn mark_authenticated(&self, peer_id: String) {
        self.authenticated_peers.write().await.insert(peer_id);
    }

    /// Check our role in ongoing authentication
    pub async fn get_auth_role(&self, peer_id: &str) -> Option<String> {
        if self
            .ongoing_auth_requests
            .read()
            .await
            .contains_key(peer_id)
        {
            Some("initiator".to_string())
        } else if self.responding_to_peers.read().await.contains(peer_id) {
            Some("responder".to_string())
        } else {
            None
        }
    }

    /// Queue a message for a peer we're responding to
    pub async fn queue_message_as_responder(
        &self,
        peer_id: String,
        message: Message,
    ) -> Result<()> {
        let mut queue = self.responder_message_queue.write().await;
        queue.entry(peer_id).or_insert_with(Vec::new).push(message);
        Ok(())
    }

    /// Start authentication with a peer
    pub async fn start_authentication(
        &self,
        peer_id: String,
        outgoing_message: Message,
    ) -> Result<String> {
        // Generate authentication challenge
        let challenge_bytes: [u8; 32] = rand::thread_rng().gen();
        let auth_challenge_message = hex::encode(challenge_bytes);

        // Store the ongoing auth challenge
        let mut ongoing_auth = self.ongoing_auth_requests.write().await;
        ongoing_auth.insert(
            peer_id,
            OngoingAuthChallenge {
                peer_wallet_address: Address::ZERO, // Will be updated when we get their signature
                auth_challenge_request_message: auth_challenge_message.clone(),
                outgoing_message,
                their_challenge: None,
            },
        );

        Ok(auth_challenge_message)
    }

    /// Handle incoming authentication initiation
    pub async fn handle_auth_initiation(
        &self,
        peer_id: &str,
        challenge: &str,
    ) -> Result<(String, String)> {
        // Mark that we're responding to this peer
        self.responding_to_peers
            .write()
            .await
            .insert(peer_id.to_string());

        // Sign the challenge
        let signature = sign_message(challenge, &self.node_wallet)
            .await
            .map_err(|e| {
                PrimeProtocolError::BlockchainError(format!("Failed to sign challenge: {}", e))
            })?;

        // Generate our own challenge for the peer
        let our_challenge_bytes: [u8; 32] = rand::thread_rng().gen();
        let our_challenge = hex::encode(our_challenge_bytes);

        // Store the challenge we're sending so we can verify the signature later
        self.outgoing_challenges
            .write()
            .await
            .insert(peer_id.to_string(), our_challenge.clone());

        Ok((our_challenge, signature))
    }

    /// Handle authentication response from peer
    pub async fn handle_auth_response(
        &self,
        peer_id: &str,
        their_challenge: &str,
        their_signature: &str,
    ) -> Result<(String, Option<Message>)> {
        // Verify we have an ongoing auth request for this peer
        let mut ongoing_auth = self.ongoing_auth_requests.write().await;
        let auth_challenge = ongoing_auth.get_mut(peer_id).ok_or_else(|| {
            PrimeProtocolError::InvalidConfig(format!(
                "No ongoing auth request for peer {}",
                peer_id
            ))
        })?;

        // Verify their signature to get their address
        let parsed_signature = Signature::from_str(their_signature).map_err(|e| {
            PrimeProtocolError::InvalidConfig(format!("Invalid signature format: {}", e))
        })?;

        // Recover the peer's address from their signature
        let recovered_address = parsed_signature
            .recover_address_from_msg(&auth_challenge.auth_challenge_request_message)
            .map_err(|e| {
                PrimeProtocolError::InvalidConfig(format!("Failed to recover address: {}", e))
            })?;

        // Update the peer's wallet address and store their challenge
        auth_challenge.peer_wallet_address = recovered_address;
        auth_challenge.their_challenge = Some(their_challenge.to_string());
        log::debug!("Recovered peer address: {}", recovered_address);

        // Sign their challenge
        let our_signature = sign_message(their_challenge, &self.node_wallet)
            .await
            .map_err(|e| {
                PrimeProtocolError::BlockchainError(format!("Failed to sign challenge: {}", e))
            })?;

        // Mark peer as authenticated
        self.mark_authenticated(peer_id.to_string()).await;

        // Get the queued message to send after auth
        let queued_message = ongoing_auth
            .remove(peer_id)
            .map(|auth| auth.outgoing_message);

        Ok((our_signature, queued_message))
    }

    /// Handle authentication solution from peer
    pub async fn handle_auth_solution(
        &self,
        peer_id: &str,
        signature: &str,
    ) -> Result<(Address, Vec<Message>)> {
        // Get the challenge we sent to this peer
        let mut outgoing_challenges = self.outgoing_challenges.write().await;
        let challenge = outgoing_challenges.remove(peer_id).ok_or_else(|| {
            PrimeProtocolError::InvalidConfig(format!(
                "No outgoing challenge found for peer {}",
                peer_id
            ))
        })?;

        // Parse and verify the signature
        let parsed_signature = Signature::from_str(signature).map_err(|e| {
            PrimeProtocolError::InvalidConfig(format!("Invalid signature format: {}", e))
        })?;

        // Recover the peer's address from their signature of our challenge
        let recovered_address = parsed_signature
            .recover_address_from_msg(&challenge)
            .map_err(|e| {
                PrimeProtocolError::InvalidConfig(format!("Failed to recover address: {}", e))
            })?;

        log::debug!(
            "Verified auth solution from peer {} with address {}",
            peer_id,
            recovered_address
        );
        self.mark_authenticated(peer_id.to_string()).await;

        // Clean up responding state
        self.responding_to_peers.write().await.remove(peer_id);

        // Get any queued messages to send now that we're authenticated
        let queued_messages = self
            .responder_message_queue
            .write()
            .await
            .remove(peer_id)
            .unwrap_or_default();

        Ok((recovered_address, queued_messages))
    }

    /// Get wallet address
    pub fn wallet_address(&self) -> String {
        self.node_wallet
            .wallet
            .default_signer()
            .address()
            .to_string()
    }
}
