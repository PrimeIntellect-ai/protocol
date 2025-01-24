use actix_web::{
    web::{self, get, Data},
    HttpResponse, Scope, HttpRequest
};

pub async fn get_nodes_for_platform() -> HttpResponse {
    HttpResponse::Ok().json("Hello, world!")
}

pub fn platform_routes() -> Scope {
    web::scope("/platform").route("", get().to(get_nodes_for_platform))
}
