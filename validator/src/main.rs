use alloy::primitives::{hex, Address};
use alloy::signers::Signer;
use clap::Parser;
use log::LevelFilter;
use log::{error, info};
use shared::models::api::ApiResponse;
use shared::models::node::DiscoveryNode;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use std::env;
use url::Url;

#[derive(Parser)]
struct Args {
    /// RPC URL
    #[arg(short = 'r', long, default_value = "http://localhost:8545")]
    rpc_url: String,

    /// Owner key
    #[arg(short = 'k', long)]
    validator_key: String,

    /// Discovery url
    #[arg(long, default_value = "http://localhost:8089")]
    discovery_url: String,
}
fn main() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    env_logger::Builder::new()
        .filter_level(LevelFilter::Info)
        .format_timestamp(None)
        .init();

    let args = Args::parse();
    let private_key_validator = args.validator_key;
    let rpc_url: Url = args.rpc_url.parse().unwrap();
    let discovery_url = args.discovery_url;

    let validator_wallet = Wallet::new(&private_key_validator, rpc_url).unwrap_or_else(|err| {
        error!("Error creating wallet: {:?}", err);
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

        let nodes: Result<Vec<DiscoveryNode>, Box<dyn std::error::Error>> =
            runtime.block_on(async {
                let discovery_route = "/api/validator";
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

                info!("Fetching nodes from: {}{}", discovery_url, discovery_route);
                let response = reqwest::Client::new()
                    .get(format!("{}{}", discovery_url, discovery_route))
                    .headers(headers)
                    .send()
                    .await?;

                let response_text = response.text().await?;
                let parsed_response: ApiResponse<Vec<DiscoveryNode>> =
                    serde_json::from_str(&response_text)?;

                if !parsed_response.success {
                    error!("Failed to fetch nodes: {:?}", parsed_response);
                    return Ok(vec![]);
                }

                Ok(parsed_response.data)
            });
        let non_validated_nodes: Vec<DiscoveryNode> = nodes
            .iter()
            .flatten()
            .filter(|node| !node.is_validated)
            .cloned()
            .collect();

        info!("Non validated nodes: {:?}", non_validated_nodes);

        for node in non_validated_nodes {
            let node_address = node.id.trim_start_matches("0x").parse::<Address>().unwrap();

            let provider_address = node
                .provider_address
                .trim_start_matches("0x")
                .parse::<Address>()
                .unwrap();

            if let Err(e) = runtime.block_on(
                contracts
                    .prime_network
                    .validate_node(provider_address, node_address),
            ) {
                error!("Failed to validate node {}: {}", node.id, e);
            } else {
                info!("Successfully validated node: {}", node.id);
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(10));
    }
}
