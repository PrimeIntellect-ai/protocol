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
    let address = wallet
        .wallet
        .default_signer()
        .address()
        .to_string();

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
    let route = format!("/api/pool/{}", compute_pool_id);
    fetch_nodes_from_discovery_urls(discovery_urls, &route, wallet).await
}

/// Fetch all validator-accessible nodes from discovery
pub async fn fetch_validator_nodes_from_discovery(
    discovery_urls: &[String],
    wallet: &Wallet,
) -> Result<Vec<DiscoveryNode>> {
    let route = "/api/validator";
    fetch_nodes_from_discovery_urls(discovery_urls, &route, wallet).await
} 