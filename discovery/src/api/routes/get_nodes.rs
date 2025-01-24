use crate::api::server::AppState;
use shared::models::node::Node;
use actix_web::{
    web::Data,
    web::{self, get},
    HttpResponse,
};

pub async fn get_nodes(data: Data<AppState>) -> HttpResponse {
    let nodes = data.node_store.get_nodes();
    HttpResponse::Ok().json(nodes)
}

pub async fn get_nodes_for_pool(data: Data<AppState>, pool_id: web::Path<String>) -> HttpResponse {
    let nodes = data.node_store.get_nodes();
    let pool_id = pool_id.into_inner().parse::<u32>().unwrap();
    let nodes_for_pool: Vec<Node> = nodes
        .iter()
        .filter(|node| node.compute_pool_id == pool_id)
        .cloned()
        .collect();
    HttpResponse::Ok().json(nodes_for_pool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::node_store::NodeStore;
    use crate::store::redis::RedisStore;
    use actix_web::test;
    use actix_web::App;
    use std::sync::Arc;

    #[actix_web::test]
    async fn test_get_nodes() {
        let app_state = AppState {
            node_store: Arc::new(NodeStore::new(RedisStore::new_test())),
        };
        let app = test::init_service(
            App::new()
                .app_data(Data::new(app_state.clone()))
                .route("/nodes", get().to(get_nodes)),
        )
        .await;

        let sample_node = Node {
            id: "0x32A8dFdA26948728e5351e61d62C190510CF1C88".to_string(),
            provider_address: None,
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            compute_pool_id: 0,
            last_seen: None,
            compute_specs: None,
        };
        app_state.node_store.register_node(sample_node);

        let req = test::TestRequest::get().uri("/nodes").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        let body = test::read_body(resp).await;
        let nodes: Vec<Node> = serde_json::from_slice(&body).unwrap();
        assert_eq!(nodes.len(), 1);
    }

    #[actix_web::test]
    async fn test_get_nodes_for_pool() {
        let app_state = AppState {
            node_store: Arc::new(NodeStore::new(RedisStore::new_test())),
        };
        let app = test::init_service(
            App::new()
                .app_data(Data::new(app_state.clone()))
                .route("/nodes/pool/{pool_id}", get().to(get_nodes_for_pool)),
        )
        .await;

        let sample_node = Node {
            id: "0x32A8dFdA26948728e5351e61d62C190510CF1C88".to_string(),
            provider_address: None,
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            compute_pool_id: 1,
            last_seen: None,
            compute_specs: None,
        };
        app_state.node_store.register_node(sample_node);

        let req = test::TestRequest::get().uri("/nodes/pool/0").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        let body = test::read_body(resp).await;
        let nodes: Vec<Node> = serde_json::from_slice(&body).unwrap();
        assert_eq!(nodes.len(), 0);
    }

    #[actix_web::test]
    async fn test_get_nodes_for_pool_with_pool_id() {
        let app_state = AppState {
            node_store: Arc::new(NodeStore::new(RedisStore::new_test())),
        };
        let app = test::init_service(
            App::new()
                .app_data(Data::new(app_state.clone()))
                .route("/nodes/pool/{pool_id}", get().to(get_nodes_for_pool)),
        )
        .await;

        let sample_node = Node {
            id: "0x32A8dFdA26948728e5351e61d62C190510CF1C88".to_string(),
            provider_address: None,
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            compute_pool_id: 0,
            last_seen: None,
            compute_specs: None,
        };
        app_state.node_store.register_node(sample_node);

        let req = test::TestRequest::get().uri("/nodes/pool/0").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        let body = test::read_body(resp).await;
        let nodes: Vec<Node> = serde_json::from_slice(&body).unwrap();
        assert_eq!(nodes.len(), 1);
    }
}
