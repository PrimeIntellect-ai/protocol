use crate::api::models::invite::InviteRequest;
use crate::api::server::AppState;
use actix_web::{
    web::{self, delete, get, post, Data},
    HttpRequest, HttpResponse, Scope,
};
use alloy::primitives::FixedBytes;
use alloy::primitives::U256;
use hex;
use log::{debug, info};
use serde::{Deserialize, Serialize};
use serde_json::json;
use shared::web3::contracts::structs::compute_pool::PoolStatus;
use validator::Validate;
use alloy::primitives::PrimitiveSignature;
use std::str::FromStr; 
use alloy::primitives::Address; 

pub async fn invite_node(
    invite: web::Json<InviteRequest>,
    app_state: Data<AppState>,
) -> HttpResponse {
    // Validate request
    if let Err(errors) = invite.validate() {
        return HttpResponse::BadRequest().json(json!({
            "error": errors.to_string(),
        }));
    }

    // Tasks to finish compute onboarding mvp 
    // TODO: Implement logic to verify the sender - maybe middleware? - Added bug looks like shit right now codewise
    // TODO: Check if we actually want to join the pool - based on the compute pool var that we set on cmd startup
    // TODO: Start heartbeat sending incl. state store logic
    // TODO: Check for hardcoded values on orchestrator side
    // TODO: Fix redis setup on provider side 

    // Future Questions:
    // TODO: What happens when a compute pools switches to be finished?


    let invite_bytes = hex::decode(invite.invite.clone()).unwrap();
    let contracts = &app_state.contracts;
    let wallet = &app_state.node_wallet;
    let pool_id = U256::from(invite.pool_id);

    // Nodes is actually my own node address so I need wallet access
    let bytes_array: &[u8; 65] = invite_bytes[..65].try_into().unwrap();
    let signatures: Vec<FixedBytes<65>> = vec![FixedBytes::from(bytes_array)];
    let node_address = vec![wallet.wallet.default_signer().address()];
    let provider_address = app_state.provider_wallet.wallet.default_signer().address();

    let pool_info = contracts.compute_pool.get_pool_info(pool_id).await.unwrap();
    if let PoolStatus::PENDING = pool_info.status {
        println!("Pool is pending - Invite is invalid");
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
            println!("Successfully joined compute pool: {:?}", result);
            println!("Starting to send heartbeats now.");
        }
        Err(err) => {
            println!("Error joining compute pool: {:?}", err);
            return HttpResponse::InternalServerError().json(json!({
                "error": "Failed to join compute pool"
            }));
        }
    }

    // TODO: Start heartbeat sending

    HttpResponse::Accepted().json(json!({
        "status": "ok"
    }))
}
pub fn invite_routes() -> Scope {
    web::scope("/invite")
        .route("", post().to(invite_node))
        .route("/", post().to(invite_node))
}
