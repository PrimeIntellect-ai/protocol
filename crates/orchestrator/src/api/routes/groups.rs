use crate::api::server::AppState;
use actix_web::{
    web::{self, delete, get, post, Data},
    HttpResponse, Scope,
};
use alloy::primitives::Address;
use futures::future::join_all;
use log::error;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::str::FromStr;
use std::time::Duration;
use utoipa::ToSchema;

const NODE_REQUEST_TIMEOUT: u64 = 30;

#[derive(Debug, Deserialize, Serialize, ToSchema)]
struct ForceRegroupRequest {
    configuration_name: String,
}

#[utoipa::path(
    get,
    path = "/groups",
    responses(
        (status = 200, description = "List of all groups retrieved successfully"),
        (status = 503, description = "Node groups plugin is not enabled"),
        (status = 500, description = "Internal server error")
    ),
    tag = "groups"
)]
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

#[utoipa::path(
    get,
    path = "/groups/configs",
    responses(
        (status = 200, description = "List of all configurations retrieved successfully"),
        (status = 503, description = "Node groups plugin is not enabled")
    ),
    tag = "groups"
)]
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

#[utoipa::path(
    delete,
    path = "/groups/{group_id}",
    params(
        ("group_id" = String, Path, description = "Group ID to delete")
    ),
    responses(
        (status = 200, description = "Group deleted successfully"),
        (status = 503, description = "Node groups plugin is not enabled"),
        (status = 500, description = "Internal server error")
    ),
    tag = "groups"
)]
async fn delete_group(group_id: web::Path<String>, app_state: Data<AppState>) -> HttpResponse {
    if let Some(node_groups_plugin) = &app_state.node_groups_plugin {
        match node_groups_plugin.dissolve_group(&group_id).await {
            Ok(()) => HttpResponse::Ok().json(json!({
                "success": true,
                "message": format!("Group {} successfully deleted", group_id.as_str())
            })),
            Err(e) => HttpResponse::InternalServerError().json(json!({
                "success": false,
                "error": format!("Failed to delete group: {}", e)
            })),
        }
    } else {
        HttpResponse::ServiceUnavailable().json(json!({
            "success": false,
            "error": "Node groups plugin is not enabled"
        }))
    }
}

#[utoipa::path(
    get,
    path = "/groups/{group_id}/logs",
    params(
        ("group_id" = String, Path, description = "Group ID to get logs for")
    ),
    responses(
        (status = 200, description = "Group logs retrieved successfully"),
        (status = 404, description = "Group not found"),
        (status = 503, description = "Node groups plugin is not enabled"),
        (status = 500, description = "Internal server error")
    ),
    tag = "groups"
)]
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
            error!("Failed to get node {node_address}: {e}");
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
                        error!("Node {node_address} does not have P2P information");
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
                    error!("P2P request failed for node {node_address}: {e}");
                    json!({
                        "success": false,
                        "error": format!("P2P request failed: {}", e),
                        "status": node.status.to_string()
                    })
                }
                Err(_) => {
                    error!("P2P request timed out for node {node_address}");
                    json!({
                        "success": false,
                        "error": "P2P request timed out",
                        "status": node.status.to_string()
                    })
                }
            }
        }
        None => {
            error!("Node {node_address} not found in orchestrator");
            json!({
                "success": false,
                "error": "Node not found in orchestrator",
                "address": node_address.to_string()
            })
        }
    }
}

#[utoipa::path(
    post,
    path = "/groups/force-regroup",
    request_body = ForceRegroupRequest,
    responses(
        (status = 200, description = "Force regroup initiated successfully"),
        (status = 404, description = "Configuration not found"),
        (status = 503, description = "Node groups plugin is not enabled"),
        (status = 500, description = "Internal server error")
    ),
    tag = "groups"
)]
async fn force_regroup(
    request: web::Json<ForceRegroupRequest>,
    app_state: Data<AppState>,
) -> HttpResponse {
    if let Some(node_groups_plugin) = &app_state.node_groups_plugin {
        // Check if the configuration exists
        let all_configs = node_groups_plugin.get_all_configuration_templates();
        let config_exists = all_configs
            .iter()
            .any(|c| c.name == request.configuration_name);

        if !config_exists {
            return HttpResponse::NotFound().json(json!({
                "success": false,
                "error": format!("Configuration '{}' not found", request.configuration_name)
            }));
        }

        match node_groups_plugin.get_all_groups().await {
            Ok(groups) => {
                let groups_to_dissolve: Vec<_> = groups
                    .into_iter()
                    .filter(|g| g.configuration_name == request.configuration_name)
                    .collect();

                let group_count = groups_to_dissolve.len();
                let node_count: usize = groups_to_dissolve.iter().map(|g| g.nodes.len()).sum();

                // Dissolve all groups of this configuration
                for group in groups_to_dissolve {
                    if let Err(e) = node_groups_plugin.dissolve_group(&group.id).await {
                        error!(
                            "Failed to dissolve group {} during force regroup: {}",
                            group.id, e
                        );
                    }
                }

                HttpResponse::Ok().json(json!({
                    "success": true,
                    "message": format!(
                        "Force regroup initiated for configuration '{}'. Dissolved {} groups containing {} nodes. New groups will form automatically.",
                        request.configuration_name,
                        group_count,
                        node_count
                    ),
                    "dissolved_groups": group_count,
                    "affected_nodes": node_count
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

pub fn groups_routes() -> Scope {
    web::scope("/groups")
        .route("", get().to(get_groups))
        .route("/configs", get().to(get_configurations))
        .route("/force-regroup", post().to(force_regroup))
        .route("/{group_id}", delete().to(delete_group))
        .route("/{group_id}/logs", get().to(get_group_logs))
}
