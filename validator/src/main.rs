pub mod validators;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use alloy::primitives::utils::Unit;
use alloy::primitives::{Address, U256};
use anyhow::{Context, Result};
use clap::Parser;
use log::LevelFilter;
use log::{error, info};
use serde_json::json;
use shared::models::api::ApiResponse;
use shared::models::node::DiscoveryNode;
use shared::security::request_signer::sign_request;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use std::sync::Arc;
use url::Url;
use validators::hardware::HardwareValidator;
use validators::synthetic_data::SyntheticDataValidator;

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

    /// Optional: Work validation contract address
    #[arg(long, default_value = None)]
    work_validation_contract: Option<String>,

    /// Optional: Pool Id for work validation
    /// If not provided, the validator will not validate work
    #[arg(long, default_value = None)]
    pool_id: Option<String>,

    /// Optional: Work validation interval in seconds
    #[arg(long, default_value = "30")]
    work_validation_interval: u64,

    /// Optional: Leviticus Validator URL
    #[arg(long, default_value = None)]
    leviticus_url: Option<String>,

    /// Optional: Leviticus Auth Token
    #[arg(long, default_value = None)]
    leviticus_token: Option<String>,

    /// Optional: Validator penalty in whole tokens
    /// Note: This value will be multiplied by 10^18 (1 token = 10^18 wei)
    #[arg(long, default_value = "1000")]
    validator_penalty: u64,
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

    let work_validation_address: Option<Address> = args
        .work_validation_contract
        .map(|address| address.parse::<Address>().unwrap());

    let contracts = ContractBuilder::new(&validator_wallet)
        .with_compute_registry()
        .with_ai_token()
        .with_prime_network()
        .with_compute_pool()
        .with_synthetic_data_validator(work_validation_address)
        .build()
        .unwrap();

    let contracts = Arc::new(contracts);
    let hardware_validator = HardwareValidator::new(&validator_wallet, contracts.clone());

    let mut synthetic_validator = if let Some(pool_id) = args.pool_id.clone() {
        let penalty = U256::from(args.validator_penalty) * Unit::ETHER.wei();
        match contracts.synthetic_data_validator.clone() {
            Some(validator) => {
                if let Some(leviticus_url) = args.leviticus_url {
                    Some(SyntheticDataValidator::new(
                        None,
                        pool_id,
                        validator,
                        contracts.prime_network.clone(),
                        leviticus_url,
                        args.leviticus_token,
                        penalty,
                    ))
                } else {
                    error!("Leviticus URL is not provided");
                    std::process::exit(1);
                }
            }
            None => {
                error!("Synthetic data validator not found");
                std::process::exit(1);
            }
        }
    } else {
        None
    };

    loop {
        if let Some(validator) = &mut synthetic_validator {
            runtime.block_on(async {
                if let Err(e) = validator.validate_work().await {
                    error!("Failed to validate work: {}", e);
                }
            });
        }

        async fn _generate_signature(wallet: &Wallet, message: &str) -> Result<String> {
            let signature = sign_request(message, wallet, None)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            Ok(signature)
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

        if let Err(e) = runtime.block_on(hardware_validator.validate_nodes(nodes)) {
            error!("Error validating nodes: {:#}", e);
        }
    }
}

#[cfg(test)]
mod tests {

    use actix_web::{test, App};
    use actix_web::{
        web::{self, post},
        HttpResponse, Scope,
    };
    use shared::models::challenge::{calc_matrix, ChallengeRequest, ChallengeResponse, FixedF64};

    pub async fn handle_challenge(challenge: web::Json<ChallengeRequest>) -> HttpResponse {
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

        let vec_a = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        let vec_b = [9.0, 8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0];

        // convert vectors to FixedF64
        let data_a: Vec<FixedF64> = vec_a.iter().map(|x| FixedF64(*x)).collect();
        let data_b: Vec<FixedF64> = vec_b.iter().map(|x| FixedF64(*x)).collect();

        let challenge_request = ChallengeRequest {
            rows_a: 3,
            cols_a: 3,
            data_a,
            rows_b: 3,
            cols_b: 3,
            data_b,
        };

        let req = test::TestRequest::post()
            .uri("/challenge")
            .set_json(&challenge_request)
            .to_request();

        let resp: ChallengeResponse = test::call_and_read_body_json(&app, req).await;
        let expected_response = calc_matrix(&challenge_request);

        assert_eq!(resp.result, expected_response.result);
    }
}
