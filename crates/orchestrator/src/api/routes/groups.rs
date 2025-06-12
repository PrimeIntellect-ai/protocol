use crate::api::server::AppState;
use actix_web::{
    web::{self, get, Data},
    HttpResponse, Scope,
};
use alloy::primitives::Address;
use futures::future::join_all;
use log::error;
use serde_json::json;
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
                        async move { fetch_node_logs_p2p(node_address, app_state).await }
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

async fn fetch_node_logs_p2p(
    node_address: Address,
    app_state: Data<AppState>,
) -> serde_json::Value {
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
            // Check if P2P client is available
            let p2p_client = app_state.p2p_client.clone();

            // Check if node has P2P information
            let (worker_p2p_id, worker_p2p_addresses) =
                match (&node.worker_p2p_id, &node.worker_p2p_addresses) {
                    (Some(p2p_id), Some(p2p_addrs)) if !p2p_addrs.is_empty() => (p2p_id, p2p_addrs),
                    _ => {
                        error!("Node {} does not have P2P information", node_address);
                        return json!({
                            "success": false,
                            "error": "Node does not have P2P information",
                            "status": node.status.to_string()
                        });
                    }
                };

            // Send P2P request for task logs
            match tokio::time::timeout(
                Duration::from_secs(NODE_REQUEST_TIMEOUT),
                p2p_client.get_task_logs(node_address, worker_p2p_id, worker_p2p_addresses),
            )
            .await
            {
                Ok(Ok(log_lines)) => {
                    json!({
                        "success": true,
                        "logs": log_lines,
                        "status": node.status.to_string()
                    })
                }
                Ok(Err(e)) => {
                    error!("P2P request failed for node {}: {}", node_address, e);
                    json!({
                        "success": false,
                        "error": format!("P2P request failed: {}", e),
                        "status": node.status.to_string()
                    })
                }
                Err(_) => {
                    error!("P2P request timed out for node {}", node_address);
                    json!({
                        "success": false,
                        "error": "P2P request timed out",
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
