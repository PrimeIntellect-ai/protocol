use crate::api::server::AppState;
use actix_web::{
    web::{self, put, Data},
    HttpResponse, Scope,
};
use shared::models::api::ApiResponse;
use shared::models::node::Node;

pub async fn register_node(
    node: web::Json<Node>,
    data: Data<AppState>,
    req: actix_web::HttpRequest,
) -> HttpResponse {
    let node_store = data.node_store.clone();

    // Check for the x-address header
    let address_str = match req.headers().get("x-address") {
        Some(address) => match address.to_str() {
            Ok(addr) => addr.to_string(),
            Err(_) => {
                return HttpResponse::BadRequest()
                    .json(ApiResponse::new(false, "Invalid x-address header"))
            }
        },
        None => {
            return HttpResponse::BadRequest()
                .json(ApiResponse::new(false, "Missing x-address header"))
        }
    };

    if address_str != node.id {
        return HttpResponse::BadRequest()
            .json(ApiResponse::new(false, "Invalid x-address header"));
    }

    node_store.register_node(node.clone());
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
    use shared::models::node::DiscoveryNode;
    use shared::security::auth_signature_middleware::{ValidateSignature, ValidatorState};
    use shared::security::request_signer::sign_request;
    use shared::web3::wallet::Wallet;
    use std::sync::Arc;
    use url::Url;

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
            .insert_header(("x-address", "wrong_address")) // Set header to an incorrect address
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST); // Expecting a Bad Request response

        let body: ApiResponse<String> = test::read_body_json(resp).await;
        assert!(!body.success);
        assert_eq!(body.data, "Invalid x-address header"); // Expecting the appropriate error message
    }

    #[actix_web::test]
    async fn test_register_node_already_validated() {
        let private_key = "0000000000000000000000000000000000000000000000000000000000000001";
        let node = Node {
            id: "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf".to_string(),
            provider_address: "0x32A8dFdA26948728e5351e61d62C190510CF1C88".to_string(),
            ip_address: "127.0.0.1".to_string(),
            port: 8089,
            compute_pool_id: 0,
            compute_specs: None,
        };

        let node_clone_for_recall = node.clone();

        let app_state = AppState {
            node_store: Arc::new(NodeStore::new(RedisStore::new_test())),
            contracts: None,
        };

        let validate_signatures =
            Arc::new(ValidatorState::new(vec![]).with_validator(move |_| true));
        let app = test::init_service(
            App::new()
                .app_data(Data::new(app_state.clone()))
                .route("/nodes", put().to(register_node))
                .wrap(ValidateSignature::new(validate_signatures.clone())),
        )
        .await;

        let json = serde_json::to_value(node.clone()).unwrap();
        let signature = sign_request(
            "/nodes",
            &Wallet::new(private_key, Url::parse("http://localhost:8080").unwrap()).unwrap(),
            Some(&json),
        )
        .await
        .unwrap();

        let req = test::TestRequest::put()
            .uri("/nodes")
            .set_json(json)
            .insert_header(("x-address", node.id.clone()))
            .insert_header(("x-signature", signature))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body: ApiResponse<String> = test::read_body_json(resp).await;
        assert!(body.success);
        assert_eq!(body.data, "Node registered successfully");

        let nodes = app_state.node_store.get_nodes();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].id, node.id);

        let validated = DiscoveryNode {
            node: node,
            is_validated: true,
            is_active: true,
        };

        app_state.node_store.update_node(validated);

        let nodes = app_state.node_store.get_nodes();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].id, node_clone_for_recall.id);
        assert_eq!(nodes[0].is_validated, true);
        assert_eq!(nodes[0].is_active, true);

        let json = serde_json::to_value(node_clone_for_recall.clone()).unwrap();
        let signature = sign_request(
            "/nodes",
            &Wallet::new(private_key, Url::parse("http://localhost:8080").unwrap()).unwrap(),
            Some(&json),
        )
        .await
        .unwrap();

        let req = test::TestRequest::put()
            .uri("/nodes")
            .set_json(json)
            .insert_header(("x-address", node_clone_for_recall.id.clone()))
            .insert_header(("x-signature", signature))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let nodes = app_state.node_store.get_nodes();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].id, node_clone_for_recall.id);
        assert_eq!(nodes[0].is_validated, true);
        assert_eq!(nodes[0].is_active, true);
    }

    #[actix_web::test]
    async fn test_register_node_with_correct_signature() {
        let private_key = "0000000000000000000000000000000000000000000000000000000000000001";
        let node = Node {
            id: "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf".to_string(),
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

        let validate_signatures =
            Arc::new(ValidatorState::new(vec![]).with_validator(move |_| true));
        let app = test::init_service(
            App::new()
                .app_data(Data::new(app_state.clone()))
                .route("/nodes", put().to(register_node))
                .wrap(ValidateSignature::new(validate_signatures.clone())),
        )
        .await;

        let json = serde_json::to_value(node.clone()).unwrap();
        let signature = sign_request(
            "/nodes",
            &Wallet::new(private_key, Url::parse("http://localhost:8080").unwrap()).unwrap(),
            Some(&json),
        )
        .await
        .unwrap();

        let req = test::TestRequest::put()
            .uri("/nodes")
            .set_json(json)
            .insert_header(("x-address", node.id.clone()))
            .insert_header(("x-signature", signature))
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

    #[actix_web::test]
    async fn test_register_node_with_incorrect_signature() {
        let private_key = "0000000000000000000000000000000000000000000000000000000000000001";
        let node = Node {
            id: "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdd".to_string(),
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

        let validate_signatures =
            Arc::new(ValidatorState::new(vec![]).with_validator(move |_| true));
        let app = test::init_service(
            App::new()
                .app_data(Data::new(app_state.clone()))
                .route("/nodes", put().to(register_node))
                .wrap(ValidateSignature::new(validate_signatures.clone())),
        )
        .await;

        let json = serde_json::to_value(node.clone()).unwrap();
        let signature = sign_request(
            "/nodes",
            &Wallet::new(private_key, Url::parse("http://localhost:8080").unwrap()).unwrap(),
            Some(&json),
        )
        .await
        .unwrap();

        let req = test::TestRequest::put()
            .uri("/nodes")
            .set_json(json)
            .insert_header(("x-address", "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"))
            .insert_header(("x-signature", signature))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
