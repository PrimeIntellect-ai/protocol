use alloy::primitives::Address;
use anyhow::{Context, Error, Result};
use log::{error, info};
use rand::{rng, Rng};
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{json, Value};
use shared::models::api::ApiResponse;
use shared::models::challenge::{calc_matrix, ChallengeRequest, ChallengeResponse, FixedF64};
use shared::models::gpu_challenge::*;
use shared::models::node::DiscoveryNode;
use shared::security::request_signer::sign_request;
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::wallet::Wallet;
use std::env;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct NodeChallengeState {
    pub session_id: Option<String>,
    pub timestamp: u64,
}

pub struct HardwareValidator<'a> {
    wallet: &'a Wallet,
    contracts: Arc<Contracts>,
    verifier_service_url: String,
    node_sessions: Arc<Mutex<std::collections::HashMap<String, NodeChallengeState>>>,
}

impl<'a> HardwareValidator<'a> {
    pub fn new(wallet: &'a Wallet, contracts: Arc<Contracts>) -> Self {
        let verifier_url = env::var("VERIFIER_SERVICE_URL")
            .unwrap_or_else(|_| "http://localhost:14141".to_string());

        Self {
            wallet,
            contracts,
            verifier_service_url: verifier_url,
            node_sessions: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }

    pub async fn validate_nodes(&self, nodes: Vec<DiscoveryNode>) -> Result<()> {
        //let non_validated_nodes: Vec<DiscoveryNode> = nodes
        //   .into_iter()
        //   .filter(|node| !node.is_validated)
        //   .collect();

        let non_validated_nodes = nodes;

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

            // First, validate with a simple matrix challenge
            let basic_challenge_result = self.basic_challenge_node(&node).await;
            if basic_challenge_result.is_err() {
                error!(
                    "Node {} failed basic challenge: {:?}",
                    node.id, basic_challenge_result
                );
                continue;
            }

            // skip if node is already being checked
            if self.node_sessions.lock().await.contains_key(&node.id) {
                info!("Node {} is already being checked", node.id);
                continue;
            }

            // Then perform the GPU challenge
            let gpu_challenge_result = self.gpu_challenge_node(&node).await;
            if gpu_challenge_result.is_err() {
                error!(
                    "Node {} failed GPU challenge: {:?}",
                    node.id, gpu_challenge_result
                );
                continue;
            }

            // If both challenges pass, validate the node on-chain
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

    async fn basic_challenge_node(&self, node: &DiscoveryNode) -> Result<(), Error> {
        let node_url = format!("http://{}:{}", node.node.ip_address, node.node.port);
        let challenge_route = "/challenge/submit";
        let mut headers = reqwest::header::HeaderMap::new();

        // Create random challenge matrix
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
            info!("Basic challenge successful");
            Ok(())
        } else {
            error!("Basic challenge failed");
            Err(anyhow::anyhow!("Node failed basic challenge"))
        }
    }

    async fn verifier_send<T: serde::de::DeserializeOwned>(
        &self,
        endpoint: &str,
        payload: Option<Value>,
    ) -> Result<T> {
        info!("Sending request to verifier service: {}", endpoint);
        let client = reqwest::Client::new();

        let mut request = client.post(format!("{}{}", self.verifier_service_url, endpoint));

        // Add signature and address headers
        let address = self.wallet.wallet.default_signer().address().to_string();
        let signature = sign_request(endpoint, self.wallet, payload.as_ref())
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        let mut headers = HeaderMap::new();
        headers.insert("x-address", HeaderValue::from_str(&address)?);
        headers.insert("x-signature", HeaderValue::from_str(&signature)?);
        request = request.headers(headers);

        if let Some(payload) = payload {
            request = request.json(&payload);
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            error!("Endpoint failure: {}, {}", endpoint, error_text);
            return Err(anyhow::anyhow!("Verifier request failed: {}", error_text));
        }

        response
            .json::<T>()
            .await
            .context("Failed to deserialize verifier response")
    }

    async fn worker_send<
        T: serde::de::DeserializeOwned + serde::Serialize + std::fmt::Debug,
        P: serde::Serialize,
    >(
        &self,
        node: &DiscoveryNode,
        endpoint: &str,
        payload_struct: Option<P>,
    ) -> Result<T> {
        info!("Sending request to worker node: {}", endpoint);
        let client = reqwest::Client::new();
        let node_url = format!("http://{}:{}", node.node.ip_address, node.node.port);

        let mut request = client.post(format!("{}{}", node_url, endpoint));

        let payload = payload_struct
            .map(|p| serde_json::to_value(p))
            .transpose()?;

        // Add signature and address headers
        let address = self.wallet.wallet.default_signer().address().to_string();
        let signature = sign_request(endpoint, self.wallet, payload.as_ref())
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        let mut headers = HeaderMap::new();
        headers.insert("x-address", HeaderValue::from_str(&address)?);
        headers.insert("x-signature", HeaderValue::from_str(&signature)?);
        request = request.headers(headers);

        if let Some(payload) = payload {
            request = request.json(&payload);
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            error!("Endpoint failure: {}, {}", endpoint, error_text);
            return Err(anyhow::anyhow!("Worker request failed: {}", error_text));
        }

        // Clone the response so we can read the body twice
        let response_text = response.text().await?;
        info!("Worker response body: {}", response_text);

        let parsed_response: ApiResponse<T> = serde_json::from_str(&response_text)?;

        info!("Parsed worker response: {:?}", parsed_response.data);

        // process this json into T struct
        Ok(parsed_response.data)
    }

    async fn worker_get(
        &self,
        node: &DiscoveryNode,
        endpoint: &str,
        payload_struct: Option<Value>,
    ) -> Result<String, Error> {
        info!("Sending request to worker node: {}", endpoint);
        let client = reqwest::Client::new();
        let node_url = format!("http://{}:{}", node.node.ip_address, node.node.port);

        let mut request = client.get(format!("{}{}", node_url, endpoint));

        let payload = payload_struct.map(serde_json::to_value).transpose()?;

        if let Some(payload) = payload {
            // For GET requests, add the payload as query parameters instead of JSON body
            let query_params =
                serde_json::from_value::<std::collections::HashMap<String, String>>(payload)
                    .map_err(|e| {
                        anyhow::anyhow!("Failed to convert payload to query params: {}", e)
                    })?;

            for (key, value) in query_params {
                request = request.query(&[(key, value)]);
            }
        }

        // get the built /endpoint?query=param URL
        let request_url = request
            .try_clone()
            .unwrap()
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build request URL: {}", e))?;
        let request_url_str = request_url.url().to_string();

        // Add signature and address headers
        let address = self.wallet.wallet.default_signer().address().to_string();
        let signature = sign_request(&request_url_str, self.wallet, None)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        let mut headers = HeaderMap::new();
        headers.insert("x-address", HeaderValue::from_str(&address)?);
        headers.insert("x-signature", HeaderValue::from_str(&signature)?);
        request = request.headers(headers);

        let response = request.send().await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            error!("Endpoint failure: {}, {}", endpoint, error_text);
            Err(anyhow::anyhow!("Worker request failed: {}", error_text))
        } else {
            let response_text = response.text().await?;
            info!("Worker response body: {}", response_text);
            Ok(response_text)
        }
    }

