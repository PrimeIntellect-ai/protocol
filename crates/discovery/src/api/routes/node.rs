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
    let existing_node = data.node_store.get_node(update_node.id.clone()).await;
    if let Ok(Some(existing_node)) = existing_node {
        // Node already exists - check if it's active in a pool
        if existing_node.is_active {
            if existing_node.node == update_node {
                log::info!("Node {} is already active in a pool", update_node.id);
                return HttpResponse::Ok()
                    .json(ApiResponse::new(true, "Node registered successfully"));
            }
            // Temp. adjustment: The gpu object has changed and includes a vec of indices now.
            // This now causes the discovery svc to reject nodes that have just updated their software.
            // This is a temporary fix to ensure the node is accepted even though the indices are different.
            let mut existing_clone = existing_node.node.clone();
            existing_clone.worker_p2p_id = update_node.worker_p2p_id.clone();
            existing_clone.worker_p2p_addresses = update_node.worker_p2p_addresses.clone();
            match &update_node.compute_specs {
                Some(compute_specs) => {
                    if let Some(ref mut existing_compute_specs) = existing_clone.compute_specs {
                        match &compute_specs.gpu {
                            Some(gpu_specs) => {
                                existing_compute_specs.gpu = Some(gpu_specs.clone());
                                existing_compute_specs.storage_gb = compute_specs.storage_gb;
                                existing_compute_specs.storage_path =
                                    compute_specs.storage_path.clone();
                            }
                            None => {
                                existing_compute_specs.gpu = None;
                            }
                        }
                    }
                }
                None => {
                    existing_clone.compute_specs = None;
                }
            }

            if existing_clone == update_node {
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

    let active_nodes_count = data
        .node_store
        .count_active_nodes_by_ip(update_node.ip_address.clone())
        .await;

    if let Ok(count) = active_nodes_count {
        let existing_node_by_ip = data
            .node_store
            .get_active_node_by_ip(update_node.ip_address.clone())
            .await;

        let is_existing_node = existing_node_by_ip
            .map(|result| {
                result
                    .map(|node| node.id == update_node.id)
                    .unwrap_or(false)
            })
            .unwrap_or(false);

        let effective_count = if is_existing_node { count - 1 } else { count };

        if effective_count >= data.max_nodes_per_ip {
            warn!(
                "Node {} registration would exceed IP limit. Current active nodes on IP {}: {}, max allowed: {}",
                update_node.id, update_node.ip_address, count, data.max_nodes_per_ip
            );
            return HttpResponse::BadRequest().json(ApiResponse::new(
                false,
                &format!(
                    "IP address {} already has {} active nodes (max allowed: {})",
                    update_node.ip_address, count, data.max_nodes_per_ip
                ),
            ));
        }
    }

    if let Some(contracts) = data.contracts.clone() {
        let provider_address = match node.provider_address.parse() {
            Ok(addr) => addr,
            Err(_) => {
                return HttpResponse::BadRequest()
                    .json(ApiResponse::new(false, "Invalid provider address format"));
            }
        };

        let node_id = match node.id.parse() {
            Ok(id) => id,
            Err(_) => {
                return HttpResponse::BadRequest()
                    .json(ApiResponse::new(false, "Invalid node ID format"));
            }
        };

        if contracts
            .compute_registry
            .get_node(provider_address, node_id)
            .await
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

    match node_store.register_node(node.into_inner()).await {
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
    use shared::security::request_signer::sign_request_with_nonce;
    use shared::web3::wallet::Wallet;
    use std::sync::Arc;
    use std::time::SystemTime;
    use tokio::sync::Mutex;
    use url::Url;

    #[actix_web::test]
    async fn test_register_node() {
        let node = Node {
            id: "0x32A8dFdA26948728e5351e61d62C190510CF1C88".to_string(),
            provider_address: "0x32A8dFdA26948728e5351e61d62C190510CF1C88".to_string(),
            ip_address: "127.0.0.1".to_string(),
            port: 8089,
            compute_pool_id: 0,
            ..Default::default()
        };

        let app_state = AppState {
            node_store: Arc::new(NodeStore::new(RedisStore::new_test())),
            contracts: None,
            last_chain_sync: Arc::new(Mutex::new(None::<SystemTime>)),
            max_nodes_per_ip: 1,
            chain_sync_enabled: true,
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
                    indices: Some(vec![0, 1, 2, 3]),
                }),
                cpu: Some(CpuSpecs {
                    cores: Some(16),
                    model: None,
                }),
                ram_mb: Some(64000),
                storage_gb: Some(500),
                ..Default::default()
            }),
            ..Default::default()
        };

        let node_clone_for_recall = node.clone();

        let app_state = AppState {
            node_store: Arc::new(NodeStore::new(RedisStore::new_test())),
            contracts: None,
            last_chain_sync: Arc::new(Mutex::new(None::<SystemTime>)),
            max_nodes_per_ip: 1,
            chain_sync_enabled: true,
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
        let signed_request = sign_request_with_nonce(
            "/nodes",
            &Wallet::new(private_key, Url::parse("http://localhost:8080").unwrap()).unwrap(),
            Some(&json),
        )
        .await
        .unwrap();

        let req = test::TestRequest::put()
            .uri("/nodes")
            .set_json(signed_request.data.as_ref().unwrap())
            .insert_header(("x-address", node.id.clone()))
            .insert_header(("x-signature", signed_request.signature))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body: ApiResponse<String> = test::read_body_json(resp).await;
        assert!(body.success);
        assert_eq!(body.data, "Node registered successfully");

        let nodes = app_state.node_store.get_nodes().await;
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
            location: None,
            latest_balance: None,
        };

        match app_state.node_store.update_node(validated).await {
            Ok(_) => (),
            Err(_) => {
                unreachable!("Error updating node");
            }
        }

        let nodes = app_state.node_store.get_nodes().await;
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
        let signed_request = sign_request_with_nonce(
            "/nodes",
            &Wallet::new(private_key, Url::parse("http://localhost:8080").unwrap()).unwrap(),
            Some(&json),
        )
        .await
        .unwrap();

        let req = test::TestRequest::put()
            .uri("/nodes")
            .set_json(signed_request.data.as_ref().unwrap())
            .insert_header(("x-address", node_clone_for_recall.id.clone()))
            .insert_header(("x-signature", signed_request.signature))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let nodes = app_state.node_store.get_nodes().await;
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
            ..Default::default()
        };

        let app_state = AppState {
            node_store: Arc::new(NodeStore::new(RedisStore::new_test())),
            contracts: None,
            last_chain_sync: Arc::new(Mutex::new(None::<SystemTime>)),
            max_nodes_per_ip: 1,
            chain_sync_enabled: true,
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
        let signed_request = sign_request_with_nonce(
            "/nodes",
            &Wallet::new(private_key, Url::parse("http://localhost:8080").unwrap()).unwrap(),
            Some(&json),
        )
        .await
        .unwrap();

        let req = test::TestRequest::put()
            .uri("/nodes")
            .set_json(signed_request.data.as_ref().unwrap())
            .insert_header(("x-address", node.id.clone()))
            .insert_header(("x-signature", signed_request.signature))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body: ApiResponse<String> = test::read_body_json(resp).await;
        assert!(body.success);
        assert_eq!(body.data, "Node registered successfully");

        let nodes = app_state.node_store.get_nodes().await;
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
            ..Default::default()
        };

        let app_state = AppState {
            node_store: Arc::new(NodeStore::new(RedisStore::new_test())),
            contracts: None,
            last_chain_sync: Arc::new(Mutex::new(None::<SystemTime>)),
            max_nodes_per_ip: 1,
            chain_sync_enabled: true,
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
        let signed_request = sign_request_with_nonce(
            "/nodes",
            &Wallet::new(private_key, Url::parse("http://localhost:8080").unwrap()).unwrap(),
            Some(&json),
        )
        .await
        .unwrap();

        let req = test::TestRequest::put()
            .uri("/nodes")
            .set_json(signed_request.data.as_ref().unwrap())
            .insert_header(("x-address", "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"))
            .insert_header(("x-signature", signed_request.signature))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_web::test]
    async fn test_register_node_already_active_in_pool() {
        let private_key = "0000000000000000000000000000000000000000000000000000000000000001";
        let mut node = Node {
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
                    indices: None,
                }),
                cpu: Some(CpuSpecs {
                    cores: Some(16),
                    model: None,
                }),
                ram_mb: Some(64000),
                storage_gb: Some(500),
                ..Default::default()
            }),
            ..Default::default()
        };

        let app_state = AppState {
            node_store: Arc::new(NodeStore::new(RedisStore::new_test())),
            contracts: None,
            last_chain_sync: Arc::new(Mutex::new(None::<SystemTime>)),
            max_nodes_per_ip: 1,
            chain_sync_enabled: true,
        };

        app_state
            .node_store
            .register_node(node.clone())
            .await
            .unwrap();

        node.compute_specs.as_mut().unwrap().storage_gb = Some(300);
        node.compute_specs
            .as_mut()
            .unwrap()
            .gpu
            .as_mut()
            .unwrap()
            .indices = Some(vec![0, 1, 2, 3]);

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
        let signed_request = sign_request_with_nonce(
            "/nodes",
            &Wallet::new(private_key, Url::parse("http://localhost:8080").unwrap()).unwrap(),
            Some(&json),
        )
        .await
        .unwrap();

        let req = test::TestRequest::put()
            .uri("/nodes")
            .set_json(signed_request.data.as_ref().unwrap())
            .insert_header(("x-address", node.id.clone()))
            .insert_header(("x-signature", signed_request.signature))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body: ApiResponse<String> = test::read_body_json(resp).await;
        assert!(body.success);
        assert_eq!(body.data, "Node registered successfully");

        let nodes = app_state.node_store.get_nodes().await;
        let nodes = match nodes {
            Ok(nodes) => nodes,
            Err(_) => {
                panic!("Error getting nodes");
            }
        };
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].id, node.id);
    }

    #[actix_web::test]
    async fn test_register_node_with_max_nodes_per_ip() {
        let private_key = "0000000000000000000000000000000000000000000000000000000000000001";
        let private_key_2 = "0000000000000000000000000000000000000000000000000000000000000002";
        let private_key_3 = "0000000000000000000000000000000000000000000000000000000000000003";

        let node1 = Node {
            id: "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf".to_string(),
            provider_address: "0x32A8dFdA26948728e5351e61d62C190510CF1C88".to_string(),
            ip_address: "127.0.0.1".to_string(),
            port: 8089,
            compute_pool_id: 0,
            ..Default::default()
        };

        let node2 = Node {
            id: "0x2546BcD3c84621e976D8185a91A922aE77ECEc30".to_string(),
            provider_address: "0x2546BcD3c84621e976D8185a91A922aE77ECEc30".to_string(),
            ip_address: "127.0.0.1".to_string(),
            port: 8090,
            compute_pool_id: 0,
            ..Default::default()
        };

        let node3 = Node {
            id: "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC".to_string(),
            provider_address: "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC".to_string(),
            ip_address: "127.0.0.1".to_string(),
            port: 8091,
            compute_pool_id: 0,
            ..Default::default()
        };

        let app_state = AppState {
            node_store: Arc::new(NodeStore::new(RedisStore::new_test())),
            contracts: None,
            last_chain_sync: Arc::new(Mutex::new(None::<SystemTime>)),
            max_nodes_per_ip: 2,
            chain_sync_enabled: true,
        };

        let app = test::init_service(
            App::new()
                .app_data(Data::new(app_state.clone()))
                .route("/nodes", put().to(register_node)),
        )
        .await;

        // Register first node - should succeed
        let json1 = serde_json::to_value(node1.clone()).unwrap();
        let signature1 = sign_request_with_nonce(
            "/nodes",
            &Wallet::new(private_key, Url::parse("http://localhost:8080").unwrap()).unwrap(),
            Some(&json1),
        )
        .await
        .unwrap();

        let req1 = test::TestRequest::put()
            .uri("/nodes")
            .set_json(signature1.data)
            .insert_header(("x-address", node1.id.clone()))
            .insert_header(("x-signature", signature1.signature))
            .to_request();

        let resp1 = test::call_service(&app, req1).await;
        assert_eq!(resp1.status(), StatusCode::OK);

        // Try to register same node again - should succeed (update)
        let json1_duplicate = serde_json::to_value(node1.clone()).unwrap();
        let signature1_duplicate = sign_request_with_nonce(
            "/nodes",
            &Wallet::new(private_key, Url::parse("http://localhost:8080").unwrap()).unwrap(),
            Some(&json1_duplicate),
        )
        .await
        .unwrap();

        let req1_duplicate = test::TestRequest::put()
            .uri("/nodes")
            .set_json(signature1_duplicate.data)
            .insert_header(("x-address", node1.id.clone()))
            .insert_header(("x-signature", signature1_duplicate.signature))
            .to_request();

        let resp1_duplicate = test::call_service(&app, req1_duplicate).await;
        assert_eq!(resp1_duplicate.status(), StatusCode::OK);

        // Register second node with different ID - should succeed
        let json2 = serde_json::to_value(node2.clone()).unwrap();
        let signature2 = sign_request_with_nonce(
            "/nodes",
            &Wallet::new(private_key_2, Url::parse("http://localhost:8080").unwrap()).unwrap(),
            Some(&json2),
        )
        .await
        .unwrap();

        let req2 = test::TestRequest::put()
            .uri("/nodes")
            .set_json(signature2.data)
            .insert_header(("x-address", node2.id.clone()))
            .insert_header(("x-signature", signature2.signature))
            .to_request();

        let resp2 = test::call_service(&app, req2).await;
        assert_eq!(resp2.status(), StatusCode::OK);

        // Make node 1 and two active
        let mut node1_active = DiscoveryNode::from(node1.clone());
        node1_active.is_active = true;
        app_state
            .node_store
            .update_node(node1_active)
            .await
            .unwrap();
        let mut node2_active = DiscoveryNode::from(node2.clone());
        node2_active.is_active = true;
        app_state
            .node_store
            .update_node(node2_active)
            .await
            .unwrap();

        // Register third node - should fail (exceeds max_nodes_per_ip)
        let json3 = serde_json::to_value(node3.clone()).unwrap();
        let signature3 = sign_request_with_nonce(
            "/nodes",
            &Wallet::new(private_key_3, Url::parse("http://localhost:8080").unwrap()).unwrap(),
            Some(&json3),
        )
        .await
        .unwrap();

        let req3 = test::TestRequest::put()
            .uri("/nodes")
            .set_json(signature3.data)
            .insert_header(("x-address", node3.id.clone()))
            .insert_header(("x-signature", signature3.signature))
            .to_request();

        let resp3 = test::call_service(&app, req3).await;
        assert_eq!(resp3.status(), StatusCode::BAD_REQUEST);

        // Verify only 2 nodes are registered
        let nodes = app_state.node_store.get_nodes().await.unwrap();
        assert_eq!(nodes.len(), 2);
    }
}
