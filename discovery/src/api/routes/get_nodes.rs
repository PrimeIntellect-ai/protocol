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
    let response = ApiResponse::new(true, nodes);
    HttpResponse::Ok().json(response)
}

pub async fn get_nodes_for_pool(
    data: Data<AppState>,
    pool_id: web::Path<String>,
    req: actix_web::HttpRequest,
) -> HttpResponse {
    let nodes = data.node_store.get_nodes();
    let id_clone = pool_id.clone();
    let pool_contract_id: U256 = id_clone.parse::<U256>().unwrap();
    let pool_id: u32 = pool_id.parse().unwrap();

    println!("Pool id: {:?}", pool_id);
    println!("Pool contract id: {:?}", pool_contract_id);
    let debug_address = req.headers().get("x-address");
    println!("Address: {:?}", debug_address);

    match data.contracts.clone() {
        Some(contracts) => {
            let pool_info = contracts
                .compute_pool
                .get_pool_info(pool_contract_id)
                .await
                .unwrap();
            let owner = pool_info.creator;
            let manager = pool_info.compute_manager_key;
            println!("Owner: {:?}", owner);
            println!("Manager: {:?}", manager);
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

            if address_str != owner.to_string() {
                return HttpResponse::BadRequest().json(ApiResponse::new(
                    false,
                    "Invalid x-address header - not owner or manager",
                ));
            }
        }
        None => {
            return HttpResponse::BadRequest().json(ApiResponse::new(false, "No contracts found"))
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

pub async fn get_node_by_subkey(node_id: web::Path<String>, data: Data<AppState>) -> HttpResponse {
    let node = data.node_store.get_node_by_id(node_id.to_string());

    match node {
        Some(node) => HttpResponse::Ok().json(ApiResponse::new(true, node)),
        None => HttpResponse::NotFound().json(ApiResponse::new(false, "Node not found")),
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

    #[actix_web::test]
    async fn test_get_nodes() {
        let app_state = AppState {
            node_store: Arc::new(NodeStore::new(RedisStore::new_test())),
            contracts: None,
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
        app_state.node_store.register_node(sample_node);

        let req = test::TestRequest::get().uri("/nodes").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
        let body = test::read_body(resp).await;
        let api_response: ApiResponse<Vec<DiscoveryNode>> = serde_json::from_slice(&body).unwrap();
        assert!(api_response.success);
        assert_eq!(api_response.data.len(), 1);
    }
}
