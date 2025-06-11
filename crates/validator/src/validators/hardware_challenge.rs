use anyhow::{Context, Error, Result};
use log::{error, info};
use rand::{rng, Rng};
use reqwest::Client;
use shared::{
    models::{
        api::ApiResponse,
        challenge::{calc_matrix, ChallengeRequest, ChallengeResponse, FixedF64},
        node::DiscoveryNode,
    },
    security::request_signer::sign_request,
    web3::wallet::Wallet,
};

pub struct HardwareChallenge<'a> {
    wallet: &'a Wallet,
    client: Client,
}

impl<'a> HardwareChallenge<'a> {
    pub fn new(wallet: &'a Wallet) -> Self {
        Self {
            wallet,
            client: Client::new(),
        }
    }

    pub async fn challenge_node(
        &self,
        node: &DiscoveryNode,
        challenge_route: &str,
    ) -> Result<i32, Error> {
        let node_url = format!("http://{}:{}", node.node.ip_address, node.node.port);

        let mut headers = reqwest::header::HeaderMap::new();

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

        let post_url = format!("{}{}", node_url, challenge_route);

        let address = self.wallet.wallet.default_signer().address().to_string();
        let challenge_matrix_value = serde_json::to_value(&challenge_with_timestamp)?;
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
        let response = self
            .client
            .post(post_url)
            .headers(headers)
            .json(&challenge_matrix_value)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;

        let response_text = response.text().await?;
        let parsed_response: ApiResponse<ChallengeResponse> = serde_json::from_str(&response_text)
            .map_err(|e| {
                error!(
                    "Failed to parse JSON response from node {}: {}",
                    node.id, response_text
                );
                anyhow::anyhow!("Failed to parse response: {}", e)
            })?;

        if !parsed_response.success {
            error!("Challenge failed for node {}: {}", node.id, response_text);
            Err(anyhow::anyhow!(
                "Error fetching challenge from node: {}",
                response_text
            ))
        } else if challenge_expected.result == parsed_response.data.result {
            info!("Challenge for node {} successful", node.id);
            Ok(0)
        } else {
            error!(
                "Challenge failed for node {}: expected {:?}, got {:?}",
                node.id, challenge_expected.result, parsed_response.data.result
            );
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
            timestamp: None,
        }
    }
}
