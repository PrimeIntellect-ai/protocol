use crate::api::server::AppState;
use crate::types::Node;
use crate::types::ORCHESTRATOR_BASE_KEY;
use actix_web::{
    web::{self, get, Data},
    HttpResponse, Scope,
};
use redis::Commands;
use serde_json::json;

async fn get_nodes(app_state: Data<AppState>) -> HttpResponse {
    let mut con = app_state.store.client.get_connection().unwrap();
    let keys: Vec<String> = con.keys(format!("{}:*", ORCHESTRATOR_BASE_KEY)).unwrap();
    let mut nodes: Vec<Node> = Vec::new();

    for node in keys {
        let node_string: String = con.get(node).unwrap();
        let node: Node = Node::from_string(&node_string);
        nodes.push(node);
    }

    HttpResponse::Ok().json(json!({"success": true, "nodes": nodes}))
}

pub fn nodes_routes() -> Scope {
    web::scope("/nodes").route("", get().to(get_nodes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests::helper::create_test_app_state;
    use crate::types::Node;
    use crate::types::NodeStatus;
    use actix_web::http::StatusCode;
    use actix_web::test;
    use actix_web::App;
    use alloy::primitives::Address;
    use std::str::FromStr;

    #[actix_web::test]
    async fn test_get_nodes() {
        let app_state = create_test_app_state().await;
        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .route("/nodes", get().to(get_nodes)),
        )
        .await;

        let node = Node {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Discovered,
            task_id: None,
            task_state: None,
        };

        let mut con = app_state.store.client.get_connection().unwrap();
        let _: () = con.set(node.orchestrator_key(), node.to_string()).unwrap();

        let req = test::TestRequest::get().uri("/nodes").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "Expected status OK but got {:?}",
            resp.status()
        );
        let body = test::read_body(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json["success"], true,
            "Expected success to be true but got {:?}",
            json["success"]
        );
        let nodes_array = json["nodes"].as_array().unwrap();
        assert_eq!(
            nodes_array.len(),
            1,
            "Expected 1 node but got {}",
            nodes_array.len()
        );
        assert_eq!(
            nodes_array[0]["id"],
            node.address.to_string(),
            "Expected address to be {} but got {}",
            node.address,
            nodes_array[0]["address"]
        );
    }
}
