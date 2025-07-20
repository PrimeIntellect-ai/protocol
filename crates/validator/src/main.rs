use clap::Parser;
use log::LevelFilter;
use shared::utils::signal::trigger_cancellation_on_signal;
use tokio_util::sync::CancellationToken;

use validator::Cli;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let log_level = match cli.log_level.as_str() {
        "error" => LevelFilter::Error,
        "warn" => LevelFilter::Warn,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        _ => anyhow::bail!("invalid log level: {}", cli.log_level),
    };
    env_logger::Builder::new()
        .filter_level(log_level)
        .filter_module("tracing::span", log::LevelFilter::Warn)
        .format_timestamp(None)
        .init();

    let cancellation_token = CancellationToken::new();

    let _signal_handle = trigger_cancellation_on_signal(cancellation_token.clone())?;

    cli.run(cancellation_token).await?;

    // TODO: handle spawn handles here (https://github.com/PrimeIntellect-ai/protocol/issues/627)
    Ok(())
}

#[cfg(test)]
mod tests {
    use actix_web::{test, App};
    use actix_web::{
        web::{self, post},
        HttpResponse, Scope,
    };
    use p2p::{calc_matrix, ChallengeRequest, ChallengeResponse, FixedF64};

    async fn handle_challenge(challenge: web::Json<ChallengeRequest>) -> HttpResponse {
        let result = calc_matrix(&challenge);
        HttpResponse::Ok().json(result)
    }

    fn challenge_routes() -> Scope {
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
            timestamp: None,
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
