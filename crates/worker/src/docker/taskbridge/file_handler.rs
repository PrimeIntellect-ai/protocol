use crate::state::system_state::SystemState;
use alloy::primitives::{Address, U256};
use anyhow::Context as _;
use anyhow::Result;
use log::{debug, error, info, warn};
use reqwest::header::HeaderValue;
use reqwest::Client;
use rust_ipfs::Ipfs;
use shared::models::node::Node;
use shared::models::storage::RequestUploadRequest;
use shared::security::request_signer::sign_request_with_nonce;
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::contracts::helpers::utils::retry_call;
use shared::web3::wallet::{Wallet, WalletProvider};
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

/// Handles a file upload request
pub async fn handle_file_upload(
    storage_path: String,
    task_id: String,
    file_name: String,
    wallet: Wallet,
    state: Arc<SystemState>,
    ipfs: Option<Ipfs>,
) -> Result<()> {
    info!("📄 Received file upload request: {file_name}");
    info!("Task ID: {task_id}, Storage path: {storage_path}");

    // Get orchestrator endpoint
    let endpoint = match state.get_heartbeat_endpoint().await {
        Some(ep) => {
            let clean_ep = ep.replace("/heartbeat", "");
            info!("Using orchestrator endpoint: {clean_ep}");
            clean_ep
        }
        None => {
            error!("Orchestrator endpoint is not set - cannot upload file.");
            return Err(anyhow::anyhow!("Orchestrator endpoint not set"));
        }
    };

    // Clean filename by removing /data prefix if present
    let clean_file_name = file_name.trim_start_matches("/data/");
    info!("Clean file name: {clean_file_name}");

    let task_dir = format!("prime-task-{task_id}");
    let file_path = Path::new(&storage_path)
        .join(&task_dir)
        .join("data")
        .join(clean_file_name);
    let file = file_path.to_string_lossy().to_string();
    info!("Full file path: {file}");

    // Get file size
    let file_size = match std::fs::metadata(&file) {
        Ok(metadata) => {
            let size = metadata.len();
            info!("File size: {size} bytes");
            size
        }
        Err(e) => {
            error!("Failed to get file metadata: {e}");
            return Err(anyhow::anyhow!("Failed to get file metadata: {}", e));
        }
    };

    // Calculate SHA
    let file_sha = match tokio::fs::read(&file).await {
        Ok(contents) => {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(&contents);
            let sha = format!("{:x}", hasher.finalize());
            info!("Calculated file SHA: {sha}");
            sha
        }
        Err(e) => {
            error!("Failed to read file for SHA calculation: {e}");
            return Err(anyhow::anyhow!("Failed to read file: {}", e));
        }
    };

    // Create upload request
    let client = Client::new();
    let request = RequestUploadRequest {
        file_name: file_name.to_string(),
        file_size,
        file_type: "application/json".to_string(), // Assume JSON
        sha256: file_sha.clone(),
        task_id: task_id.to_string(),
    };

    let signed_url = get_signed_url_for_upload(request, wallet.clone(), &endpoint, &client)
        .await
        .context("failed to get signed URL for upload")?;

    info!("Reading file contents for S3 upload: {file}");
    let file_contents = match tokio::fs::read(&file).await {
        Ok(contents) => {
            info!("Successfully read file ({} bytes)", contents.len());
            contents
        }
        Err(e) => {
            anyhow::bail!("failed to read file: {}", e);
        }
    };

    if let Some(ipfs) = ipfs {
        handle_file_upload_ipfs(file_contents.clone(), &ipfs)
            .await
            .context("failed to upload file to IPFS")?;
    }

    handle_file_upload_s3(file_contents, signed_url, &client)
        .await
        .context("failed to upload file to S3")
}

