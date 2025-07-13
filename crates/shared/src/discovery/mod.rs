use crate::models::api::ApiResponse;
use crate::models::node::DiscoveryNode;
use crate::security::request_signer::sign_request_with_nonce;
use crate::web3::wallet::Wallet;
use anyhow::{Context, Result};
use log::{debug, error};
use std::time::Duration;

/// Fetch nodes from a single discovery URL
pub async fn fetch_nodes_from_discovery_url(
    discovery_url: &str,
    route: &str,
    wallet: &Wallet,
) -> Result<Vec<DiscoveryNode>> {
    let address = wallet.wallet.default_signer().address().to_string();

    let signature = sign_request_with_nonce(route, wallet, None)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "x-address",
        reqwest::header::HeaderValue::from_str(&address)
            .context("Failed to create address header")?,
    );
    headers.insert(
        "x-signature",
        reqwest::header::HeaderValue::from_str(&signature.signature)
            .context("Failed to create signature header")?,
    );

    debug!("Fetching nodes from: {discovery_url}{route}");
    let response = reqwest::Client::new()
        .get(format!("{discovery_url}{route}"))
        .query(&[("nonce", signature.nonce)])
        .headers(headers)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .context("Failed to fetch nodes")?;

    let response_text = response
        .text()
        .await
        .context("Failed to get response text")?;

    let parsed_response: ApiResponse<Vec<DiscoveryNode>> =
        serde_json::from_str(&response_text).context("Failed to parse response")?;

    if !parsed_response.success {
        error!("Failed to fetch nodes from {discovery_url}: {parsed_response:?}");
        return Ok(vec![]);
    }

    Ok(parsed_response.data)
}

/// Fetch nodes from multiple discovery URLs with deduplication
pub async fn fetch_nodes_from_discovery_urls(
    discovery_urls: &[String],
    route: &str,
    wallet: &Wallet,
) -> Result<Vec<DiscoveryNode>> {
    let mut all_nodes = Vec::new();
    let mut any_success = false;

    for discovery_url in discovery_urls {
        match fetch_nodes_from_discovery_url(discovery_url, route, wallet).await {
            Ok(nodes) => {
                debug!(
                    "Successfully fetched {} nodes from {}",
                    nodes.len(),
                    discovery_url
                );
                all_nodes.extend(nodes);
                any_success = true;
            }
            Err(e) => {
                error!("Failed to fetch nodes from {discovery_url}: {e:#}");
            }
        }
    }

    if !any_success {
        error!("Failed to fetch nodes from all discovery services");
        return Ok(vec![]);
    }

    // Remove duplicates based on node ID
    let mut unique_nodes = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();
    for node in all_nodes {
        if seen_ids.insert(node.node.id.clone()) {
            unique_nodes.push(node);
        }
    }

    debug!(
        "Total unique nodes after deduplication: {}",
        unique_nodes.len()
    );
    Ok(unique_nodes)
}

/// Fetch nodes for a specific pool from discovery
pub async fn fetch_pool_nodes_from_discovery(
    discovery_urls: &[String],
    compute_pool_id: u32,
    wallet: &Wallet,
) -> Result<Vec<DiscoveryNode>> {
    let route = format!("/api/pool/{compute_pool_id}");
    fetch_nodes_from_discovery_urls(discovery_urls, &route, wallet).await
}

/// Fetch all validator-accessible nodes from discovery
pub async fn fetch_validator_nodes_from_discovery(
    discovery_urls: &[String],
    wallet: &Wallet,
) -> Result<Vec<DiscoveryNode>> {
    let route = "/api/validator";
    fetch_nodes_from_discovery_urls(discovery_urls, route, wallet).await
}

/// Upload node information to discovery services
///
/// This function attempts to upload node information to all provided discovery URLs.
/// It returns Ok(()) if at least one upload succeeds.
pub async fn upload_node_to_discovery(
    discovery_urls: &[String],
    node_data: &serde_json::Value,
    wallet: &Wallet,
) -> Result<()> {
    let endpoint = "/api/nodes";
    let mut last_error: Option<String> = None;

    for discovery_url in discovery_urls {
        match upload_to_single_discovery(discovery_url, endpoint, node_data, wallet).await {
            Ok(_) => {
                debug!("Successfully uploaded node info to {discovery_url}");
                return Ok(());
            }
            Err(e) => {
                error!("Failed to upload to {discovery_url}: {e}");
                last_error = Some(e.to_string());
            }
        }
    }

    // If we reach here, all discovery services failed
    if let Some(error) = last_error {
        Err(anyhow::anyhow!(
            "Failed to upload to all discovery services. Last error: {}",
            error
        ))
    } else {
        Err(anyhow::anyhow!(
            "Failed to upload to all discovery services"
        ))
    }
}

async fn upload_to_single_discovery(
    base_url: &str,
    endpoint: &str,
    node_data: &serde_json::Value,
    wallet: &Wallet,
) -> Result<()> {

    let signed_request = sign_request_with_nonce(endpoint, wallet, Some(node_data))
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "x-address",
        wallet
            .wallet
            .default_signer()
            .address()
            .to_string()
            .parse()
            .context("Failed to parse address header")?,
    );
    headers.insert(
        "x-signature",
        signed_request
            .signature
            .parse()
            .context("Failed to parse signature header")?,
    );

    let request_url = format!("{base_url}{endpoint}");
    let response = reqwest::Client::new()
        .put(&request_url)
        .headers(headers)
        .json(
            &signed_request
                .data
                .expect("Signed request data should always be present for discovery upload"),
        )
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .context("Failed to send request")?;

    if !response.status().is_success() {
        let status = response.status();
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "No error message".to_string());
        return Err(anyhow::anyhow!(
            "Error: Received response with status code {} from {}: {}",
            status,
            base_url,
            error_text
        ));
    }

    Ok(())
}
