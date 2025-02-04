use crate::api::server::AppState;
use crate::console::Console;
use actix_web::{
    web::{self, post, Data},
    HttpResponse, Scope,
};
use alloy::primitives::FixedBytes;
use alloy::primitives::U256;
use hex;
use serde_json::json;
use shared::models::invite::InviteRequest;
use shared::web3::contracts::structs::compute_pool::PoolStatus;

pub async fn invite_node(
    invite: web::Json<InviteRequest>,
    app_state: Data<AppState>,
) -> HttpResponse {
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
        Console::error("Pool is pending - Invite is invalid");
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
            Console::info("Starting to send heartbeats now.", "");
        }
        Err(err) => {
            Console::error(&format!("Error joining compute pool: {:?}", err));
            return HttpResponse::InternalServerError().json(json!({
                "error": "Failed to join compute pool"
            }));
        }
    }

    let endpoint = if let Some(url) = &invite.master_url {
        format!("{}/heartbeat", url)
    } else {
        format!(
            "http://{}:{}/heartbeat",
            &invite.master_ip.as_ref().unwrap(),
            &invite.master_port.as_ref().unwrap()
        )
    };

    println!("Starting heartbeat service with endpoint: {}", endpoint);
    let _ = app_state.heartbeat_service.start(endpoint).await;

    HttpResponse::Accepted().json(json!({
        "status": "ok"
    }))
}
pub fn invite_routes() -> Scope {
    web::scope("/invite")
        .route("", post().to(invite_node))
        .route("/", post().to(invite_node))
}
