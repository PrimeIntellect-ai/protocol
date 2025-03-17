use crate::state::system_state::SystemState;
use alloy::primitives::{Address, U256};
use anyhow::Result;
use log::{debug, error, info};
use reqwest::header::HeaderValue;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use shared::models::node::Node;
use shared::security::request_signer::sign_request;
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::wallet::Wallet;
use std::str::FromStr;
use std::sync::Arc;

#[derive(Deserialize, Serialize, Debug)]
pub struct RequestUploadRequest {
    pub file_name: String,
    pub file_size: u64,
    pub file_type: String,
}

/// Handles a file upload request
pub async fn handle_file_upload(
    storage_path: &str,
    task_id: &str,
    file_name: &str,
    wallet: &Arc<Wallet>,
    state: &Arc<SystemState>,
) -> Result<()> {
    info!("📄 Received file upload request: {}", file_name);

    // Get orchestrator endpoint
    let endpoint = state
        .get_heartbeat_endpoint()
        .await
        .ok_or_else(|| {
            error!("Orchestrator endpoint is not set - cannot upload file.");
            anyhow::anyhow!("Orchestrator endpoint not set")
        })?
        .replace("/heartbeat", "");

    // Clean filename by removing /data prefix if present
    let clean_file_name = file_name.trim_start_matches("/data/");

    // Construct file path
    let file = format!(
        "{}/prime-task-{}/{}",
        storage_path, task_id, clean_file_name
    );
    debug!("File: {:?}", file);

    // Get file size
    let file_size = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);

    // Calculate SHA
    let file_sha = tokio::fs::read(&file)
        .await
        .map(|contents| {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(&contents);
            format!("{:x}", hasher.finalize())
        })
        .unwrap_or_else(|e| {
            error!("Failed to calculate file SHA: {}", e);
            String::new()
        });

    debug!("File size: {:?}", file_size);
    debug!("File SHA: {}", file_sha);

    // Create upload request
    let client = Client::new();
    let request = RequestUploadRequest {
        file_name: if clean_file_name.ends_with(".parquet") {
            format!("{}.parquet", file_sha)
        } else {
            file_sha
        },
        file_size,
        file_type: "application/json".to_string(), // Assume JSON
    };

    // Sign request
    let request_value = serde_json::to_value(&request)?;
    let signature = sign_request("/storage/request-upload", wallet, Some(&request_value))
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    // Prepare headers
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "x-address",
        HeaderValue::from_str(&wallet.address().to_string())?,
    );
    headers.insert("x-signature", HeaderValue::from_str(&signature)?);

    // Create upload URL
    let upload_url = format!("{}/storage/request-upload", endpoint);

    // Send request
    let response = client
        .post(&upload_url)
        .json(&request)
        .headers(headers)
        .send()
        .await?;

    // Process response
    let json = response.json::<serde_json::Value>().await?;

    if let Some(signed_url) = json["signed_url"].as_str() {
        info!("Got signed URL for upload: {}", signed_url);

        // Read file contents
        let file_contents = tokio::fs::read(&file).await?;

        // Upload file to S3 using signed URL
        client
            .put(signed_url)
            .body(file_contents)
            .header("Content-Type", "application/json")
            .send()
            .await?;

        info!("Successfully uploaded file to S3");
    } else {
        println!("Error: Missing signed_url in response");
        return Err(anyhow::anyhow!("Missing signed_url in response"));
    }

    Ok(())
}

/// Handles a file validation request
pub async fn handle_file_validation(
    file_sha: &str,
    contracts: &Arc<Contracts>,
    node: &Node,
) -> Result<()> {
    info!("📄 Received file SHA for validation: {}", file_sha);

    let pool_id = node.compute_pool_id;
    let node_address = &node.id;

    let decoded_sha = hex::decode(file_sha)?;
    debug!(
        "Decoded file sha: {:?} ({} bytes)",
        decoded_sha,
        decoded_sha.len()
    );

    let result = contracts
        .compute_pool
        .submit_work(
            U256::from(pool_id),
            Address::from_str(node_address)?,
            decoded_sha.to_vec(),
        )
        .await;

    debug!("Submit work result: {:?}", result);

    Ok(())
}
