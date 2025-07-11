use alloy::primitives::Address;
use anyhow::{bail, Context as _, Result};
use log::{error, info};
use rand::{rng, Rng};
use shared::models::node::DiscoveryNode;
use std::str::FromStr;

use crate::p2p::HardwareChallengeRequest;

pub(crate) struct HardwareChallenge {
    challenge_tx: tokio::sync::mpsc::Sender<HardwareChallengeRequest>,
}

impl HardwareChallenge {
    pub(crate) fn new(challenge_tx: tokio::sync::mpsc::Sender<HardwareChallengeRequest>) -> Self {
        Self { challenge_tx }
    }

    pub(crate) async fn challenge_node(&self, node: &DiscoveryNode) -> Result<()> {
        // Check if node has P2P ID and addresses
        let p2p_id = node
            .node
            .worker_p2p_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Node {} does not have P2P ID", node.id))?;

        let p2p_addresses = node
            .node
            .worker_p2p_addresses
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Node {} does not have P2P addresses", node.id))?;

        // create random challenge matrix
        let challenge_matrix = self.random_challenge(3, 3, 3, 3);
        let challenge_expected = p2p::calc_matrix(&challenge_matrix);

        // Add timestamp to the challenge
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut challenge_with_timestamp = challenge_matrix.clone();
        challenge_with_timestamp.timestamp = Some(current_time);

        let node_address = Address::from_str(&node.node.id)
            .map_err(|e| anyhow::anyhow!("Failed to parse node address {}: {}", node.node.id, e))?;

        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        let hardware_challenge = HardwareChallengeRequest {
            worker_wallet_address: node_address,
            worker_p2p_id: p2p_id,
            worker_addresses: p2p_addresses,
            challenge: challenge_with_timestamp,
            response_tx,
        };

        // Send challenge via P2P
        self.challenge_tx
            .send(hardware_challenge)
            .await
            .context("failed to send hardware challenge request to p2p service")?;

        let resp = response_rx
            .await
            .context("failed to receive response from node")?;

        if challenge_expected.result == resp.result {
            info!("Challenge for node {} successful", node.id);
        } else {
            error!(
                "Challenge failed for node {}: expected {:?}, got {:?}",
                node.id, challenge_expected.result, resp.result
            );
            bail!("Node failed challenge");
        }

        Ok(())
    }

    fn random_challenge(
        &self,
        rows_a: usize,
        cols_a: usize,
        rows_b: usize,
        cols_b: usize,
    ) -> p2p::ChallengeRequest {
        use p2p::FixedF64;

        let mut rng = rng();

        let data_a_vec: Vec<f64> = (0..(rows_a * cols_a))
            .map(|_| rng.random_range(0.0..1.0))
            .collect();

        let data_b_vec: Vec<f64> = (0..(rows_b * cols_b))
            .map(|_| rng.random_range(0.0..1.0))
            .collect();

        // convert to FixedF64
        let data_a: Vec<FixedF64> = data_a_vec.iter().map(|x| FixedF64(*x)).collect();
        let data_b: Vec<FixedF64> = data_b_vec.iter().map(|x| FixedF64(*x)).collect();

        p2p::ChallengeRequest {
            rows_a,
            cols_a,
            data_a,
            rows_b,
            cols_b,
            data_b,
            timestamp: None,
        }
    }
}
