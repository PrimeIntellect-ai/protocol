use actix_web::{
    web::{self, post},
    HttpResponse, Scope,
};
use shared::models::challenge::calc_matrix;
use shared::models::challenge::ChallengeRequest;

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
