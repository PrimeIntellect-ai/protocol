use crate::api::server::AppState;
use shared::models::node::Node;
use actix_web::{
    web::{self, post, Data},
    HttpResponse, Scope,
};

pub async fn register_node(node: web::Json<Node>, data: Data<AppState>) -> HttpResponse {
    let node_store = data.node_store.clone();
    node_store.register_node(node.clone());
    println!("Node: {:?}", node);
    HttpResponse::Ok().json("Node registered successfully")
}

pub fn node_routes() -> Scope {
    web::scope("/nodes").route("", post().to(register_node))
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
            provider_address: None,
            ip_address: "127.0.0.1".to_string(),
            port: 8089,
            compute_pool_id: 0,
            last_seen: None,
            compute_specs: None,
        };

        let app_state = AppState {
            node_store: Arc::new(NodeStore::new(RedisStore::new_test())),
        };

        let app = test::init_service(
            App::new()
                .app_data(Data::new(app_state.clone()))
                .route("/nodes", post().to(register_node)),
        )
        .await;

        let node_clone = node.clone();
        let json = serde_json::to_value(node).unwrap();
        let deserialized_node: Node = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(deserialized_node, node_clone);

        let req = test::TestRequest::post()
            .uri("/nodes")
            .set_json(json)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let nodes = app_state.node_store.get_nodes();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0], node_clone);
    }
}
