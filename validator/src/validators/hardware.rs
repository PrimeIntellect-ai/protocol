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

const VALIDATION_TIMEOUT: u64 = 600;
const ERROR_TIME_BUFFER: u64 = 30;
const MATRIX_CHALLENGE_SIZE_DEFAULT: u64 = 8192;
const MAX_CHALLENGE_ATTEMPTS: u64 = 3;

#[derive(Debug, Clone, PartialEq)]
pub enum NodeChallengeStatus {
    Init,
    Running,
    Completed,
    Failed,
    Blacklisted,
}

#[derive(Debug, Clone)]
pub struct NodeChallengeState {
    pub session_id: Option<String>,
    pub timestamp: u64,
    pub commitment_time: u64,
    pub status: NodeChallengeStatus,
    pub attempts: u64,
}

fn get_time_as_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub struct HardwareReqs {
    pub memory: u32,
    pub count: u32,
    pub benchmark: u64,
}

pub struct HardwareValidator<'a> {
    wallet: &'a Wallet,
    contracts: Arc<Contracts>,
    verifier_service_url: String,
    node_sessions: Arc<Mutex<std::collections::HashMap<String, NodeChallengeState>>>,
    reqs: HardwareReqs,
}

impl<'a> HardwareValidator<'a> {
    pub fn new(wallet: &'a Wallet, contracts: Arc<Contracts>) -> Self {
        let verifier_url = env::var("GPU_VERIFIER_SERVICE_URL")
            .unwrap_or_else(|_| "http://localhost:14141".to_string());

        let benchmark: u64 = std::env::var("GPU_VALIDATOR_BENCHMARK_TARGET")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(150);

        let memory: u32 = std::env::var("GPU_VALIDATOR_MIN_VRAM_MB")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(80000);

        let count: u32 = std::env::var("GPU_VALIDATOR_MIN_GPUS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);

        Self {
            wallet,
            contracts,
            verifier_service_url: verifier_url,
            node_sessions: Arc::new(Mutex::new(std::collections::HashMap::new())),
            reqs: HardwareReqs {
                memory,
                count,
                benchmark,
            },
        }
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

            // First, validate with a simple matrix challenge
            let basic_challenge_result = self.basic_challenge_node(&node).await;
            if basic_challenge_result.is_err() {
                error!(
                    "Node {} failed basic challenge: {:?}",
                    node.id, basic_challenge_result
                );
                continue;
            }

            // skip if node is already being checked and hasn't timed out
            {
                let mut sessions = self.node_sessions.lock().await;
                if let Some(session) = sessions.get_mut(&node.id) {
                    let current_time = get_time_as_secs();
                    let session_age = current_time - session.timestamp;

                    match session.status {
                        NodeChallengeStatus::Init | NodeChallengeStatus::Running => {
                            info!("Node {} challenge is still pending", node.id);
                            if session_age < VALIDATION_TIMEOUT {
                                info!("Node {} challenge is still pending", node.id);
                                continue;
                            } else {
                                session.attempts += 1;
                                session.status = NodeChallengeStatus::Init;
                                session.timestamp = get_time_as_secs();
                                session.session_id = None;
                            }
                        }
                        NodeChallengeStatus::Failed => {
                            info!("Node {} challenge failed", node.id);
                            if session.attempts >= MAX_CHALLENGE_ATTEMPTS {
                                session.status = NodeChallengeStatus::Blacklisted;
                                info!("Node {} is blacklisted", node.id);
                            } else {
                                let failure_age = current_time - session.timestamp;
                                if failure_age > VALIDATION_TIMEOUT + ERROR_TIME_BUFFER {
                                    info!("Node {} challenge is still pending", node.id);
                                    session.status = NodeChallengeStatus::Init;
                                    session.timestamp = get_time_as_secs();
                                    session.session_id = None;
                                    session.attempts += 1;
                                }
                            }
                            continue;
                        }
                        NodeChallengeStatus::Completed => {
                            info!("Node {} challenge has completed", node.id);
                            return Ok(());
                        }
                        NodeChallengeStatus::Blacklisted => {
                            info!("Node {} is blacklisted", node.id);
                            continue;
                        }
                    }
                }
            }

            // Check if debug mode is enabled via environment variable
            let debug_mode = env::var("GPU_VALIDATOR_DEBUG")
                .unwrap_or_else(|_| "false".to_string())
                .to_lowercase()
                == "true";

            if debug_mode {
                info!("Running GPU challenge in debug mode for node {}", node.id);
            }

            let matrix_size = node
                .compute_specs
                .as_ref()
                .and_then(|specs| specs.gpu.as_ref())
                .and_then(|gpu| {
                    if let (Some(memory), Some(count)) = (gpu.memory_mb, gpu.count) {
                        if memory >= self.reqs.memory && count >= self.reqs.count {
                            // saturate memory: fp32 square matrix, and 3 matrices required
                            let mem_per_matrix =
                                ((self.reqs.memory * 1024 * 1024) as f64) / 3.0 / 4.0;
                            // multiply by 0.9 to leave some room for overhead
                            let n = (mem_per_matrix * 0.9).sqrt();
                            // clip to nearest multiple of 4096
                            let matrix_size = (n / 4096.0).floor() * 4096.0;
                            Some(matrix_size as u64)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                });

            if matrix_size.is_none() && !debug_mode {
                error!("Node {} does not meet minimum requirements", node.id);
                return Err(anyhow::anyhow!("Node does not meet minimum requirements"));
            }

            // Then perform the GPU challenge
            let gpu_challenge_result = self.gpu_challenge_node(&node, matrix_size).await;
            if gpu_challenge_result.is_err() {
                error!(
                    "Node {} failed GPU challenge: {:?}",
                    node.id, gpu_challenge_result
                );
                let mut sessions = self.node_sessions.lock().await;
                if let Some(session) = sessions.get_mut(&node.id) {
                    session.status = NodeChallengeStatus::Failed;
                    session.timestamp = get_time_as_secs();
                }
                continue;
            } else {
                let mut sessions = self.node_sessions.lock().await;
                let session = sessions.get_mut(&node.id).unwrap();
                if session.commitment_time != 0 && session.commitment_time < self.reqs.benchmark {
                    info!(
                        "Node {} is validated as having H100 level performance",
                        node.id
                    );
                    session.status = NodeChallengeStatus::Completed;
                } else {
                    info!("Node {} did not meet performance requirements", node.id);
                    session.status = NodeChallengeStatus::Failed;
                    session.timestamp = get_time_as_secs();
                    continue;
                }
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
        let parsed_response: ApiResponse<T> = serde_json::from_str(&response_text)?;

        // process this json into T struct
        Ok(parsed_response.data)
    }

    async fn worker_get(&self, node: &DiscoveryNode, endpoint: &str) -> Result<String, Error> {
        info!("Sending request to worker node: {}", endpoint);
        let client = reqwest::Client::new();
        let node_url = format!("http://{}:{}", node.node.ip_address, node.node.port);

        let mut request = client.get(format!("{}{}", node_url, endpoint));

        // Add signature and address headers
        let address = self.wallet.wallet.default_signer().address().to_string();
        let signature = sign_request(endpoint, self.wallet, None)
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
            Ok(response_text)
        }
    }

    async fn gpu_challenge_node(
        &self,
        node: &DiscoveryNode,
        matrix_size: Option<u64>,
    ) -> Result<(), Error> {
        // check if node is already running a challenge session
        {
            // Create a separate scope for the lock
            let mut sessions = self.node_sessions.lock().await;

            if let Some(session) = sessions.get(&node.id) {
                // if we get here, it should be in the init stage
                if session.status != NodeChallengeStatus::Init {
                    info!("Node {} already has a challenge session running", node.id);
                    return Err(anyhow::anyhow!(
                        "Node already has a challenge session running"
                    ));
                }
            } else {
                info!("No session found for node, creating: {}", node.id);
                sessions.insert(
                    node.id.clone(),
                    NodeChallengeState {
                        session_id: None,
                        timestamp: get_time_as_secs(),
                        status: NodeChallengeStatus::Init,
                        attempts: 0,
                        commitment_time: 0,
                    },
                );
            }
        }

        // STEP 1: Initialize a new challenge with the verifier service
        let init_request = GpuChallengeInitRequest {
            n: matrix_size.unwrap_or(MATRIX_CHALLENGE_SIZE_DEFAULT), // Default matrix size
        };

        let init_data: GpuChallengeResponse = self
            .verifier_send("/init", Some(json!(init_request)))
            .await
            .context("Failed to initialize GPU challenge")?;

        info!("Initialized verifier session: {}", init_data.session_id);

        {
            let mut sessions = self.node_sessions.lock().await;
            let session = sessions.get_mut(&node.id).unwrap();
            session.session_id = Some(init_data.session_id.clone());
            session.status = NodeChallengeStatus::Running;
            session.timestamp = get_time_as_secs();

            info!(
                "Starting challenge state: {}, {}",
                session.session_id.as_ref().unwrap(),
                session.timestamp
            );
        }

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

        if response.status != "initializing" {
            return Err(anyhow::anyhow!(
                "Failed to start GPU challenge on worker node"
            ));
        }

        // STEP 3: Check status until worker is ready
        let status_route = "/gpu-challenge/status";

        let mut max_attempts = 100;

        loop {
            match self.worker_get(node, status_route).await {
                Ok(response_text) => {
                    let status_response: ApiResponse<GpuChallengeStatus> =
                        serde_json::from_str(&response_text)?;
                    info!("Worker response: {:?}", status_response);
                    if status_response.data.status == "ready" {
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

        // reset timestamp to calculate time taken to compute commitment
        {
            let mut sessions = self.node_sessions.lock().await;
            let session = sessions.get_mut(&node.id).unwrap();
            session.timestamp = get_time_as_secs();
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

        {
            let mut sessions = self.node_sessions.lock().await;
            let session = sessions.get_mut(&node.id).unwrap();
            // the following value is the time it took the node to compute the A*B matmul
            session.commitment_time = get_time_as_secs() - session.timestamp;
        }

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

        let row_proofs_payload = GpuChallengeWorkerComputeRowProofs {
            session_id: init_data.session_id.clone(),
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
