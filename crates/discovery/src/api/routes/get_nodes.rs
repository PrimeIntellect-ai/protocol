use crate::api::server::AppState;
use actix_web::{
    web::Data,
    web::{self},
    HttpResponse,
};
use alloy::primitives::{Address, U256};
use alloy::signers::Signature;
use shared::models::api::ApiResponse;
use shared::models::node::DiscoveryNode;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

pub async fn get_nodes(data: Data<AppState>) -> HttpResponse {
    let nodes = data.node_store.get_nodes().await;
    match nodes {
        Ok(nodes) => {
            let response = ApiResponse::new(true, nodes);
            HttpResponse::Ok().json(response)
        }
        Err(_) => HttpResponse::InternalServerError()
            .json(ApiResponse::new(false, "Internal server error")),
    }
}

fn filter_nodes_for_pool(nodes: Vec<DiscoveryNode>, pool_id: u32) -> Vec<DiscoveryNode> {
    let nodes_for_pool: Vec<DiscoveryNode> = nodes
        .iter()
        .filter(|node| node.compute_pool_id == pool_id)
        .cloned()
        .collect();

    // Filter out nodes with IPs that are currently active in another pool
    let filtered: Vec<DiscoveryNode> = nodes_for_pool
        .iter()
        .filter(|node| {
            // Check if there's any other node with the same IP address in a different pool that is active
            !nodes.iter().any(|other| {
                other.ip_address == node.ip_address
                    && other.compute_pool_id != node.compute_pool_id
                    && other.is_active
            })
        })
        .cloned()
        .collect();
    filtered
}

pub async fn get_nodes_for_pool(
    data: Data<AppState>,
    pool_id: web::Path<String>,
    req: actix_web::HttpRequest,
) -> HttpResponse {
    // Extract and validate authentication headers first
    let address_str = match req.headers().get("x-address") {
        Some(address) => match address.to_str() {
            Ok(addr) => addr.to_string(),
            Err(_) => {
                return HttpResponse::BadRequest().json(ApiResponse::new(
                    false,
                    "Invalid x-address header - parsing issue",
                ))
            }
        },
        None => {
            return HttpResponse::BadRequest()
                .json(ApiResponse::new(false, "Missing x-address header"))
        }
    };

    let signature_str = match req.headers().get("x-signature") {
        Some(signature) => match signature.to_str() {
            Ok(sig) => sig.to_string(),
            Err(_) => {
                return HttpResponse::BadRequest().json(ApiResponse::new(
                    false,
                    "Invalid x-signature header - parsing issue",
                ))
            }
        },
        None => {
            return HttpResponse::BadRequest()
                .json(ApiResponse::new(false, "Missing x-signature header"))
        }
    };

    // Validate signature format and recover address
    let signature = signature_str.trim_start_matches("0x");
    let parsed_signature = match Signature::from_str(signature) {
        Ok(sig) => sig,
        Err(_) => {
            return HttpResponse::BadRequest()
                .json(ApiResponse::new(false, "Invalid signature format"))
        }
    };

    let expected_address = match Address::from_str(&address_str) {
        Ok(addr) => addr,
        Err(_) => {
            return HttpResponse::BadRequest()
                .json(ApiResponse::new(false, "Invalid address format"))
        }
    };

    // Create message for signature verification (path only for GET requests)
    let path = req.path();
    let msg = path.to_string();

    let recovered_address = match parsed_signature.recover_address_from_msg(&msg) {
        Ok(addr) => addr,
        Err(_) => {
            return HttpResponse::BadRequest().json(ApiResponse::new(
                false,
                "Failed to recover address from signature",
            ))
        }
    };

    if recovered_address != expected_address {
        return HttpResponse::BadRequest()
            .json(ApiResponse::new(false, "Signature verification failed"));
    }

    // Validate timestamp if present in query parameters to prevent replay attacks
    if let Some(query) = req.uri().query() {
        if let Some(timestamp_param) = query
            .split('&')
            .find(|param| param.starts_with("timestamp="))
            .and_then(|param| param.split('=').nth(1))
        {
            if let Ok(timestamp) = timestamp_param.parse::<u64>() {
                let current_time = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                if current_time - timestamp > 10 {
                    return HttpResponse::BadRequest().json(ApiResponse::new(
                        false,
                        "Request expired - timestamp too old",
                    ));
                }
            }
        }
    }

    let nodes = data.node_store.get_nodes().await;
    match nodes {
        Ok(nodes) => {
            let id_clone = pool_id.clone();
            let pool_contract_id: U256 = match id_clone.parse::<U256>() {
                Ok(id) => id,
                Err(_) => {
                    return HttpResponse::BadRequest()
                        .json(ApiResponse::new(false, "Invalid pool ID format"));
                }
            };
            let pool_id: u32 = match pool_id.parse() {
                Ok(id) => id,
                Err(_) => {
                    return HttpResponse::BadRequest()
                        .json(ApiResponse::new(false, "Invalid pool ID format"));
                }
            };

            match data.contracts.clone() {
                Some(contracts) => {
                    // Only make expensive RPC call after signature validation
                    let pool_info =
                        match contracts.compute_pool.get_pool_info(pool_contract_id).await {
                            Ok(info) => info,
                            Err(_) => {
                                return HttpResponse::NotFound()
                                    .json(ApiResponse::new(false, "Pool not found"));
                            }
                        };
                    let owner = pool_info.creator;
                    let manager = pool_info.compute_manager_key;

                    // Check if authenticated address is authorized for this pool
                    if expected_address != owner && expected_address != manager {
                        return HttpResponse::Forbidden().json(ApiResponse::new(
                            false,
                            "Access denied - caller is not pool owner or manager",
                        ));
                    }
                }
                None => {
                    return HttpResponse::BadRequest()
                        .json(ApiResponse::new(false, "No contracts found"))
                }
            }

            let nodes_for_pool: Vec<DiscoveryNode> = filter_nodes_for_pool(nodes, pool_id);
            let response = ApiResponse::new(true, nodes_for_pool);
            HttpResponse::Ok().json(response)
        }
        Err(_) => HttpResponse::InternalServerError()
            .json(ApiResponse::new(false, "Internal server error")),
    }
}

