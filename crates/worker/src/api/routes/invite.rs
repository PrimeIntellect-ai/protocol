use crate::api::server::AppState;
use crate::console::Console;
use actix_web::{
    web::{self, post, Data},
    HttpResponse, Scope,
};
use alloy::primitives::FixedBytes;
use alloy::primitives::U256;
use alloy::providers::Provider;
use hex;
use log::error;
use serde_json::json;
use shared::models::invite::InviteRequest;
use shared::web3::contracts::structs::compute_pool::PoolStatus;

pub async fn invite_node(
    invite: web::Json<InviteRequest>,
    app_state: Data<AppState>,
) -> HttpResponse {
    if app_state.system_state.is_running().await {
        return HttpResponse::BadRequest().json(json!({
            "error": "Heartbeat is currently running and in a compute pool"
        }));
    }
    if let Some(pool_id) = app_state.system_state.compute_pool_id.clone() {
        if invite.pool_id.to_string() != pool_id {
            return HttpResponse::BadRequest().json(json!({
                "error": "Invalid pool ID"
            }));
        }
    }

    let invite_bytes = match hex::decode(&invite.invite) {
        Ok(bytes) => bytes,
        Err(err) => {
            error!("Failed to decode invite hex string: {:?}", err);
            return HttpResponse::BadRequest().json(json!({
                "error": "Invalid invite format"
            }));
        }
    };

    if invite_bytes.len() < 65 {
        return HttpResponse::BadRequest().json(json!({
            "error": "Invite data is too short"
        }));
    }

    let contracts = &app_state.contracts;
    let wallet = &app_state.node_wallet;
    let pool_id = U256::from(invite.pool_id);

    // Nodes is actually my own node address so I need wallet access
    let bytes_array: [u8; 65] = match invite_bytes[..65].try_into() {
        Ok(array) => array,
        Err(_) => {
            error!("Failed to convert invite bytes to fixed-size array");
            return HttpResponse::BadRequest().json(json!({
                "error": "Invalid invite signature format"
            }));
        }
    };

    let signatures: Vec<FixedBytes<65>> = vec![FixedBytes::from(&bytes_array)];
    let provider_address = app_state.provider_wallet.wallet.default_signer().address();

    let pool_info = match contracts.compute_pool.get_pool_info(pool_id).await {
        Ok(info) => info,
        Err(err) => {
            error!("Failed to get pool info: {:?}", err);
            return HttpResponse::InternalServerError().json(json!({
                "error": "Failed to get pool information"
            }));
        }
    };

    if let PoolStatus::PENDING = pool_info.status {
        Console::user_error("Pool is pending - Invite is invalid");
        return HttpResponse::BadRequest().json(json!({
            "error": "Pool is pending - Invite is invalid"
        }));
    }

    let max_tries = 3;
    let mut tries = 0;
    let mut gas_price: Option<u128> = None;

    while tries < max_tries {
        let node_address = vec![wallet.wallet.default_signer().address()];
        let signatures = vec![FixedBytes::from(&bytes_array)];
        let mut call = match contracts.compute_pool.build_join_compute_pool_call(
            pool_id,
            provider_address,
            node_address,
            signatures,
        ) {
            Ok(call) => call,
            Err(err) => {
                error!("Failed to build join compute pool call: {:?}", err);
                return HttpResponse::InternalServerError().json(json!({
                    "error": "Failed to build join compute pool call"
                }));
            }
        };

        if let Some(gas_price) = gas_price {
            call = call.gas_price(gas_price);
        }

        let result = call.send().await;

        match result {
            Ok(result) => {
                // TODO: Timeout handling missing
                let join_success = result.watch().await;
                if join_success.is_ok() {
                    Console::success("Successfully joined compute pool");
                    break;
                } else {
                    error!("Failed to join compute pool: {:?}", join_success);
                    tries += 1;
                }
            }
            Err(err) => {
                error!("Failed to join compute pool: {:?}", err);
                if err
                    .to_string()
                    .contains("replacement transaction underpriced")
                {
                    tries += 1;
                    Console::user_error("Replacement transaction underpriced - Retrying...");
                    match app_state.provider_wallet.provider.get_gas_price().await {
                        Ok(price) => {
                            gas_price = Some(price * (110 + tries * 10) / 100);
                        }
                        Err(err) => {
                            error!("Failed to get gas price: {:?}", err);
                            return HttpResponse::InternalServerError().json(json!({
                                "error": "Failed to get gas price"
                            }));
                        }
                    }
                } else {
                    error!("Failed to join compute pool: {:?}", err);
                    return HttpResponse::InternalServerError().json(json!({
                        "error": "Failed to join compute pool"
                    }));
                }
            }
        }
    }

    let endpoint = if let Some(url) = &invite.master_url {
        format!("{}/heartbeat", url)
    } else {
        match (&invite.master_ip, &invite.master_port) {
            (Some(ip), Some(port)) => format!("http://{}:{}/heartbeat", ip, port),
            _ => {
                error!("Missing master IP or port in invite request");
                return HttpResponse::BadRequest().json(json!({
                    "error": "Missing master IP or port"
                }));
            }
        }
    };

    if let Err(err) = app_state.heartbeat_service.start(endpoint).await {
        error!("Failed to start heartbeat service: {:?}", err);
        return HttpResponse::InternalServerError().json(json!({
            "error": "Failed to start heartbeat service"
        }));
    }

    HttpResponse::Accepted().json(json!({
        "status": "ok"
    }))
}

pub fn invite_routes() -> Scope {
    web::scope("/invite")
        .route("", post().to(invite_node))
        .route("/", post().to(invite_node))
}
