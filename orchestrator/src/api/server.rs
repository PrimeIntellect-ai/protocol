use crate::api::routes::nodes::nodes_routes;
use crate::{api::routes::heartbeat::heartbeat_routes, store::redis::RedisStore};
use actix_web::{middleware, web::Data, App, HttpServer};
use anyhow::Error;
use std::sync::Arc;

pub struct AppState {
    pub store: Arc<RedisStore>,
}

pub async fn start_server(host: &str, port: u16, store: Arc<RedisStore>) -> Result<(), Error> {
    println!("Starting server at http://{}:{}", host, port);

    let app_state = Data::new(AppState { store });

    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .wrap(middleware::Logger::default())
            .service(heartbeat_routes())
            .service(nodes_routes())
    })
    .bind((host, port))?
    .run()
    .await?;
    Ok(())
}
