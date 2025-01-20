use crate::api::routes::heartbeat::heartbeat_routes;
use actix_web::{middleware, App, HttpServer};
use anyhow::Error;

pub async fn start_server(host: &str, port: u16) -> Result<(), Error> {
    println!("Starting server at http://{}:{}", host, port);

    HttpServer::new(move || {
        App::new()
            .wrap(middleware::Logger::default())
            .service(heartbeat_routes())
    })
    .bind((host, port))?
    .run()
    .await?;
    Ok(())
}
