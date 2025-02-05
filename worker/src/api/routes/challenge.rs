use crate::api::server::AppState;
use actix_web::{
    web::{self, post, Data},
    HttpResponse, Scope,
};
use log::info;
use shared::models::api::ApiResponse;
use shared::models::challenge::calc_matrix;
use shared::models::challenge::ChallengeRequest;

pub async fn handle_challenge(
    challenge: web::Json<ChallengeRequest>,
    app_state: Data<AppState>,
) -> HttpResponse {
    info!(
        "Wallet: {:?}",
        app_state.node_wallet.wallet.default_signer().address()
    );
    let result = calc_matrix(&challenge);

    let response = ApiResponse::new(true, result);

    HttpResponse::Ok().json(response)
}

pub fn challenge_routes() -> Scope {
    web::scope("/challenge").route("/submit", post().to(handle_challenge))
}
