use crate::api::routes::get_nodes::{get_nodes, get_nodes_for_pool};
use crate::api::routes::node::node_routes;
use crate::store::node_store::NodeStore;
use actix_web::{
    middleware,
    web::Data,
    web::{self, get},
    App, HttpServer,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub node_store: Arc<NodeStore>,
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
            .service(web::scope("/api/platform").route("", get().to(get_nodes)))
            .service(web::scope("/api/validator").route("", get().to(get_nodes)))
            .service(web::scope("/api/pool/{pool_id}").route("", get().to(get_nodes_for_pool)))
    })
    .bind((host, port))?
    .run()
    .await
}
