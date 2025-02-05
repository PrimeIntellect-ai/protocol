use log::{debug, error, info, warn};
use crate::api::server::AppState;
use actix_web::{
    web::{self, post, Data},
    HttpResponse, Scope,
};
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
    HttpResponse::Ok().json(result)
}

pub fn challenge_routes() -> Scope {
    web::scope("/challenge").route("/submit", post().to(handle_challenge))
}
