use crate::api::server::AppState;
use actix_web::{
    web::{self, put, Data},
    HttpResponse, Scope,
};
use alloy::primitives::U256;
use log::warn;
use shared::models::api::ApiResponse;
use shared::models::node::{ComputeRequirements, Node};
use std::str::FromStr;

pub async fn register_node(
    node: web::Json<Node>,
    data: Data<AppState>,
    req: actix_web::HttpRequest,
) -> HttpResponse {
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

    let update_node = node.clone();
    let existing_node = data.node_store.get_node(update_node.id.clone());
    if let Ok(Some(existing_node)) = existing_node {
        // Node already exists - check if it's active in a pool
        if existing_node.is_active {
            if existing_node.node == update_node {
                log::info!("Node {} is already active in a pool", update_node.id);
                return HttpResponse::Ok()
                    .json(ApiResponse::new(true, "Node registered successfully"));
            }

            warn!(
                "Node {} tried to change discovery but is already active in a pool",
                update_node.id
            );
            // Node is currently active in pool - cannot be updated
            // Did the user actually change node information?
            return HttpResponse::BadRequest().json(ApiResponse::new(
                false,
                "Node is currently active in pool - cannot be updated",
            ));
        }
    }

    // Check if any other node is on this same IP with different address
    let existing_node_by_ip = data
        .node_store
        .get_active_node_by_ip(update_node.ip_address.clone());
    if let Ok(Some(existing_node)) = existing_node_by_ip {
        if existing_node.id != update_node.id {
            warn!(
                "Node {} tried to change discovery but another active node is already registered to this IP address",
                update_node.id
            );
            return HttpResponse::BadRequest().json(ApiResponse::new(
                false,
                "Another active Node is already registered to this IP address",
            ));
        }
    }

    if let Some(contracts) = data.contracts.clone() {
        if (contracts
            .compute_registry
            .get_node(
                node.provider_address.parse().unwrap(),
                node.id.parse().unwrap(),
            )
            .await)
            .is_err()
        {
            return HttpResponse::BadRequest().json(ApiResponse::new(
                false,
                "Node not found in compute registry",
            ));
        }

        // Check if node meets the pool's compute requirements
        match contracts
            .compute_pool
            .get_pool_info(U256::from(node.compute_pool_id))
            .await
        {
            Ok(pool_info) => {
                if let Ok(required_specs) = ComputeRequirements::from_str(&pool_info.pool_data_uri)
                {
                    if let Some(ref compute_specs) = node.compute_specs {
                        if !compute_specs.meets(&required_specs) {
                            log::info!(
                                "Node {} does not meet compute requirements for pool {}",
                                node.id,
                                node.compute_pool_id
                            );
                            return HttpResponse::BadRequest().json(ApiResponse::new(
                                false,
                                "Node does not meet the compute requirements for this pool",
                            ));
                        }
                    } else {
                        log::info!("Node specs not provided for node {}", node.id);
                        return HttpResponse::BadRequest().json(ApiResponse::new(
                            false,
                            "Cannot verify compute requirements: node specs not provided",
                        ));
                    }
                } else {
                    log::info!(
                        "Could not parse compute requirements from pool data URI: {}",
                        &pool_info.pool_data_uri
                    );
                }
            }
            Err(e) => {
                log::info!(
                    "Failed to get pool information for pool ID {}: {:?}",
                    node.compute_pool_id,
                    e
                );
                return HttpResponse::BadRequest()
                    .json(ApiResponse::new(false, "Failed to get pool information"));
            }
        }
    }

    let node_store = data.node_store.clone();

    match node_store.register_node(node.clone()) {
        Ok(_) => HttpResponse::Ok().json(ApiResponse::new(true, "Node registered successfully")),
        Err(_) => HttpResponse::InternalServerError()
            .json(ApiResponse::new(false, "Internal server error")),
    }
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
    use shared::models::node::{ComputeSpecs, CpuSpecs, DiscoveryNode, GpuSpecs};
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
            compute_specs: Some(ComputeSpecs {
                gpu: Some(GpuSpecs {
                    count: Some(4),
                    model: Some("A100".to_string()),
                    memory_mb: Some(40000),
                }),
                cpu: Some(CpuSpecs {
                    cores: Some(16),
                    model: None,
                }),
                ram_mb: Some(64000),
                storage_gb: Some(500),
                storage_path: None,
            }),
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
        match nodes {
            Ok(nodes) => {
                assert_eq!(nodes.len(), 1);
                assert_eq!(nodes[0].id, node.id);
            }
            Err(_) => {
                unreachable!("Error getting nodes");
            }
        }
        let validated = DiscoveryNode {
            node,
            is_validated: true,
            is_active: true,
            is_provider_whitelisted: false,
            is_blacklisted: false,
            last_updated: None,
            created_at: None,
        };

        match app_state.node_store.update_node(validated) {
            Ok(_) => (),
            Err(_) => {
                unreachable!("Error updating node");
            }
        }

        let nodes = app_state.node_store.get_nodes();
        match nodes {
            Ok(nodes) => {
                assert_eq!(nodes.len(), 1);
                assert_eq!(nodes[0].id, node_clone_for_recall.id);
                assert!(nodes[0].is_validated);
                assert!(nodes[0].is_active);
            }
            Err(_) => {
                unreachable!("Error getting nodes");
            }
        }

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
        match nodes {
            Ok(nodes) => {
                assert_eq!(nodes.len(), 1);
                assert_eq!(nodes[0].id, node_clone_for_recall.id);
                assert!(nodes[0].is_validated);
                assert!(nodes[0].is_active);
            }
            Err(_) => {
                unreachable!("Error getting nodes");
            }
        }
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
        let nodes = match nodes {
            Ok(nodes) => nodes,
            Err(_) => {
                panic!("Error getting nodes");
            }
        };
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].id, node.id);
        assert_eq!(nodes[0].last_updated, None);
        assert_ne!(nodes[0].created_at, None);
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
