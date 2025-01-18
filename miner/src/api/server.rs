use crate::api::routes::invite::invite_routes;
use crate::api::routes::task::task_routes;
use crate::api::middleware::validate_signature::ValidateSignature;
use crate::api::middleware::validate_signature::ValidateSignatureMiddleware;
use actix_web::{middleware, web::Data, App, Error, HttpServer};
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::wallet::Wallet;
use std::sync::Arc; // Import Arc for shared ownership

#[derive(Clone)]
pub struct AppState {
    pub contracts: Arc<Contracts>,
    pub node_wallet: Arc<Wallet>,
    pub provider_wallet: Arc<Wallet>,
}
use actix_web::{dev::Service, HttpRequest, HttpResponse, Result, web};


pub async fn start_server(
    host: &str,
    port: u16,
    contracts: Arc<Contracts>,
    node_wallet: Arc<Wallet>,
    provider_wallet: Arc<Wallet>,
) -> std::io::Result<()> {
    println!("Starting server at http://{}:{}", host, port);
    let app_state = Data::new(AppState {
        contracts,
        node_wallet,
        provider_wallet,
    });

    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .wrap(middleware::Logger::default())
            .wrap(ValidateSignature)
            // TODO: Add authentication middleware
            // TODO: Add rate limiting middleware
            .service(task_routes())
            .service(invite_routes())
    })
    .bind((host, port))?
    .run()
    .await
}
