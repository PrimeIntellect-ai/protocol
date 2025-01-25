use crate::api::server::AppState;
use actix_web::{
    web::{self, put, Data},
    HttpResponse, Scope,
};
use shared::models::api::ApiResponse;
use shared::models::node::Node;

pub async fn register_node(node: web::Json<Node>, data: Data<AppState>, req: actix_web::HttpRequest) -> HttpResponse {
    let node_store = data.node_store.clone();
    
    // Check for the x-address header
    let address_str = match req.headers().get("x-address") {
        Some(address) => match address.to_str() {
            Ok(addr) => {
                println!("Received x-address header: {}", addr);
                addr.to_string()
            }
            Err(_) => return HttpResponse::BadRequest().json(ApiResponse::new(false, "Invalid x-address header")),
        },
        None => return HttpResponse::BadRequest().json(ApiResponse::new(false, "Missing x-address header")),
    };

    println!("Comparing {} with payload id: {}", address_str, node.id);
    if address_str != node.id {
        return HttpResponse::BadRequest().json(ApiResponse::new(false, "Invalid x-address header"));
    }

    node_store.register_node(node.clone());
    println!("Node: {:?}", node);
    HttpResponse::Ok().json(ApiResponse::new(true, "Node registered successfully"))
}

pub fn node_routes() -> Scope {
    web::scope("/api/nodes").route("", put().to(register_node))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::node_store::NodeStore;
    use crate::store::redis::RedisStore;
    use actix_web::http::StatusCode;
    use actix_web::test;
    use actix_web::App;
    use std::sync::Arc;

    #[actix_web::test]
    async fn test_register_node() {
        let node = Node {
            id: "0x32A8dFdA26948728e5351e61d62C190510CF1C88".to_string(),
            provider_address: "0x32A8dFdA26948728e5351e61d62C190510CF1C88".to_string(),
            ip_address: "127.0.0.1".to_string(),
            port: 8089,
            compute_pool_id: 0,
            compute_specs: None,
        };

        let app_state = AppState {
            node_store: Arc::new(NodeStore::new(RedisStore::new_test())),
            contracts: None,
        };

        let app = test::init_service(
            App::new()
                .app_data(Data::new(app_state.clone()))
                .route("/nodes", put().to(register_node)),
        )
        .await;

        let json = serde_json::to_value(node.clone()).unwrap();
        let req = test::TestRequest::put()
            .uri("/nodes")
            .set_json(json)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body: ApiResponse<String> = test::read_body_json(resp).await;
        assert!(body.success);
        assert_eq!(body.data, "Node registered successfully");

        let nodes = app_state.node_store.get_nodes();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].id, node.id);
    }
}
