use alloy::primitives::Address;
use anyhow::Result;
use iroh::endpoint::{RecvStream, SendStream};
use iroh::{Endpoint, NodeAddr, NodeId, SecretKey};
use log::{debug, info};
use std::str::FromStr;
use std::time::Duration;

use crate::p2p::messages::{P2PMessage, P2PRequest, P2PResponse};
use crate::p2p::protocol::PRIME_P2P_PROTOCOL;
use crate::security::request_signer::sign_message;
use crate::web3::wallet::Wallet;
use rand_v8::rngs::OsRng;
use rand_v8::Rng;

pub struct P2PClient {
    endpoint: Endpoint,
    node_id: NodeId,
    wallet: Wallet,
}

impl P2PClient {
    pub async fn new(wallet: Wallet) -> Result<Self> {
        let mut rng = rand_v8::thread_rng();
        let secret_key = SecretKey::generate(&mut rng);
        let node_id = secret_key.public();

        let endpoint = Endpoint::builder()
            .secret_key(secret_key)
            .alpns(vec![PRIME_P2P_PROTOCOL.to_vec()])
            .discovery_n0()
            .bind()
            .await?;

        info!("P2P client initialized with node ID: {}", node_id);

        Ok(Self {
            endpoint,
            node_id,
            wallet,
        })
    }

    pub fn node_id(&self) -> NodeId {
        self.node_id
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Helper function to write a message with length prefix
    async fn write_message<T: serde::Serialize>(send: &mut SendStream, message: &T) -> Result<()> {
        let message_bytes = serde_json::to_vec(message)?;
        send.write_all(&(message_bytes.len() as u32).to_be_bytes())
            .await?;
        send.write_all(&message_bytes).await?;
        Ok(())
    }

    /// Helper function to read a message with length prefix
    async fn read_message<T: serde::de::DeserializeOwned>(recv: &mut RecvStream) -> Result<T> {
        let mut len_bytes = [0u8; 4];
        recv.read_exact(&mut len_bytes).await?;
        let len = u32::from_be_bytes(len_bytes) as usize;

        let mut message_bytes = vec![0u8; len];
        recv.read_exact(&mut message_bytes).await?;

        let message: T = serde_json::from_slice(&message_bytes)?;
        Ok(message)
    }

    pub async fn send_request(
        &self,
        target_p2p_id: &str,
        target_addresses: &[String],
        target_wallet_address: Address,
        message: P2PMessage,
        timeout_secs: u64,
    ) -> Result<P2PMessage> {
        let timeout_duration = Duration::from_secs(timeout_secs);

        tokio::time::timeout(timeout_duration, async {
            self.send_request_inner(
                target_p2p_id,
                target_addresses,
                target_wallet_address,
                message,
            )
            .await
        })
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "P2P request to {} timed out after {}s",
                target_p2p_id,
                timeout_secs
            )
        })?
    }

    async fn send_request_inner(
        &self,
        target_p2p_id: &str,
        target_addresses: &[String],
        target_wallet_address: Address,
        message: P2PMessage,
    ) -> Result<P2PMessage> {
        // Parse target node ID
        let node_id = NodeId::from_str(target_p2p_id)?;

        let mut socket_addrs = Vec::new();
        for addr in target_addresses {
            if let Ok(socket_addr) = addr.parse() {
                socket_addrs.push(socket_addr);
            }
        }

        if socket_addrs.is_empty() {
            return Err(anyhow::anyhow!(
                "No valid addresses provided for target node"
            ));
        }

        // Create node address
        let node_addr = NodeAddr::new(node_id).with_direct_addresses(socket_addrs);

        debug!("Connecting to P2P node: {}", target_p2p_id);

        // Connect to the target node
        let connection = self.endpoint.connect(node_addr, PRIME_P2P_PROTOCOL).await?;

        let (mut send, mut recv) = connection.open_bi().await?;

        // First request an auth challenge
        let challenge_bytes: [u8; 32] = OsRng.gen();
        let challenge_message: String = hex::encode(challenge_bytes);

        let request_auth_challenge = P2PRequest::new(P2PMessage::RequestAuthChallenge {
            message: challenge_message.clone(),
        });
        Self::write_message(&mut send, &request_auth_challenge).await?;

        // Response contains the auth challenge we have to solve (to show we are the right node)
        let auth_challenge_response: P2PResponse = Self::read_message(&mut recv).await?;
        let auth_challenge_solution: P2PRequest = match auth_challenge_response.message {
            P2PMessage::AuthChallenge {
                signed_message,
                message,
            } => {
                // Parse the signature from the server
                let parsed_signature =
                    if let Ok(sig) = alloy::primitives::Signature::from_str(&signed_message) {
                        sig
                    } else {
                        return Err(anyhow::anyhow!("Failed to parse signature from server"));
                    };

                // Recover address from the challenge message that the server signed
                let recovered_address = if let Ok(addr) =
                    parsed_signature.recover_address_from_msg(&challenge_message)
                {
                    addr
                } else {
                    return Err(anyhow::anyhow!(
                        "Failed to recover address from server signature"
                    ));
                };

                // Verify the recovered address matches the expected target wallet address
                if recovered_address != target_wallet_address {
                    return Err(anyhow::anyhow!(
                        "Server address verification failed: expected {}, got {}",
                        target_wallet_address,
                        recovered_address
                    ));
                }

                debug!("Auth challenge received from node: {}", target_p2p_id);
                let signature = sign_message(&message, &self.wallet).await.unwrap();
                P2PRequest::new(P2PMessage::AuthSolution {
                    signed_message: signature,
                })
            }
            _ => {
                return Err(anyhow::anyhow!(
                    "Expected auth challenge, got different message type"
                ));
            }
        };
        Self::write_message(&mut send, &auth_challenge_solution).await?;

        // Check if we are granted or rejected
        let auth_response: P2PResponse = Self::read_message(&mut recv).await?;
        match auth_response.message {
            P2PMessage::AuthGranted { .. } => {
                debug!("Auth granted with node: {}", target_p2p_id);
            }
            P2PMessage::AuthRejected { .. } => {
                debug!("Auth rejected with node: {}", target_p2p_id);
                return Err(anyhow::anyhow!(
                    "Auth rejected with node: {}",
                    target_p2p_id
                ));
            }
            _ => {
                return Err(anyhow::anyhow!(
                    "Expected auth response, got different message type"
                ));
            }
        }

        // Now send the actual request
        let request = P2PRequest::new(message);
        Self::write_message(&mut send, &request).await?;
        send.finish()?;

        // Read response
        let response: P2PResponse = Self::read_message(&mut recv).await?;

        // Close connection
        connection.close(0u32.into(), b"request complete");
        tokio::time::sleep(Duration::from_millis(50)).await;

        Ok(response.message)
    }

    /// Shutdown the P2P client gracefully
    pub async fn shutdown(self) -> Result<()> {
        info!("Shutting down P2P client with node ID: {}", self.node_id);
        self.endpoint.close().await;
        Ok(())
    }
}

impl Drop for P2PClient {
    fn drop(&mut self) {
        debug!("P2P client dropped for node ID: {}", self.node_id);
    }
}
