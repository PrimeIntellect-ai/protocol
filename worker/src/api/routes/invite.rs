use crate::api::server::AppState;
use crate::console::Console;
use actix_web::{
    web::{self, post, Data},
    HttpResponse, Scope,
};
use alloy::primitives::FixedBytes;
use alloy::primitives::U256;
use hex;
use log::error;
use serde_json::json;
use shared::models::invite::InviteRequest;
use shared::web3::contracts::structs::compute_pool::PoolStatus;

pub async fn invite_node(
    invite: web::Json<InviteRequest>,
    app_state: Data<AppState>,
) -> HttpResponse {
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
    let node_address = vec![wallet.wallet.default_signer().address()];
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

    match contracts
        .compute_pool
        .join_compute_pool(pool_id, provider_address, node_address, signatures)
        .await
    {
        Ok(result) => {
            Console::success(&format!("Successfully joined compute pool: {:?}", result));
        }
        Err(err) => {
            error!("Failed to join compute pool: {:?}", err);
            return HttpResponse::InternalServerError().json(json!({
                "error": "Failed to join compute pool"
            }));
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