    async fn gpu_challenge_node(&self, node: &DiscoveryNode) -> Result<(), Error> {
        // check if node is already running a challenge session
        {
            // Create a separate scope for the lock
            let mut sessions = self.node_sessions.lock().await;

            if let Some(session) = sessions.get(&node.id) {
                let current_time = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                let session_age = current_time - session.timestamp;

                if session_age < 600 {
                    return Err(anyhow::anyhow!(
                        "Node is already running a challenge session"
                    ));
                } else {
                    info!("Removing expired session for node: {}", node.id);
                    sessions.remove(&node.id);
                }
            } else {
                info!("No session found for node, creating: {}", node.id);
                sessions.insert(
                    node.id.clone(),
                    NodeChallengeState {
                        session_id: None,
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs(),
                    },
                );
            }
        }

        // STEP 1: Initialize a new challenge with the verifier service
        let init_request = GpuChallengeInitRequest {
            n: 8192, // Default matrix size
        };

        let init_data: GpuChallengeResponse = self
            .verifier_send("/init", Some(json!(init_request)))
            .await
            .context("Failed to initialize GPU challenge")?;

        info!("Initialized verifier session: {}", init_data.session_id);

        // store session id in node_sessions
        let challenge_state = NodeChallengeState {
            session_id: Some(init_data.session_id.clone()),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };

        self.node_sessions
            .lock()
            .await
            .insert(node.id.clone(), challenge_state.clone());

        info!(
            "Starting challenge state: {}, {}",
            challenge_state.session_id.unwrap(),
            challenge_state.timestamp
        );

        // STEP 2: Start GPU challenge on worker node
        let start_route = "/gpu-challenge/init-container";

        let start_payload = GpuChallengeWorkerStart {
            session_id: init_data.session_id.clone(),
        };

        let response: GpuChallengeStatus = match self
            .worker_send(node, start_route, Some(start_payload))
            .await
        {
            Ok(status) => status,
            Err(e) => {
                error!("Failed to start GPU challenge on worker node: {}", e);
                return Err(anyhow::anyhow!("Failed to start GPU challenge: {}", e));
            }
        };

        info!("Worker response: {:?}", response);

        if response.status != "initializing" {
            return Err(anyhow::anyhow!(
                "Failed to start GPU challenge on worker node"
            ));
        }

        // STEP 3: Check status until worker is ready
        let status_route = "/gpu-challenge/status";

        let mut max_attempts = 100;

        loop {
            match self.worker_get(node, status_route, None).await {
                Ok(response_text) => {
                    let status_response: GpuChallengeStatus = serde_json::from_str(&response_text)?;
                    info!("Worker status: {}", status_response.status);
                    if status_response.status == "ready" {
                        info!("Worker node is ready for GPU challenge");
                        break;
                    }
                }
                Err(e) => {
                    error!("Failed to get worker status: {}", e);
                    return Err(anyhow::anyhow!("Failed to get worker status: {}", e));
                }
            }

            max_attempts -= 1;
            if max_attempts == 0 {
                return Err(anyhow::anyhow!(
                    "Worker node GPU container did not become ready in time"
                ));
            }

            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }

        // STEP 4: Send initial challenge parameters to worker, get commitment
        let compute_commitment_route = "/gpu-challenge/compute-commitment";

        let compute_commitment_payload = GpuChallengeWorkerComputeCommitment {
            session_id: init_data.session_id.clone(),
            master_seed: init_data.master_seed,
            n: init_data.n,
        };

        let commitment_response: GpuCommitmentResponse = self
            .worker_send(
                node,
                compute_commitment_route,
                Some(compute_commitment_payload),
            )
            .await?;

        // STEP 5: Send commitment to verifier
        let commitment_request = GpuCommitmentRequest {
            session_id: init_data.session_id.clone(),
            commitment_root: commitment_response.commitment_root,
        };

        let commitment_result: GpuChallengeVectorResponse = self
            .verifier_send("/commitment", Some(json!(commitment_request)))
            .await
            .context("Failed to submit commitment to verifier")?;

        // STEP 6: Compute C*r on worker
        let compute_cr_route = "/gpu-challenge/compute-cr";

        let compute_cr_payload = GpuChallengeWorkerComputeCR {
            session_id: init_data.session_id.clone(),
            r: commitment_result.challenge_vector,
        };

        let cr_response: GpuComputeCRResponse = self
            .worker_send(node, compute_cr_route, Some(compute_cr_payload))
            .await?;

        // STEP 6: Send Cr to verifier
        let cr_request = GpuRowChallengeRequest {
            session_id: init_data.session_id.clone(),
            Cr: cr_response.Cr,
        };

        let cr_result: GpuRowChallengeResponse = self
            .verifier_send("/row_challenge", Some(json!(cr_request)))
            .await
            .context("Failed to submit Cr to verifier")?;

        if !cr_result.freivalds_ok {
            return Err(anyhow::anyhow!("GPU challenge failed"));
        }

        // STEP 7: Get row proofs from worker
        let row_proofs_route = "/gpu-challenge/compute-row-proofs";

        let row_proofs_payload = GpuRowProofsRequest {
            row_idxs: cr_result.spot_rows,
        };

        let row_proofs_response: GpuChallengeWorkerComputeRowProofsResponse = self
            .worker_send(node, row_proofs_route, Some(row_proofs_payload))
            .await?;

        // STEP 8: Send row proofs to verifier
        let row_proofs_request = GpuMultiRowCheckRequest {
            session_id: init_data.session_id.clone(),
            rows: row_proofs_response.rows,
        };

        let row_proofs_result: GpuMultiRowCheckResponse = self
            .verifier_send("/multi_row_check", Some(json!(row_proofs_request)))
            .await
            .context("Failed to submit row proofs to verifier")?;

        // STEP 9: Check verifier response
        if row_proofs_result.all_passed {
            info!("GPU challenge successful");
            Ok(())
        } else {
            info!("GPU challenge: not all rows passed");
            // pass anyway if >= 50% of rows passed
            let total_rows = row_proofs_result.results.len();
            let total_passed = row_proofs_result.results.iter().filter(|r| r.pass).count();
            if total_passed as f64 / total_rows as f64 >= 0.5 {
                info!(
                    "GPU challenge passed with {} out of {} rows",
                    total_passed, total_rows
                );
                Ok(())
            } else {
                error!("GPU challenge failed");
                Err(anyhow::anyhow!("GPU challenge failed"))
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
        }
    }
}
