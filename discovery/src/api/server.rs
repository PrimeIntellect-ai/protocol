use crate::api::routes::node::node_routes;
use crate::models::node::Node;
use crate::store::node_store::NodeStore;
use actix_web::{
    middleware,
    web::Data,
    web::{self, get},
    App, HttpResponse, HttpServer,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub node_store: Arc<NodeStore>,
}

pub async fn get_nodes(data: Data<AppState>) -> HttpResponse {
    let nodes = data.node_store.get_nodes();
    HttpResponse::Ok().json(nodes)
}

pub async fn get_nodes_for_pool(data: Data<AppState>, pool_id: web::Path<String>) -> HttpResponse {
    let nodes = data.node_store.get_nodes();
    let pool_id = pool_id.into_inner().parse::<u32>().unwrap();
    let nodes_for_pool: Vec<Node> = nodes
        .iter()
        .filter(|node| node.compute_pool_id == pool_id).cloned()
        .collect();
    HttpResponse::Ok().json(nodes_for_pool)
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
            .service(web::scope("/platform").route("", get().to(get_nodes)))
            .service(web::scope("/validator").route("", get().to(get_nodes)))
            .service(web::scope("/pool/{pool_id}").route("", get().to(get_nodes_for_pool)))
    })
    .bind((host, port))?
    .run()
    .await
}
