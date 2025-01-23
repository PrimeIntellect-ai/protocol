use alloy::primitives::{hex, Address};
use alloy::signers::Signer;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use std::env;
use url::Url;
#[allow(dead_code)]
#[derive(serde::Deserialize, Debug, Clone)]
struct ComputeNode {
    id: String,
    #[serde(rename = "providerAddress")]
    provider_address: String,
    #[serde(rename = "ipAddress")]
    ip_address: String,
    port: u16,
    #[serde(rename = "computePoolId")]
    compute_pool_id: u64,
    #[serde(rename = "computeSpecs")]
    compute_specs: ComputeSpecs,
    #[serde(rename = "lastSeen")]
    last_seen: u64,
    #[serde(rename = "isActive")]
    is_active: bool,
    #[serde(rename = "isValidated")]
    is_validated: bool,
}

#[allow(dead_code)]
#[derive(serde::Deserialize, Debug, Clone)]
struct ComputeSpecs {
    gpu: Option<GpuSpecs>,
    cpu: Option<CpuSpecs>,
    #[serde(rename = "ramMB")]
    ram_mb: Option<u32>,
    #[serde(rename = "storageGB")]
    storage_gb: Option<u32>,
}

#[allow(dead_code)]
#[derive(serde::Deserialize, Debug, Clone)]
struct GpuSpecs {
    count: Option<u32>,
    model: Option<String>,
    #[serde(rename = "memoryMB")]
    memory_mb: Option<u32>,
}
#[allow(dead_code)]
#[derive(serde::Deserialize, Debug, Clone)]
struct CpuSpecs {
    cores: Option<u32>,
    model: Option<String>,
}

fn main() {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let private_key_validator =
        env::var("PRIVATE_KEY_VALIDATOR").expect("PRIVATE_KEY_VALIDATOR not set");
    let rpc_url = "http://localhost:8545";
    let validator_wallet = Wallet::new(&private_key_validator, Url::parse(rpc_url).unwrap())
        .unwrap_or_else(|err| {
            eprintln!("Error creating wallet: {:?}", err);
            std::process::exit(1);
        });

    let contracts = ContractBuilder::new(&validator_wallet)
        .with_compute_registry()
        .with_ai_token()
        .with_prime_network()
        .with_compute_pool()
        .build()
        .unwrap();

    loop {
        async fn _generate_signature(
            wallet: &Wallet,
            message: &str,
        ) -> Result<String, Box<dyn std::error::Error>> {
            let signature = wallet
                .signer
                .sign_message(message.as_bytes())
                .await?
                .as_bytes();
            Ok(format!("0x{}", hex::encode(signature)))
        }

        let nodes: Result<Vec<ComputeNode>, Box<dyn std::error::Error>> = runtime.block_on(async {
            let discovery_url = "http://localhost:8089";
            let discovery_route = "/api/nodes/validator";
            let address = validator_wallet
                .wallet
                .default_signer()
                .address()
                .to_string();
            let signature = _generate_signature(&validator_wallet, discovery_route)
                .await
                .unwrap();

            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert("x-address", address.parse().unwrap());
            headers.insert("x-signature", signature.parse().unwrap());

            println!("Fetching nodes from: {}{}", discovery_url, discovery_route);
            let response = reqwest::Client::new()
                .get(format!("{}{}", discovery_url, discovery_route))
                .headers(headers)
                .send()
                .await?;

            let response_text = response.text().await?;
            println!("Response received: {}", response_text);
            let parsed_response: serde_json::Value = serde_json::from_str(&response_text)?;
            serde_json::from_value(parsed_response["data"].clone()).map_err(Into::into)
        });

        let non_validated_nodes: Vec<ComputeNode> = nodes
            .iter()
            .filter_map(|node_vec| node_vec.first())
            .filter(|node| !node.is_validated)
            .cloned()
            .collect();

        println!("Non validated nodes: {:?}", non_validated_nodes);

        for compute_node in non_validated_nodes {
            if !compute_node.provider_address.is_empty() && !compute_node.id.is_empty() {
                let provider_address = compute_node
                    .provider_address
                    .trim_start_matches("0x")
                    .parse::<Address>()
                    .unwrap();
                let node_address = compute_node
                    .id
                    .trim_start_matches("0x")
                    .parse::<Address>()
                    .unwrap();

                println!(
                    "Validating node: {} for provider: {}",
                    compute_node.id, compute_node.provider_address
                );
                if let Err(e) = runtime.block_on(
                    contracts
                        .prime_network
                        .validate_node(provider_address, node_address),
                ) {
                    eprintln!("Failed to validate node {}: {}", compute_node.id, e);
                } else {
                    println!("Successfully validated node: {}", compute_node.id);
                }
            }
        }

        println!("Sleeping for 10 seconds before the next iteration...");
        std::thread::sleep(std::time::Duration::from_secs(10));
    }
}
