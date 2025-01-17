use alloy::primitives::{hex, Address};
use alloy::signers::Signer;
use serde;
use serde_json;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use std::env;
use tokio;
use url::Url;
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
    compute_specs: ComputeSpecs, // Fixed field name to match the JSON response
    #[serde(rename = "lastSeen")]
    last_seen: u64,
    #[serde(rename = "isActive")]
    is_active: bool,
    #[serde(rename = "isValidated")]
    is_validated: bool,
}

#[derive(serde::Deserialize, Debug, Clone)]
struct ComputeSpecs {
    gpu: Option<GpuSpecs>,
    cpu: Option<CpuSpecs>,
    #[serde(rename = "ramMB")]
    ram_mb: Option<u32>,
    #[serde(rename = "storageGB")]
    storage_gb: Option<u32>,
}

#[derive(serde::Deserialize, Debug, Clone)]
struct GpuSpecs {
    count: Option<u32>,
    model: Option<String>,
    #[serde(rename = "memoryMB")]
    memory_mb: Option<u32>,
}

#[derive(serde::Deserialize, Debug, Clone)]
struct CpuSpecs {
    cores: Option<u32>,
    model: Option<String>,
}
fn main() {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    // Load private key validator from environment variable
    let private_key_validator =
        env::var("PRIVATE_KEY_VALIDATOR").expect("PRIVATE_KEY_VALIDATOR not set");

    let rpc_url = "http://localhost:8545";
    let validator_wallet = match Wallet::new(&private_key_validator, Url::parse(rpc_url).unwrap()) {
        Ok(wallet) => wallet,
        Err(err) => {
            println!("Error creating wallet: {:?}", err);
            std::process::exit(1);
        }
    };

    let contracts = ContractBuilder::new(&validator_wallet)
        .with_compute_registry()
        .with_ai_token()
        .with_prime_network()
        .build()
        .unwrap();

    let response = runtime.block_on(async {
        let address = Address::new(hex!("0x0000000000000000000000000000000000000000"));
        match contracts.ai_token.balance_of(address).await {
            Ok(balance) => println!("Balance: {:?}", balance),
            Err(err) => eprintln!("Error fetching balance: {:?}", err),
        }
    });

    async fn _generate_signature(
        wallet: &Wallet,
        message: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        println!(
            "Signin with address: {:?}",
            wallet.wallet.default_signer().address()
        );
        let signature = wallet
            .signer
            .sign_message(message.as_bytes())
            .await?
            .as_bytes();
        let signature_string = format!("0x{}", hex::encode(signature));
        Ok(signature_string)
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
            .unwrap(); // Replace with actual signature generation logic
        println!("Signature: {:?}", signature);

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-address", address.parse().unwrap());
        headers.insert("x-signature", signature.parse().unwrap());
        let response = reqwest::Client::new()
            .get(format!("{}{}", discovery_url, discovery_route))
            .headers(headers)
            .send()
            .await
            .expect("Failed to send request");

        let response_text = response.text().await.expect("Failed to read response text");
        println!("Response: {:?}", response_text);

        // Define the struct to parse the response

        // Parse the JSON response
        let parsed_response: serde_json::Value =
            serde_json::from_str(&response_text).expect("Failed to parse JSON response");
        let nodes: Vec<ComputeNode> = serde_json::from_value(parsed_response["data"].clone())
            .expect("Failed to deserialize nodes");
        Ok(nodes)
    });

    let non_validated_nodes: Vec<ComputeNode> = nodes
        .iter()
        .filter_map(|node_vec| node_vec.first()) // Get first node if it exists
        .filter(|node| !node.is_validated)
        .cloned()
        .collect();

    println!("Non validated nodes: {:?}", non_validated_nodes);

    for compute_node in non_validated_nodes {
        // Call validate contract function only if provider_address and id are valid
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

            let result = runtime.block_on(
                contracts
                    .prime_network
                    .validate_node(provider_address.into(), node_address.into()),
            );
            if let Err(e) = result {
                eprintln!("Failed to validate node {}: {}", compute_node.id, e);
            }
        }
    }

    println!("Response: {:?}", response);
}
