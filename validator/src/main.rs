use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use alloy::primitives::{hex, Address};
use alloy::signers::Signer;
use anyhow::{Context, Result};
use clap::Parser;
use log::LevelFilter;
use log::{error, info};
use rand::rng;
use rand::Rng;
use serde_json::json;
use nalgebra::DMatrix;
use rand::rng;
use rand::Rng;
use shared::models::api::ApiResponse;
use shared::models::challenge::{ChallengeRequest, ChallengeResponse};
use shared::models::node::DiscoveryNode;
use shared::security::request_signer::sign_request;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use url::Url;

<<<<<<< HEAD
async fn health_check() -> impl Responder {
    HttpResponse::Ok().json(json!({ "status": "ok" }))
}

=======
>>>>>>> 21b6ce5 (fmt)
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

            let challenge_route = "/challenge/submit";
            let challenge_result =
                runtime.block_on(challenge_node(&node, &validator_wallet, &challenge_route));
            if challenge_result.is_err() {
                error!(
                    "Failed to challenge node {}: {:?}",
                    node.id, challenge_result
                );
                continue;
            }

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

fn random_challenge(
    rows_a: usize,
    cols_a: usize,
    rows_b: usize,
    cols_b: usize,
) -> ChallengeRequest {
    let mut rng = rng();

    let data_a: Vec<f64> = (0..(rows_a * cols_a))
        .map(|_| rng.random_range(0.0..1.0))
        .collect();

    let data_b: Vec<f64> = (0..(rows_b * cols_b))
        .map(|_| rng.random_range(0.0..1.0))
        .collect();

    ChallengeRequest {
        rows_a,
        cols_a,
        data_a,
        rows_b,
        cols_b,
        data_b,
    }
}

pub async fn challenge_node(
    node: &DiscoveryNode,
    wallet: &Wallet,
    challenge_route: &str,
) -> Result<i32, Box<dyn std::error::Error>> {
    let node_url = format!("http://{}:{}", node.node.ip_address, node.node.port);

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("x-compute", "pytorch".parse().unwrap());

    // create random challenge matrix
    let challenge_matrix = random_challenge(3, 3, 3, 3);

    let a = DMatrix::from_vec(
        challenge_matrix.rows_a,
        challenge_matrix.cols_a,
        challenge_matrix.data_a.clone(),
    );
    let b = DMatrix::from_vec(
        challenge_matrix.rows_b,
        challenge_matrix.cols_b,
        challenge_matrix.data_b.clone(),
    );
    let c = a * b;

    println!("Challenge request: {:?}", challenge_matrix);

    let post_url = format!("{}{}", node_url, challenge_route);
    println!("Challenge post url: {}", post_url);

    let address = wallet.wallet.default_signer().address().to_string();
    let challenge_matrix_value = serde_json::to_value(&challenge_matrix)?;
    let signature = sign_request(challenge_route, &wallet, Some(&challenge_matrix_value)).await?;

    headers.insert("x-address", address.parse().unwrap());
    headers.insert("x-signature", signature.parse().unwrap());

    let response = reqwest::Client::new()
        .post(post_url)
        .json(&challenge_matrix)
        .headers(headers)
        .send()
        .await?;

    println!("Challenge response: {:?}", response);

    let response_text = response.text().await?;
    let parsed_response: ApiResponse<Vec<ChallengeResponse>> =
        serde_json::from_str(&response_text)?;

    println!("Challenge response: {:?}", parsed_response);

    if !parsed_response.success {
        Err("Error fetching challenge from node".into())
    } else if c.data.as_vec().clone() == parsed_response.data[0].result {
        info!("Challenge successful");
        Ok(0)
    } else {
        error!("Challenge failed");
        Err("Node failed challenge".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{test, App};
    use actix_web::{
        web::{self, post},
        HttpResponse, Scope,
    };
    use shared::models::challenge::calc_matrix;

    pub async fn handle_challenge(
        challenge: web::Json<ChallengeRequest>,
        //app_state: Data<AppState>,
    ) -> HttpResponse {
        let result = calc_matrix(&challenge);
        HttpResponse::Ok().json(result)
    }

    pub fn challenge_routes() -> Scope {
        web::scope("/challenge")
            .route("", post().to(handle_challenge))
            .route("/", post().to(handle_challenge))
    }

    #[actix_web::test]
    async fn test_challenge_route() {
        let app = test::init_service(App::new().service(challenge_routes())).await;

        let challenge_request = ChallengeRequest {
            rows_a: 3,
            cols_a: 3,
            data_a: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0],
            rows_b: 3,
            cols_b: 3,
            data_b: vec![9.0, 8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0],
        };

        let req = test::TestRequest::post()
            .uri("/challenge")
            .set_json(&challenge_request)
            .to_request();

        let resp: ChallengeResponse = test::call_and_read_body_json(&app, req).await;
        let expected_response = calc_matrix(&challenge_request);

        assert_eq!(resp, expected_response);
    }
}
