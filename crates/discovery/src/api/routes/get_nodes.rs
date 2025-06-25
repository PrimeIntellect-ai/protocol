use crate::api::server::AppState;
use actix_web::{
    web::Data,
    web::{self},
    HttpResponse,
};
use alloy::primitives::U256;
use shared::models::api::ApiResponse;
use shared::models::node::DiscoveryNode;

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

                    // Normalize the address strings for comparison
                    let owner_str = owner.to_string().to_lowercase();
                    let manager_str = manager.to_string().to_lowercase();
                    let address_str_normalized = address_str.to_lowercase();

                    if address_str_normalized != owner_str && address_str_normalized != manager_str
                    {
                        return HttpResponse::BadRequest().json(ApiResponse::new(
                            false,
                            "Invalid x-address header - not owner or manager",
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

    const TEST_DELAY_MS: u64 = 100;
    use std::time::SystemTime;
    use tokio::sync::Mutex;

    #[actix_web::test]
    async fn test_get_nodes() {
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
            max_nodes_per_ip: 1,
            chain_sync_enabled: true,
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
        thread::sleep(Duration::from_millis(TEST_DELAY_MS));

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
