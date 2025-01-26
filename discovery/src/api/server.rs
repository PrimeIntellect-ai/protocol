use crate::api::routes::get_nodes::{get_node_by_subkey, get_nodes, get_nodes_for_pool};
use crate::api::routes::node::node_routes;
use crate::store::node_store::NodeStore;
use actix_web::{
    middleware,
    web::Data,
    web::{self, get},
    App, HttpServer,
};
use log::info;
use shared::security::auth_signature_middleware::{ValidateSignature, ValidatorState};
use shared::web3::contracts::core::builder::Contracts;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub node_store: Arc<NodeStore>,
    pub contracts: Option<Arc<Contracts>>,
}

pub async fn start_server(
    host: &str,
    port: u16,
    node_store: Arc<NodeStore>,
    contracts: Arc<Contracts>,
    validator_address: String,
) -> std::io::Result<()> {
    info!("Starting server at http://{}:{}", host, port);

    let app_state = AppState {
        node_store,
        contracts: Some(contracts),
    };

    // it seems we have a validator for the validator
    let validator_validator = Arc::new(ValidatorState::new(vec![validator_address
        .parse()
        .unwrap()]));

    // All nodes can register as long as they have a valid signature
    let validate_signatures = Arc::new(ValidatorState::new(vec![]).with_validator(move |_| true));

    // TODO: Platform validation - see issue
    HttpServer::new(move || {
        App::new()
            .wrap(middleware::Logger::default())
            .app_data(Data::new(app_state.clone()))
            .service(web::scope("/api/nodes/{node_id}").route("", get().to(get_node_by_subkey)))
            .service(node_routes().wrap(ValidateSignature::new(validate_signatures.clone())))
            .service(web::scope("/api/platform").route("", get().to(get_nodes)))
            .service(
                web::scope("/api/validator")
                    .wrap(ValidateSignature::new(validator_validator.clone()))
                    .route("", web::get().to(get_nodes)),
            )
            .service(
                web::scope("/api/pool/{pool_id}")
                    .wrap(ValidateSignature::new(validate_signatures.clone()))
                    .route("", get().to(get_nodes_for_pool)),
            )
    })
    .bind((host, port))?
    .run()
    .await
}