async fn get_signed_url_for_upload(
    request: RequestUploadRequest,
    wallet: Wallet,
    endpoint: &str,
    client: &Client,
) -> Result<String> {
    // Retry configuration
    const MAX_RETRIES: usize = 5;
    const INITIAL_RETRY_DELAY_MS: u64 = 1000; // 1 second

    // Retry loop for getting signed URL
    let mut retry_count = 0;
    let mut last_error = None;
    let mut signed_url = None;

    info!("Starting signed URL request with max {MAX_RETRIES} retries");
    while retry_count < MAX_RETRIES {
        if retry_count > 0 {
            let delay = INITIAL_RETRY_DELAY_MS * (1 << retry_count); // Exponential backoff
            warn!(
                "Retrying upload request (attempt {}/{}), waiting for {}ms",
                retry_count + 1,
                MAX_RETRIES,
                delay
            );
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }

        // Sign request
        let request_value = match serde_json::to_value(&request) {
            Ok(val) => val,
            Err(e) => {
                error!("Failed to serialize request: {e}");
                return Err(anyhow::anyhow!(e));
            }
        };

        let signature =
            match sign_request_with_nonce("/storage/request-upload", &wallet, Some(&request_value))
                .await
            {
                Ok(sig) => {
                    debug!("Request signed successfully: {}", sig.signature);
                    sig
                }
                Err(e) => {
                    error!("Failed to sign request: {e}");
                    last_error = Some(anyhow::anyhow!(e.to_string()));
                    retry_count += 1;
                    continue;
                }
            };

        // Prepare headers
        let mut headers = reqwest::header::HeaderMap::new();
        match HeaderValue::from_str(&wallet.address().to_string()) {
            Ok(val) => {
                headers.insert("x-address", val);
                debug!("Added x-address header: {}", wallet.address());
            }
            Err(e) => {
                error!("Failed to create header value: {e}");
                last_error = Some(anyhow::anyhow!(e));
                retry_count += 1;
                continue;
            }
        }

        match HeaderValue::from_str(&signature.signature) {
            Ok(val) => {
                headers.insert("x-signature", val);
                debug!("Added x-signature header");
            }
            Err(e) => {
                error!("Failed to create signature header: {e}");
                last_error = Some(anyhow::anyhow!(e));
                retry_count += 1;
                continue;
            }
        }

        // Create upload URL
        let upload_url = format!("{endpoint}/storage/request-upload");
        debug!("Requesting signed URL from: {upload_url}");

        // Send request
        debug!(
            "Sending request for signed URL (attempt {}/{})",
            retry_count + 1,
            MAX_RETRIES
        );
        let response = match client
            .post(&upload_url)
            .json(&signature.data)
            .headers(headers)
            .send()
            .await
        {
            Ok(resp) => {
                debug!("Received response with status: {}", resp.status());
                resp
            }
            Err(e) => {
                error!("Failed to send upload request: {e}");
                last_error = Some(anyhow::anyhow!(e));
                retry_count += 1;
                continue;
            }
        };

        // Process response
        let json = match response.json::<serde_json::Value>().await {
            Ok(j) => {
                debug!("Parsed response JSON: {j:?}");
                j
            }
            Err(e) => {
                error!("Failed to parse response: {e}");
                last_error = Some(anyhow::anyhow!(e));
                retry_count += 1;
                continue;
            }
        };

        if let Some(url) = json["signed_url"].as_str() {
            signed_url = Some(url.to_string());
            debug!("Got signed URL for upload (length: {})", url.len());
            debug!("Signed URL: {url}");
            break;
        } else {
            error!("Missing signed_url in response: {json:?}");
            last_error = Some(anyhow::anyhow!("Missing signed_url in response"));
            retry_count += 1;
            continue;
        }
    }

    let signed_url = match signed_url {
        Some(url) => url,
        None => {
            error!("Failed to get signed URL after {MAX_RETRIES} attempts");
            return Err(last_error.unwrap_or_else(|| {
                anyhow::anyhow!("Failed to get signed URL after {} attempts", MAX_RETRIES)
            }));
        }
    };
    Ok(signed_url)
}

