use alloy::primitives::Address;
use anyhow::Result;
use log::{info, warn};
use shared::models::invite::InviteRequest;
use shared::p2p::{client::P2PClient as SharedP2PClient, messages::P2PMessage};
use shared::web3::wallet::Wallet;

pub struct P2PClient {
    shared_client: SharedP2PClient,
}

impl P2PClient {
    pub async fn new(wallet: Wallet) -> Result<Self> {
        let shared_client = SharedP2PClient::new(wallet).await?;
        Ok(Self { shared_client })
    }

    pub async fn invite_worker(
        &self,
        worker_wallet_address: Address,
        worker_p2p_id: &str,
        worker_addresses: &[String],
        invite: InviteRequest,
    ) -> Result<()> {
        let response = self
            .shared_client
            .send_request(
                worker_p2p_id,
                worker_addresses,
                worker_wallet_address,
                P2PMessage::Invite(invite),
                20,
            )
            .await?;

        match response {
            P2PMessage::InviteResponse { status, error } => {
                if status == "ok" {
                    info!("Successfully invited worker {}", worker_p2p_id);
                    Ok(())
                } else {
                    let error_msg = error.unwrap_or_else(|| "Unknown error".to_string());
                    warn!("Failed to invite worker {}: {}", worker_p2p_id, error_msg);
                    Err(anyhow::anyhow!("Invite failed: {}", error_msg))
                }
            }
            _ => Err(anyhow::anyhow!("Unexpected response type for invite")),
        }
    }

    pub async fn get_task_logs(
        &self,
        worker_wallet_address: Address,
        worker_p2p_id: &str,
        worker_addresses: &[String],
    ) -> Result<Vec<String>> {
        let response = self
            .shared_client
            .send_request(
                worker_p2p_id,
                worker_addresses,
                worker_wallet_address,
                P2PMessage::GetTaskLogs,
                20,
            )
            .await?;

        match response {
            P2PMessage::GetTaskLogsResponse { logs } => {
                logs.map_err(|e| anyhow::anyhow!("Failed to get task logs: {}", e))
            }
            _ => Err(anyhow::anyhow!(
                "Unexpected response type for get_task_logs"
            )),
        }
    }

    pub async fn restart_task(
        &self,
        worker_wallet_address: Address,
        worker_p2p_id: &str,
        worker_addresses: &[String],
    ) -> Result<()> {
        let response = self
            .shared_client
            .send_request(
                worker_p2p_id,
                worker_addresses,
                worker_wallet_address,
                P2PMessage::RestartTask,
                25,
            )
            .await?;

        match response {
            P2PMessage::RestartTaskResponse { result } => {
                result.map_err(|e| anyhow::anyhow!("Failed to restart task: {}", e))
            }
            _ => Err(anyhow::anyhow!("Unexpected response type for restart_task")),
        }
    }
}
