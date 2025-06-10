use crate::api::server::AppState;
use actix_web::{
    web::{self, get, Data},
    HttpResponse, Scope,
};
use alloy::primitives::Address;
use futures::future::join_all;
use log::error;
use serde_json::json;
use shared::security::request_signer::sign_request;
use std::str::FromStr;
use std::time::Duration;

const NODE_REQUEST_TIMEOUT: u64 = 30;

async fn get_groups(app_state: Data<AppState>) -> HttpResponse {
    if let Some(node_groups_plugin) = &app_state.node_groups_plugin {
        match node_groups_plugin.get_all_groups().await {
            Ok(groups) => {
                let groups_with_details: Vec<_> = groups
                    .into_iter()
                    .map(|group| {
                        json!({
                            "id": group.id,
                            "nodes": group.nodes,
                            "node_count": group.nodes.len(),
                            "created_at": group.created_at,
                            "configuration_name": group.configuration_name
                        })
                    })
                    .collect();

                HttpResponse::Ok().json(json!({
                    "success": true,
                    "groups": groups_with_details,
                    "total_count": groups_with_details.len()
                }))
            }
            Err(e) => HttpResponse::InternalServerError().json(json!({
                "success": false,
                "error": format!("Failed to get groups: {}", e)
            })),
        }
    } else {
        HttpResponse::ServiceUnavailable().json(json!({
            "success": false,
            "error": "Node groups plugin is not enabled"
        }))
    }
}

async fn get_configurations(app_state: Data<AppState>) -> HttpResponse {
    if let Some(node_groups_plugin) = &app_state.node_groups_plugin {
        let all_configs = node_groups_plugin.get_all_configuration_templates();
        let available_configs = node_groups_plugin.get_available_configurations().await;

        let available_names: std::collections::HashSet<String> =
            available_configs.iter().map(|c| c.name.clone()).collect();

        let configs_with_status: Vec<_> = all_configs
            .into_iter()
            .map(|config| {
                json!({
                    "name": config.name,
                    "min_group_size": config.min_group_size,
                    "max_group_size": config.max_group_size,
                    "compute_requirements": config.compute_requirements,
                    "enabled": available_names.contains(&config.name)
                })
            })
            .collect();

        HttpResponse::Ok().json(json!({
            "success": true,
            "configurations": configs_with_status,
            "total_count": configs_with_status.len()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(json!({
            "success": false,
            "error": "Node groups plugin is not enabled"
        }))
    }
}

async fn get_group_logs(group_id: web::Path<String>, app_state: Data<AppState>) -> HttpResponse {
    if let Some(node_groups_plugin) = &app_state.node_groups_plugin {
        match node_groups_plugin.get_group_by_id(&group_id).await {
            Ok(Some(group)) => {
                // Collect all node addresses
                let node_addresses: Vec<Address> = group
                    .nodes
                    .iter()
                    .filter_map(|node_str| Address::from_str(node_str).ok())
                    .collect();

                // Create futures for all node log requests
                let log_futures: Vec<_> = node_addresses
                    .iter()
                    .map(|node_address| {
                        let app_state = app_state.clone();
                        let node_address = *node_address;
                        async move { fetch_node_logs(node_address, app_state).await }
                    })
                    .collect();

                let log_results = join_all(log_futures).await;

                let mut nodes = serde_json::Map::new();
                for (i, node_address) in node_addresses.iter().enumerate() {
                    let node_str = node_address.to_string();
                    nodes.insert(node_str, log_results[i].clone());
                }

                let mut all_logs = json!({
                    "success": true,
                    "group_id": group.id,
                    "configuration": group.configuration_name,
                    "created_at": group.created_at,
                });

                if let Some(obj) = all_logs.as_object_mut() {
                    obj.insert("nodes".to_string(), serde_json::Value::Object(nodes));
                }

                HttpResponse::Ok().json(all_logs)
            }
            Ok(None) => HttpResponse::NotFound().json(json!({
                "success": false,
                "error": format!("Group not found: {}", group_id.as_str())
            })),
            Err(e) => HttpResponse::InternalServerError().json(json!({
                "success": false,
                "error": format!("Failed to get group: {}", e)
            })),
        }
    } else {
        HttpResponse::ServiceUnavailable().json(json!({
            "success": false,
            "error": "Node groups plugin is not enabled"
        }))
    }
}

async fn fetch_node_logs(node_address: Address, app_state: Data<AppState>) -> serde_json::Value {
    let node = match app_state
        .store_context
        .node_store
        .get_node(&node_address)
        .await
    {
        Ok(node) => node,
        Err(e) => {
            error!("Failed to get node {}: {}", node_address, e);
            return json!({
                "success": false,
                "error": format!("Failed to get node: {}", e)
            });
        }
    };

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

            let message_signature = match sign_request(&logs_path, &app_state.wallet, None).await {
                Ok(sig) => sig,
                Err(e) => {
                    error!("Failed to sign request for node {}: {}", node_address, e);
                    return json!({
                        "success": false,
                        "error": format!("Failed to sign request: {}", e),
                        "status": node.status.to_string()
                    });
                }
            };

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

            match app_state
                .http_client
                .get(logs_url)
                .timeout(Duration::from_secs(NODE_REQUEST_TIMEOUT))
                .headers(headers)
                .send()
                .await
            {
                Ok(response) => {
                    if response.status().is_success() {
                        match response.json::<serde_json::Value>().await {
                            Ok(logs) => logs,
                            Err(e) => {
                                error!("Failed to parse logs for node {}: {}", node_address, e);
                                json!({
                                    "success": false,
                                    "error": format!("Failed to parse logs: {}", e),
                                    "status": node.status.to_string()
                                })
                            }
                        }
                    } else {
                        error!(
                            "Failed to get logs for node {}: {}",
                            node_address,
                            response.status()
                        );
                        json!({
                            "success": false,
                            "error": format!("Failed to get logs: {}", response.status()),
                            "status": node.status.to_string()
                        })
                    }
                }
                Err(e) => {
                    error!("Failed to fetch logs for node {}: {}", node_address, e);
                    json!({
                        "success": false,
                        "error": format!("Failed to connect to node: {}", e),
                        "status": node.status.to_string()
                    })
                }
            }
        }
        None => {
            error!("Node {} not found in orchestrator", node_address);
            json!({
                "success": false,
                "error": "Node not found in orchestrator",
                "address": node_address.to_string()
            })
        }
    }
}

pub fn groups_routes() -> Scope {
    web::scope("/groups")
        .route("", get().to(get_groups))
        .route("/configs", get().to(get_configurations))
        .route("/{group_id}/logs", get().to(get_group_logs))
}
