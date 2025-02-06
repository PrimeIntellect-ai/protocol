use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use alloy::primitives::{hex, Address};
use alloy::signers::Signer;
use anyhow::{Context, Result};
use clap::Parser;
use log::LevelFilter;
use log::{error, info};
use serde_json::json;
use shared::models::api::ApiResponse;
use shared::models::node::DiscoveryNode;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use url::Url;

async fn health_check() -> impl Responder {
    HttpResponse::Ok().json(json!({ "status": "ok" }))
}

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

    runtime.spawn(async {
        if let Err(e) = HttpServer::new(|| App::new().route("/health", web::get().to(health_check)))
            .bind("0.0.0.0:8080")
            .expect("Failed to bind health check server")
            .run()
            .await
        {
            error!("Actix server error: {:?}", e);
        }
    });

    let contracts = ContractBuilder::new(&validator_wallet)
        .with_compute_registry()
        .with_ai_token()
        .with_prime_network()
        .with_compute_pool()
        .build()
        .unwrap();
    loop {
        async fn _generate_signature(wallet: &Wallet, message: &str) -> Result<String> {
            let signature = wallet
                .signer
                .sign_message(message.as_bytes())
                .await
                .context("Failed to sign message")?
                .as_bytes();
            Ok(format!("0x{}", hex::encode(signature)))
        }

        let nodes = match runtime.block_on(async {
            let discovery_route = "/api/validator";
            let address = validator_wallet
                .wallet
                .default_signer()
                .address()
                .to_string();

            let signature = _generate_signature(&validator_wallet, discovery_route)
                .await
                .context("Failed to generate signature")?;

            let mut headers = reqwest::header::HeaderMap::new();
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

            info!("Fetching nodes from: {}{}", discovery_url, discovery_route);
            let response = reqwest::Client::new()
                .get(format!("{}{}", discovery_url, discovery_route))
                .headers(headers)
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
                error!("Failed to fetch nodes: {:?}", parsed_response);
                return Ok::<Vec<DiscoveryNode>, anyhow::Error>(vec![]);
            }

            Ok(parsed_response.data)
        }) {
            Ok(n) => n,
            Err(e) => {
                error!("Error in node fetching loop: {:#}", e);
                std::thread::sleep(std::time::Duration::from_secs(10));
                continue;
            }
        };

        let non_validated_nodes: Vec<DiscoveryNode> = nodes
            .iter()
            .filter(|node| !node.is_validated)
            .cloned()
            .collect();

        info!("Non validated nodes: {:?}", non_validated_nodes);

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
