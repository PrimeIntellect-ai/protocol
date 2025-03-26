use alloy::primitives::Address;
use anyhow::{Context, Error, Result};
use log::{error, info};
use rand::{rng, Rng};
use shared::models::api::ApiResponse;
use shared::models::challenge::{calc_matrix, ChallengeRequest, ChallengeResponse, FixedF64};
use shared::models::node::DiscoveryNode;
use shared::security::request_signer::sign_request;
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::wallet::Wallet;
use std::sync::Arc;

pub struct HardwareValidator<'a> {
    wallet: &'a Wallet,
    contracts: Arc<Contracts>,
}

impl<'a> HardwareValidator<'a> {
    pub fn new(wallet: &'a Wallet, contracts: Arc<Contracts>) -> Self {
        Self { wallet, contracts }
    }

    pub async fn validate_nodes(&self, nodes: Vec<DiscoveryNode>) -> Result<()> {
        let non_validated_nodes: Vec<DiscoveryNode> = nodes
            .into_iter()
            .filter(|node| !node.is_validated)
            .collect();

        log::debug!("Non validated nodes: {:?}", non_validated_nodes);

        for node in non_validated_nodes {
            let node_address = match node.id.trim_start_matches("0x").parse::<Address>() {
                Ok(addr) => addr,
                Err(e) => {
                    error!("Failed to parse node address {}: {}", node.id, e);
                    continue;
                }
            };

            let provider_address = match node
                .provider_address
                .trim_start_matches("0x")
                .parse::<Address>()
            {
                Ok(addr) => addr,
                Err(e) => {
                    error!(
                        "Failed to parse provider address {}: {}",
                        node.provider_address, e
                    );
                    continue;
                }
            };

            let challenge_route = "/challenge/submit";
            let challenge_result = self.challenge_node(&node, challenge_route).await;
            if challenge_result.is_err() {
                error!(
                    "Failed to challenge node {}: {:?}",
                    node.id, challenge_result
                );
                continue;
            }

            if let Err(e) = self
                .contracts
                .prime_network
                .validate_node(provider_address, node_address)
                .await
            {
                error!("Failed to validate node {}: {}", node.id, e);
            } else {
                info!("Successfully validated node: {}", node.id);
            }
        }

        Ok(())
    }

    async fn challenge_node(
        &self,
        node: &DiscoveryNode,
        challenge_route: &str,
    ) -> Result<i32, Error> {
        let node_url = format!("http://{}:{}", node.node.ip_address, node.node.port);

        let mut headers = reqwest::header::HeaderMap::new();

        // create random challenge matrix
        let challenge_matrix = self.random_challenge(3, 3, 3, 3);
        let challenge_expected = calc_matrix(&challenge_matrix);

        let post_url = format!("{}{}", node_url, challenge_route);

        let address = self.wallet.wallet.default_signer().address().to_string();
        let challenge_matrix_value = serde_json::to_value(&challenge_matrix)?;
        let signature = sign_request(challenge_route, self.wallet, Some(&challenge_matrix_value))
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        headers.insert(
            "x-address",
            reqwest::header::HeaderValue::from_str(&address)
                .context("Failed to create address header")?,
        );
        headers.insert(
            "x-signature",
            reqwest::header::HeaderValue::from_str(&signature)
                .context("Failed to create signature header")?,
        );

        let response = reqwest::Client::new()
            .post(post_url)
            .headers(headers)
            .json(&challenge_matrix_value)
            .send()
            .await?;

        let response_text = response.text().await?;
        let parsed_response: ApiResponse<ChallengeResponse> = serde_json::from_str(&response_text)?;

        if !parsed_response.success {
            Err(anyhow::anyhow!("Error fetching challenge from node"))
        } else if challenge_expected.result == parsed_response.data.result {
            info!("Challenge successful");
            Ok(0)
        } else {
            error!("Challenge failed");
            Err(anyhow::anyhow!("Node failed challenge"))
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
        }
    }
}
