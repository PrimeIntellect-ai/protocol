use crate::p2p::client::P2PClient;
use alloy::primitives::Address;
use anyhow::{Error, Result};
use log::{error, info};
use rand::{rng, Rng};
use shared::models::{
    challenge::{calc_matrix, ChallengeRequest, FixedF64},
    node::DiscoveryNode,
};
use std::str::FromStr;

pub(crate) struct HardwareChallenge<'a> {
    p2p_client: &'a P2PClient,
}

impl<'a> HardwareChallenge<'a> {
    pub(crate) fn new(p2p_client: &'a P2PClient) -> Self {
        Self { p2p_client }
    }

    pub(crate) async fn challenge_node(&self, node: &DiscoveryNode) -> Result<i32, Error> {
        // Check if node has P2P ID and addresses
        let p2p_id = node
            .node
            .worker_p2p_id
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Node {} does not have P2P ID", node.id))?;

        let p2p_addresses = node
            .node
            .worker_p2p_addresses
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Node {} does not have P2P addresses", node.id))?;

        // create random challenge matrix
        let challenge_matrix = self.random_challenge(3, 3, 3, 3);
        let challenge_expected = calc_matrix(&challenge_matrix);

        // Add timestamp to the challenge
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut challenge_with_timestamp = challenge_matrix.clone();
        challenge_with_timestamp.timestamp = Some(current_time);

        let node_address = Address::from_str(&node.node.id)
            .map_err(|e| anyhow::anyhow!("Failed to parse node address {}: {}", node.node.id, e))?;

        // Send challenge via P2P
        match self
            .p2p_client
            .send_hardware_challenge(
                node_address,
                p2p_id,
                p2p_addresses,
                challenge_with_timestamp,
            )
            .await
        {
            Ok(response) => {
                if challenge_expected.result == response.result {
                    info!("Challenge for node {} successful", node.id);
                    Ok(0)
                } else {
                    error!(
                        "Challenge failed for node {}: expected {:?}, got {:?}",
                        node.id, challenge_expected.result, response.result
                    );
                    Err(anyhow::anyhow!("Node failed challenge"))
                }
            }
            Err(e) => {
                error!("Failed to send challenge to node {}: {}", node.id, e);
                Err(anyhow::anyhow!("Failed to send challenge: {}", e))
            }
        }
    }

    fn random_challenge(
        &self,
        rows_a: usize,
        cols_a: usize,
        rows_b: usize,
        cols_b: usize,
    ) -> ChallengeRequest {
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

        ChallengeRequest {
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
