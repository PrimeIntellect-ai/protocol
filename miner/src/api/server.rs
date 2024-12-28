use crate::api::routes::task::task_routes;
use actix_web::{middleware, App, HttpServer};

pub async fn start_server(host: &str, port: u16) -> std::io::Result<()> {
    println!("Starting server at http://{}:{}", host, port);

    HttpServer::new(|| {
        App::new()
            // Add middleware
            .wrap(middleware::Logger::default())
            // TODO: Add authentication middleware
            // TODO: Add rate limiting middleware
            // Register routes
            .service(task_routes())
    })
    .bind((host, port))?
    .run()
    .await
}
