use alloy::primitives::Address;
use anyhow::Result;
use log::info;
use rand_v8::Rng;
use shared::models::challenge::{ChallengeRequest, ChallengeResponse};
use shared::p2p::{client::P2PClient as SharedP2PClient, messages::P2PMessage};
use shared::web3::wallet::Wallet;
use std::time::SystemTime;

pub struct P2PClient {
    shared_client: SharedP2PClient,
}

impl P2PClient {
    pub async fn new(wallet: Wallet) -> Result<Self> {
        let shared_client = SharedP2PClient::new(wallet).await?;
        Ok(Self { shared_client })
    }

    pub async fn ping_worker(
        &self,
        worker_wallet_address: Address,
        worker_p2p_id: &str,
        worker_addresses: &[String],
    ) -> Result<u64> {
        let nonce = rand_v8::thread_rng().gen::<u64>();

        let response = self
            .shared_client
            .send_request(
                worker_p2p_id,
                worker_addresses,
                worker_wallet_address,
                P2PMessage::Ping {
                    timestamp: SystemTime::now(),
                    nonce,
                },
                10,
            )
            .await?;

        match response {
            P2PMessage::Pong {
                nonce: returned_nonce,
                ..
            } => {
                if returned_nonce == nonce {
                    info!("Received valid pong from worker {worker_p2p_id} with nonce: {nonce}");
                    Ok(nonce)
                } else {
                    Err(anyhow::anyhow!("Invalid nonce in pong response"))
                }
            }
            _ => Err(anyhow::anyhow!("Unexpected response type for ping")),
        }
    }

    pub async fn send_hardware_challenge(
        &self,
        worker_wallet_address: Address,
        worker_p2p_id: &str,
        worker_addresses: &[String],
        challenge: ChallengeRequest,
    ) -> Result<ChallengeResponse> {
        let response = self
            .shared_client
            .send_request(
                worker_p2p_id,
                worker_addresses,
                worker_wallet_address,
                P2PMessage::HardwareChallenge {
                    challenge,
                    timestamp: SystemTime::now(),
                },
                30,
            )
            .await?;

        match response {
            P2PMessage::HardwareChallengeResponse { response, .. } => {
                info!("Received hardware challenge response from worker {worker_p2p_id}");
                Ok(response)
            }
            _ => Err(anyhow::anyhow!(
                "Unexpected response type for hardware challenge"
            )),
        }
    }
}
