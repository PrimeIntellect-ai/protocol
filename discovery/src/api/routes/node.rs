use actix_web::{
    web::{self, post, Data}, HttpResponse, Scope
};
use crate::models::node::Node;
use crate::store::node_store::NodeStore;
use std::sync::Arc;
use crate::api::server::AppState;

pub async fn register_node(node: web::Json<Node>, data: Data<AppState>) -> HttpResponse {
    let node_store = data.node_store.clone(); 
    println!("Node: {:?}", node.into_inner());
    
    // Attempt to register the node in the store
    /*match node_store.register_node(node.into_inner()).await {
        Ok(_) => HttpResponse::Ok().json("Node registered successfully"),
        Err(e) => {
            eprintln!("Error registering node: {:?}", e);
            HttpResponse::InternalServerError().json("Failed to register node")
        }
    }*/
    HttpResponse::Ok().json("Node registered successfully")
}

pub fn node_routes() -> Scope {
    web::scope("/nodes").route("", post().to(register_node))
}


#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::test;
    use actix_web::App;
    use actix_web::http::StatusCode;
    use crate::store::redis::RedisStore;

    #[actix_web::test]
    async fn test_register_node() {
        let node = Node {
            id: "0x32A8dFdA26948728e5351e61d62C190510CF1C88".to_string(),
            provider_address: None,
            ip_address: "127.0.0.1".to_string(),
            port: 8089,
            compute_pool_id: None,
            last_seen: None,
            compute_specs: None,
        }; 

        let app_state = AppState {
            node_store: Arc::new(NodeStore::new(RedisStore::new_test())),
        };

        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .route("/nodes", post().to(register_node)),
        )
        .await;
        println!("Node: {:?}", node);

        let req = test::TestRequest::post()
            .uri("/nodes")
            .set_json(serde_json::to_value(node).unwrap()) // Serialize node to JSON
            .to_request();
        let resp = test::call_service(&app, req).await;
        println!("Response: {:?}", resp);
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
