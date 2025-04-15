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
    let nodes = data.node_store.get_nodes();
    match nodes {
        Ok(nodes) => {
            let response = ApiResponse::new(true, nodes);
            HttpResponse::Ok().json(response)
        }
        Err(_) => HttpResponse::InternalServerError()
            .json(ApiResponse::new(false, "Internal server error")),
    }
}

pub async fn get_nodes_for_pool(
    data: Data<AppState>,
    pool_id: web::Path<String>,
    req: actix_web::HttpRequest,
) -> HttpResponse {
    let nodes = data.node_store.get_nodes();
    match nodes {
        Ok(nodes) => {
            let id_clone = pool_id.clone();
            let pool_contract_id: U256 = id_clone.parse::<U256>().unwrap();
            let pool_id: u32 = pool_id.parse().unwrap();

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

            let nodes_for_pool: Vec<DiscoveryNode> = nodes
                .iter()
                .filter(|node| node.compute_pool_id == pool_id)
                .cloned()
                .collect();

            let response = ApiResponse::new(true, nodes_for_pool);
            HttpResponse::Ok().json(response)
        }
        Err(_) => HttpResponse::InternalServerError()
            .json(ApiResponse::new(false, "Internal server error")),
    }
}

pub async fn get_node_by_subkey(node_id: web::Path<String>, data: Data<AppState>) -> HttpResponse {
    let node = data.node_store.get_node_by_id(&node_id.to_string());

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
    use tokio::sync::Mutex;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;
    use std::time::SystemTime;

    #[actix_web::test]
    async fn test_get_nodes() {
        let app_state = AppState {
            node_store: Arc::new(NodeStore::new(RedisStore::new_test())),
            contracts: None,
            last_chain_sync: Arc::new(Mutex::new(None::<SystemTime>)),
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
            compute_specs: None,
        };
        match app_state.node_store.register_node(sample_node) {
            Ok(_) => (),
            Err(_) => {
                panic!("Error registering node");
            }
        }

        let req = test::TestRequest::get().uri("/nodes").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        let body = test::read_body(resp).await;
        let api_response: ApiResponse<Vec<DiscoveryNode>> = serde_json::from_slice(&body).unwrap();
        assert!(api_response.success);
        assert_eq!(api_response.data.len(), 1);
    }

    #[actix_web::test]
    async fn test_nodes_sorted_by_newest_first() {
        let app_state = AppState {
            node_store: Arc::new(NodeStore::new(RedisStore::new_test())),
            contracts: None,
            last_chain_sync: Arc::new(Mutex::new(None::<SystemTime>)),
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
            compute_specs: None,
        };
        match app_state.node_store.register_node(older_node) {
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
            compute_specs: None,
        };
        match app_state.node_store.register_node(newer_node) {
            Ok(_) => (),
            Err(_) => {
                panic!("Error registering node");
            }
        }

        let req = test::TestRequest::get().uri("/nodes").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        let body = test::read_body(resp).await;
        let api_response: ApiResponse<Vec<DiscoveryNode>> = serde_json::from_slice(&body).unwrap();

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
}
