use crate::api::server::AppState;
use actix_web::{
    web::{self, get, post, Data},
    HttpResponse, Scope,
};
use alloy::primitives::Address;
use serde_json::json;
use shared::security::request_signer::sign_request;
use std::str::FromStr;

async fn get_nodes(app_state: Data<AppState>) -> HttpResponse {
    let nodes = app_state.store_context.node_store.get_nodes();
    HttpResponse::Ok().json(json!({"success": true, "nodes": nodes}))
}

async fn restart_node_task(node_id: web::Path<String>, app_state: Data<AppState>) -> HttpResponse {
    println!("restart_node_task: {}", node_id);
    let node_address = Address::from_str(&node_id).unwrap();
    let node = app_state.store_context.node_store.get_node(&node_address);
    match node {
        Some(node) => {
            let node_ip = node.ip_address;
            let node_port = node.port;

            let node_url = format!("http://{}:{}", node_ip, node_port);
            let restart_path = "/task/restart".to_string();
            let restart_url = format!("{}{}", node_url, restart_path);
            let payload = json!({
                "timestamp": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
            });

            let message_signature = sign_request(&restart_path, &app_state.wallet, Some(&payload))
                .await
                .unwrap();

            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                "x-address",
                app_state
                    .wallet
                    .wallet
                    .default_signer()
                    .address()
                    .to_string()
                    .parse()
                    .unwrap(),
            );
            headers.insert("x-signature", message_signature.parse().unwrap());

            match reqwest::Client::new()
                .post(restart_url)
                .headers(headers)
                .body(payload.to_string())
                .send()
                .await
            {
                Ok(response) => {
                    if response.status().is_success() {
                        let result = response.json::<serde_json::Value>().await.unwrap();
                        HttpResponse::Ok().json(result)
                    } else {
                        HttpResponse::InternalServerError().json(json!({
                            "success": false,
                            "error": format!("Failed to restart task: {}", response.status())
                        }))
                    }
                }
                Err(e) => HttpResponse::InternalServerError().json(json!({
                    "success": false,
                    "error": format!("Failed to restart task: {}", e)
                })),
            }
        }
        None => HttpResponse::NotFound().json(json!({
            "success": false,
            "error": format!("Node not found: {}", node_id)
        })),
    }
}

async fn get_node_logs(node_id: web::Path<String>, app_state: Data<AppState>) -> HttpResponse {
    let node_address = Address::from_str(&node_id).unwrap();
    let node = app_state.store_context.node_store.get_node(&node_address);
    match node {
        Some(node) => {
            let node_ip = node.ip_address;
            let node_port = node.port;

            let node_url = format!("http://{}:{}", node_ip, node_port);
            let logs_path = "/task/logs".to_string();
            let logs_url = format!(
                "{}{}?timestamp={}",
                node_url,
                logs_path,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
            );

            let message_signature = sign_request(&logs_path, &app_state.wallet, None)
                .await
                .unwrap();

            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                "x-address",
                app_state
                    .wallet
                    .wallet
                    .default_signer()
                    .address()
                    .to_string()
                    .parse()
                    .unwrap(),
            );
            headers.insert("x-signature", message_signature.parse().unwrap());

            match reqwest::Client::new()
                .get(logs_url)
                .headers(headers)
                .send()
                .await
            {
                Ok(response) => {
                    if response.status().is_success() {
                        let logs = response.json::<serde_json::Value>().await.unwrap();
                        HttpResponse::Ok().json(logs)
                    } else {
                        HttpResponse::InternalServerError().json(json!({
                            "success": false,
                            "error": format!("Failed to get logs: {}", response.status())
                        }))
                    }
                }
                Err(e) => HttpResponse::InternalServerError().json(json!({
                    "success": false,
                    "error": format!("Failed to get logs: {}", e)
                })),
            }
        }
        None => HttpResponse::Ok().json(json!({"success": false, "logs": "Node not found"})),
    }
}

async fn get_node_metrics(node_id: web::Path<String>, app_state: Data<AppState>) -> HttpResponse {
    println!("get_node_metrics: {}", node_id);
    let node_address = Address::from_str(&node_id).unwrap();
    let metrics = app_state
        .store_context
        .metrics_store
        .get_metrics_for_node(node_address);
    HttpResponse::Ok().json(json!({"success": true, "metrics": metrics}))
}

pub fn nodes_routes() -> Scope {
    web::scope("/nodes")
        .route("", get().to(get_nodes))
        .route("/{node_id}/restart", post().to(restart_node_task))
        .route("/{node_id}/logs", get().to(get_node_logs))
        .route("/{node_id}/metrics", get().to(get_node_metrics))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests::helper::create_test_app_state;
    use crate::models::node::NodeStatus;
    use crate::models::node::OrchestratorNode;
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
        let node = OrchestratorNode {
            address: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Discovered,
            task_id: None,
            task_state: None,
            version: None,
            last_status_change: None,
        };
        app_state.store_context.node_store.add_node(node.clone());

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
            nodes_array[0]["address"],
            node.address.to_string(),
            "Expected address to be {} but got {}",
            node.address,
            nodes_array[0]["address"]
        );
    }

    #[actix_web::test]
    async fn test_get_metrics_for_node_not_exist() {
        let app_state = create_test_app_state().await;
        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .route("/nodes/{node_id}/metrics", get().to(get_node_metrics)),
        )
        .await;

        let node_id = "0x0000000000000000000000000000000000000000";
        let req = test::TestRequest::get()
            .uri(&format!("/nodes/{}/metrics", node_id))
            .to_request();
        let resp = test::call_service(&app, req).await;

        let body = test::read_body(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        println!("json {:?}", json);
        assert_eq!(
            json["success"], true,
            "Expected success to be true but got {:?}",
            json["success"]
        );
        assert_eq!(
            json["metrics"],
            json!({}),
            "Expected empty metrics object but got {:?}",
            json["metrics"]
        );
    }
}
