use anyhow::Result;
use iroh::{Endpoint, NodeAddr, NodeId, SecretKey};
use log::debug;
use shared::models::invite::InviteRequest;
use shared::p2p::{P2PMessage, P2PRequest, P2PResponse, PRIME_P2P_PROTOCOL};
use std::str::FromStr;

pub struct P2PClient {
    endpoint: Endpoint,
}

impl P2PClient {
    /// Create a new P2P client for the orchestrator
    pub async fn new() -> Result<Self> {
        let mut rng = rand_v8::thread_rng();
        let secret_key = SecretKey::generate(&mut rng);

        let endpoint = Endpoint::builder()
            .secret_key(secret_key)
            .alpns(vec![PRIME_P2P_PROTOCOL.to_vec()])
            .bind()
            .await?;

        Ok(Self { endpoint })
    }

    async fn send_request(
        &self,
        worker_p2p_id: &str,
        worker_addresses: &[String],
        message: P2PMessage,
    ) -> Result<P2PMessage> {
        let node_id = NodeId::from_str(worker_p2p_id)?;
        let mut socket_addrs = Vec::new();
        for addr in worker_addresses {
            if let Ok(socket_addr) = addr.parse() {
                socket_addrs.push(socket_addr);
            }
        }
        let node_addr = NodeAddr::new(node_id).with_direct_addresses(socket_addrs);

        debug!("Connecting to worker P2P node: {}", worker_p2p_id);
        let connection = self.endpoint.connect(node_addr, PRIME_P2P_PROTOCOL).await?;
        let (mut send, mut recv) = connection.open_bi().await?;

        let request = P2PRequest::new(message);
        let request_bytes = serde_json::to_vec(&request)?;
        send.write_all(&(request_bytes.len() as u32).to_be_bytes())
            .await?;
        send.write_all(&request_bytes).await?;
        send.finish()?;

        let mut response_len_bytes = [0u8; 4];
        recv.read_exact(&mut response_len_bytes).await?;
        let response_len = u32::from_be_bytes(response_len_bytes) as usize;

        let mut response_bytes = vec![0u8; response_len];
        recv.read_exact(&mut response_bytes).await?;

        let response: P2PResponse = serde_json::from_slice(&response_bytes)?;

        connection.close(0u32.into(), b"request complete");
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        Ok(response.message)
    }

    /// Invite a worker to join a compute pool
    pub async fn invite_worker(
        &self,
        worker_p2p_id: &str,
        worker_addresses: &[String],
        invite: InviteRequest,
    ) -> Result<()> {
        let response = self
            .send_request(worker_p2p_id, worker_addresses, P2PMessage::Invite(invite))
            .await?;

        match response {
            P2PMessage::InviteResponse { status, error } => {
                if status == "ok" {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!(
                        error.unwrap_or_else(|| "Unknown error".to_string())
                    ))
                }
            }
            _ => Err(anyhow::anyhow!("Unexpected response type for invite")),
        }
    }

    /// Get task logs from a worker
    pub async fn get_task_logs(
        &self,
        worker_p2p_id: &str,
        worker_addresses: &[String],
    ) -> Result<Vec<String>> {
        let response = self
            .send_request(worker_p2p_id, worker_addresses, P2PMessage::GetTaskLogs)
            .await?;

        match response {
            P2PMessage::GetTaskLogsResponse { logs } => logs.map_err(|e| anyhow::anyhow!(e)),
            _ => Err(anyhow::anyhow!(
                "Unexpected response type for get_task_logs"
            )),
        }
    }

    /// Restart a task on a worker
    pub async fn restart_task(
        &self,
        worker_p2p_id: &str,
        worker_addresses: &[String],
    ) -> Result<()> {
        let response = self
            .send_request(worker_p2p_id, worker_addresses, P2PMessage::RestartTask)
            .await?;

        match response {
            P2PMessage::RestartTaskResponse { result } => result.map_err(|e| anyhow::anyhow!(e)),
            _ => Err(anyhow::anyhow!("Unexpected response type for restart_task")),
        }
    }
}
