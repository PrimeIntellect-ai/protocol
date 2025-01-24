use actix_web::{middleware, web::Data, App, HttpServer};
use std::sync::Arc;
use crate::api::routes::node::node_routes;
use crate::api::routes::platform::platform_routes;
use crate::store::node_store::NodeStore;

#[derive(Clone)]
pub struct AppState {
 pub    node_store: Arc<NodeStore>,
}

pub async fn start_server(
    host: &str,
    port: u16,
    node_store: Arc<NodeStore>,
) -> std::io::Result<()> {
    println!("Starting server at http://{}:{}", host, port);

    let app_state = AppState { node_store };

    HttpServer::new(move || {
        App::new()
            .wrap(middleware::Logger::default())
            .app_data(Data::new(app_state.clone()))
            .service(node_routes())
            .service(platform_routes())
    })
    .bind((host, port))?
    .run()
    .await
}