async fn handle_file_upload_s3(
    file_contents: Vec<u8>,
    signed_url: String,
    client: &Client,
) -> Result<()> {
    // Retry configuration
    const MAX_RETRIES: usize = 5;
    const INITIAL_RETRY_DELAY_MS: u64 = 1000; // 1 second

    // Retry loop for uploading file to S3
    let mut retry_count = 0;
    let mut last_error = None;

    debug!("Starting S3 upload with max {MAX_RETRIES} retries");
    while retry_count < MAX_RETRIES {
        if retry_count > 0 {
            let delay = INITIAL_RETRY_DELAY_MS * (1 << retry_count); // Exponential backoff
            warn!(
                "Retrying S3 upload (attempt {}/{}), waiting for {}ms",
                retry_count + 1,
                MAX_RETRIES,
                delay
            );
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }

        // Upload file to S3 using signed URL
        info!(
            "Uploading file to S3 (attempt {}/{})",
            retry_count + 1,
            MAX_RETRIES
        );
        match client
            .put(&signed_url)
            .body(file_contents.clone())
            .header("Content-Type", "application/json")
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                debug!("S3 upload response status: {status}");

                if status.is_success() {
                    info!("Successfully uploaded file to S3");
                    return Ok(());
                } else {
                    let error_text = resp
                        .text()
                        .await
                        .unwrap_or_else(|_| "Unknown error".to_string());
                    error!("S3 upload failed with status {status}: {error_text}");
                    last_error = Some(anyhow::anyhow!(
                        "S3 upload failed: {} - {}",
                        status,
                        error_text
                    ));
                    retry_count += 1;
                    continue;
                }
            }
            Err(e) => {
                error!("Failed to upload to S3: {e}");
                last_error = Some(anyhow::anyhow!(e));
                retry_count += 1;
                continue;
            }
        }
    }

    error!("Failed to upload file to S3 after {MAX_RETRIES} attempts");
    Err(last_error.unwrap_or_else(|| {
        anyhow::anyhow!("Failed to upload file to S3 after {} attempts", MAX_RETRIES)
    }))
}

async fn handle_file_upload_ipfs(file_contents: Vec<u8>, ipfs: &Ipfs) -> Result<cid::Cid> {
    let put = ipfs
        .put_dag(file_contents)
        .codec(rust_ipfs::block::BlockCodec::Raw);
    let cid = put.await.context("failed to put data into IPFS")?;
    info!("successfully uploaded file to IPFS with CID: {cid}");

    ipfs.provide(cid)
        .await
        .context("failed to provide CID on IPFS")?;
    Ok(cid)
}

/// Handles a file validation request
pub(crate) async fn handle_file_validation(
    file_sha: String,
    contracts: Contracts<WalletProvider>,
    node: Node,
    provider: WalletProvider,
    work_units: f64,
) -> Result<()> {
    info!("📄 Received file SHA for validation: {file_sha}");
    info!(
        "Node address: {}, Pool ID: {}",
        node.id, node.compute_pool_id
    );

    let pool_id = node.compute_pool_id;
    let node_address = &node.id;
    let decoded_sha = match hex::decode(file_sha) {
        Ok(sha) => {
            debug!("Decoded SHA bytes: {sha:?}");
            sha
        }
        Err(e) => {
            error!("Failed to decode SHA hex string: {e}");
            return Err(anyhow::anyhow!("Failed to decode SHA: {}", e));
        }
    };

    let node_addr = match Address::from_str(node_address) {
        Ok(addr) => addr,
        Err(e) => {
            error!("Failed to parse node address: {e}");
            return Err(anyhow::anyhow!("Invalid node address: {}", e));
        }
    };

    let pool_id_u256 = U256::from(pool_id);

    let call = contracts
        .compute_pool
        .build_work_submission_call(
            pool_id_u256,
            node_addr,
            decoded_sha.to_vec(),
            U256::from(work_units),
        )
        .unwrap();

    let tx = retry_call(call, 20, provider.clone(), None)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to submit work: {}", e))?;

    info!("Successfully submitted work to blockchain: {tx:?}");
    Ok(())
}