pub async fn get_node_by_subkey(node_id: web::Path<String>, data: Data<AppState>) -> HttpResponse {
    let node = data.node_store.get_node_by_id(&node_id.to_string()).await;

    match node {
        Ok(Some(node)) => HttpResponse::Ok().json(ApiResponse::new(true, node)),
        Ok(None) => HttpResponse::NotFound().json(ApiResponse::new(false, "Node not found")),
        Err(_) => HttpResponse::InternalServerError()
            .json(ApiResponse::new(false, "Internal server error")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::node_store::NodeStore;
    use crate::store::redis::RedisStore;
    use actix_web::test;
    use actix_web::web::get;
    use actix_web::App;
    use shared::models::node::DiscoveryNode;
    use shared::models::node::Node;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;
    use std::time::SystemTime;
    use tokio::sync::Mutex;

    #[actix_web::test]
    async fn test_get_nodes() {
        let app_state = AppState {
            node_store: Arc::new(NodeStore::new(RedisStore::new_test())),
            contracts: None,
            last_chain_sync: Arc::new(Mutex::new(None::<SystemTime>)),
            only_one_node_per_ip: true,
        };
        let app = test::init_service(
            App::new()
                .app_data(Data::new(app_state.clone()))
                .route("/nodes", get().to(get_nodes)),
        )
        .await;

        let sample_node = Node {
            id: "0x32A8dFdA26948728e5351e61d62C190510CF1C88".to_string(),
            provider_address: "0x32A8dFdA26948728e5351e61d62C190510CF1C88".to_string(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            compute_pool_id: 0,
            ..Default::default()
        };
        match app_state.node_store.register_node(sample_node).await {
            Ok(_) => (),
            Err(_) => {
                panic!("Error registering node");
            }
        }

        let req = test::TestRequest::get().uri("/nodes").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        let body = test::read_body(resp).await;
        let api_response: ApiResponse<Vec<DiscoveryNode>> = match serde_json::from_slice(&body) {
            Ok(response) => response,
            Err(_) => panic!("Failed to deserialize response"),
        };
        assert!(api_response.success);
        assert_eq!(api_response.data.len(), 1);
    }

    #[actix_web::test]
    async fn test_nodes_sorted_by_newest_first() {
        let app_state = AppState {
            node_store: Arc::new(NodeStore::new(RedisStore::new_test())),
            contracts: None,
            last_chain_sync: Arc::new(Mutex::new(None::<SystemTime>)),
            only_one_node_per_ip: true,
        };
        let app = test::init_service(
            App::new()
                .app_data(Data::new(app_state.clone()))
                .route("/nodes", get().to(get_nodes)),
        )
        .await;

        // Register older node first
        let older_node = Node {
            id: "0x32A8dFdA26948728e5351e61d62C190510CF1C88".to_string(),
            provider_address: "0x32A8dFdA26948728e5351e61d62C190510CF1C88".to_string(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            compute_pool_id: 0,
            ..Default::default()
        };
        match app_state.node_store.register_node(older_node).await {
            Ok(_) => (),
            Err(_) => {
                panic!("Error registering node");
            }
        }

        // Wait a moment to ensure timestamps are different
        thread::sleep(Duration::from_millis(100));

        // Register newer node
        let newer_node = Node {
            id: "0x45B8dFdA26948728e5351e61d62C190510CF1C99".to_string(),
            provider_address: "0x45B8dFdA26948728e5351e61d62C190510CF1C99".to_string(),
            ip_address: "127.0.0.2".to_string(),
            port: 8081,
            compute_pool_id: 0,
            ..Default::default()
        };
        match app_state.node_store.register_node(newer_node).await {
            Ok(_) => (),
            Err(_) => {
                panic!("Error registering node");
            }
        }

        let req = test::TestRequest::get().uri("/nodes").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        let body = test::read_body(resp).await;
        let api_response: ApiResponse<Vec<DiscoveryNode>> = match serde_json::from_slice(&body) {
            Ok(response) => response,
            Err(_) => panic!("Failed to deserialize response"),
        };

        assert!(api_response.success);
        assert_eq!(api_response.data.len(), 2);

        // Verify the newer node is first in the list
        assert_eq!(
            api_response.data[0].id,
            "0x45B8dFdA26948728e5351e61d62C190510CF1C99"
        );
        assert_eq!(
            api_response.data[1].id,
            "0x32A8dFdA26948728e5351e61d62C190510CF1C88"
        );
    }

    #[actix_web::test]
    async fn test_filter_nodes_for_pool() {
        // Create test nodes for different pools
        let mut nodes = vec![
            DiscoveryNode {
                node: Node {
                    id: "0x1111".to_string(),
                    provider_address: "0x1111".to_string(),
                    ip_address: "192.168.1.1".to_string(),
                    port: 8080,
                    compute_pool_id: 1,
                    ..Default::default()
                },
                is_validated: true,
                is_provider_whitelisted: true,
                is_active: true,
                is_blacklisted: false,
                ..Default::default()
            },
            DiscoveryNode {
                node: Node {
                    id: "0x2222".to_string(),
                    provider_address: "0x2222".to_string(),
                    ip_address: "192.168.1.2".to_string(),
                    port: 8080,
                    compute_pool_id: 1,
                    ..Default::default()
                },
                is_validated: true,
                is_provider_whitelisted: true,
                is_active: false,
                is_blacklisted: false,
                ..Default::default()
            },
        ];

        // Pool 2 nodes
        nodes.push(DiscoveryNode {
            node: Node {
                id: "0x3333".to_string(),
                provider_address: "0x3333".to_string(),
                ip_address: "192.168.1.3".to_string(),
                port: 8080,
                compute_pool_id: 2,
                ..Default::default()
            },
            is_validated: true,
            is_provider_whitelisted: true,
            is_active: true,
            is_blacklisted: false,
            ..Default::default()
        });

        // Node with same IP in different pools (active in pool 3)
        nodes.push(DiscoveryNode {
            node: Node {
                id: "0x4444".to_string(),
                provider_address: "0x4444".to_string(),
                ip_address: "192.168.1.4".to_string(),
                port: 8080,
                compute_pool_id: 3,
                ..Default::default()
            },
            is_validated: true,
            is_provider_whitelisted: true,
            is_active: true,
            is_blacklisted: false,
            ..Default::default()
        });

        // This node should be filtered out because it shares IP with an active node in pool 3
        nodes.push(DiscoveryNode {
            node: Node {
                id: "0x5555".to_string(),
                provider_address: "0x5555".to_string(),
                ip_address: "192.168.1.4".to_string(),
                port: 8081,
                compute_pool_id: 1,
                ..Default::default()
            },
            is_validated: true,
            is_provider_whitelisted: true,
            is_active: false,
            is_blacklisted: false,
            ..Default::default()
        });

        // Test filtering for pool 1
        let filtered_nodes = filter_nodes_for_pool(nodes.clone(), 1);

        // Should have 2 nodes from pool 1, but one is filtered out due to IP conflict
        assert_eq!(filtered_nodes.len(), 2);
        assert!(filtered_nodes.iter().any(|n| n.id == "0x1111"));
        assert!(filtered_nodes.iter().any(|n| n.id == "0x2222"));
        assert!(!filtered_nodes.iter().any(|n| n.id == "0x5555"));

        // Test filtering for pool 2
        let filtered_nodes = filter_nodes_for_pool(nodes.clone(), 2);
        assert_eq!(filtered_nodes.len(), 1);
        assert!(filtered_nodes.iter().any(|n| n.id == "0x3333"));

        // Test filtering for pool 3
        let filtered_nodes = filter_nodes_for_pool(nodes.clone(), 3);
        assert_eq!(filtered_nodes.len(), 1);
        assert!(filtered_nodes.iter().any(|n| n.id == "0x4444"));

        // Test filtering for non-existent pool
        let filtered_nodes = filter_nodes_for_pool(nodes.clone(), 99);
        assert_eq!(filtered_nodes.len(), 0);
    }

    #[actix_web::test]
    async fn test_filter_nodes_for_pool_with_inactive_nodes() {
        let nodes = vec![
            // Inactive node in pool 1
            DiscoveryNode {
                node: Node {
                    id: "0x1111".to_string(),
                    provider_address: "0x1111".to_string(),
                    ip_address: "192.168.1.1".to_string(),
                    port: 8080,
                    compute_pool_id: 1,
                    ..Default::default()
                },
                is_validated: true,
                is_provider_whitelisted: true,
                is_active: false,
                is_blacklisted: false,
                ..Default::default()
            },
            // Inactive node in pool 2 with same IP
            DiscoveryNode {
                node: Node {
                    id: "0x2222".to_string(),
                    provider_address: "0x2222".to_string(),
                    ip_address: "192.168.1.1".to_string(),
                    port: 8080,
                    compute_pool_id: 2,
                    ..Default::default()
                },
                is_validated: true,
                is_provider_whitelisted: true,
                is_active: false,
                is_blacklisted: false,
                ..Default::default()
            },
        ];

        // This should be included in pool 2 results since the conflicting node in pool 1
        // doesn't affect it (the filter only excludes nodes when there's an active node
        // with the same IP in a different pool)
        let filtered_nodes = filter_nodes_for_pool(nodes.clone(), 2);
        assert_eq!(filtered_nodes.len(), 1);
        assert!(filtered_nodes.iter().any(|n| n.id == "0x2222"));

        // The pool 1 node should be included in pool 1 results
        let filtered_nodes = filter_nodes_for_pool(nodes.clone(), 1);
        assert_eq!(filtered_nodes.len(), 1);
        assert!(filtered_nodes.iter().any(|n| n.id == "0x1111"));
    }
}
