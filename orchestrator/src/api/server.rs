use crate::api::routes::heartbeat::heartbeat_routes;
use crate::api::routes::nodes::nodes_routes;
use crate::api::routes::task::tasks_routes;
use crate::store::core::StoreContext;
use actix_web::{middleware, web::Data, App, HttpServer};
use anyhow::Error;
use log::info;
use shared::security::auth_signature_middleware::{ValidateSignature, ValidatorState};
use std::sync::Arc;

pub struct AppState {
    pub store_context: Arc<StoreContext>,
}

pub async fn start_server(
    host: &str,
    port: u16,
    store_context: Arc<StoreContext>,
) -> Result<(), Error> {
    info!("Starting server at http://{}:{}", host, port);
    let app_state = Data::new(AppState { store_context });
    let node_store = app_state.store_context.node_store.clone();
    let node_store_clone = node_store.clone();
    let validator_state = Arc::new(
        ValidatorState::new(vec![]).with_validator(move |address| {
            node_store_clone.get_node(address).is_some()
        }),
    );

    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .wrap(middleware::Logger::default())
            .service(heartbeat_routes().wrap(ValidateSignature::new(validator_state.clone())))
            .service(nodes_routes())
            .service(tasks_routes())
    })
    .bind((host, port))?
    .run()
    .await?;
    Ok(())
}
