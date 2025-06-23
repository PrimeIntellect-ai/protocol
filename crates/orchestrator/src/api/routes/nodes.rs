use crate::api::server::AppState;
use actix_web::{
    web::{self, get, post, Data, Query},
    HttpResponse, Scope,
};
use alloy::primitives::Address;
use log::{error, info};
use serde::Deserialize;
use serde_json::json;
use std::str::FromStr;

#[derive(Deserialize)]
struct NodeQuery {
    include_dead: Option<bool>,
}

#[utoipa::path(
    get,
    path = "/nodes",
    params(
        ("include_dead" = Option<bool>, Query, description = "Include dead nodes in the response")
    ),
    responses(
        (status = 200, description = "List of nodes retrieved successfully"),
        (status = 500, description = "Internal server error")
    ),
    tag = "nodes"
)]
async fn get_nodes(query: Query<NodeQuery>, app_state: Data<AppState>) -> HttpResponse {
    let nodes = match app_state.store_context.node_store.get_nodes().await {
        Ok(mut nodes) => {
            // Filter out dead nodes unless include_dead is true
            if !query.include_dead.unwrap_or(false) {
                nodes.retain(|node| node.status != crate::models::node::NodeStatus::Dead);
            }
            nodes
        }
        Err(e) => {
            error!("Error getting nodes: {}", e);
            return HttpResponse::InternalServerError().json(json!({
                "success": false,
                "error": "Failed to get nodes"
            }));
        }
    };

    let mut status_counts = json!({});
    for node in &nodes {
        let status_str = format!("{:?}", node.status);
        if let Some(count) = status_counts.get(&status_str) {
            if let Some(count_value) = count.as_u64() {
                status_counts[status_str] = json!(count_value + 1);
            } else {
                status_counts[status_str] = json!(1);
            }
        } else {
            status_counts[status_str] = json!(1);
        }
    }

    let mut response = json!({
        "success": true,
        "nodes": nodes,
        "counts": status_counts
    });

    // If node groups plugin exists, add group information to each node
    if let Some(node_groups_plugin) = &app_state.node_groups_plugin {
        let mut nodes_with_groups = Vec::new();

        // Batch fetch all node groups at once to eliminate N+1 queries
        let node_addresses: Vec<String> =
            nodes.iter().map(|node| node.address.to_string()).collect();

        match node_groups_plugin
            .get_node_groups_batch(&node_addresses)
            .await
        {
            Ok(node_groups) => {
                for node in &nodes {
                    let mut node_json = json!(node);

                    if let Some(Some(group)) = node_groups.get(&node.address.to_string()) {
                        node_json["group"] = json!({
                            "id": group.id,
                            "size": group.nodes.len(),
                            "created_at": group.created_at,
                            "topology_config": group.configuration_name
                        });
                    }

                    nodes_with_groups.push(node_json);
                }
            }
            Err(e) => {
                error!("Error getting node groups batch: {}", e);
                // Fall back to nodes without group information
                nodes_with_groups = nodes.iter().map(|node| json!(node)).collect();
            }
        }

        response["nodes"] = json!(nodes_with_groups);
    }

    HttpResponse::Ok().json(response)
}

#[utoipa::path(
    post,
    path = "/nodes/{node_id}/restart",
    params(
        ("node_id" = String, Path, description = "Node address to restart task for")
    ),
    responses(
        (status = 200, description = "Task restarted successfully"),
        (status = 400, description = "Bad request - invalid node address or node missing p2p information"),
        (status = 404, description = "Node not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "nodes"
)]
async fn restart_node_task(node_id: web::Path<String>, app_state: Data<AppState>) -> HttpResponse {
    let node_address = match Address::from_str(&node_id) {
        Ok(address) => address,
        Err(_) => {
            return HttpResponse::BadRequest().json(json!({
                "success": false,
                "error": format!("Invalid node address: {}", node_id)
            }));
        }
    };

    let node = match app_state
        .store_context
        .node_store
        .get_node(&node_address)
        .await
    {
        Ok(Some(node)) => node,
        Ok(None) => {
            return HttpResponse::NotFound().json(json!({
                "success": false,
                "error": format!("Node not found: {}", node_id)
            }));
        }
        Err(e) => {
            error!("Error getting node: {}", e);
            return HttpResponse::InternalServerError().json(json!({
                "success": false,
                "error": "Failed to get node"
            }));
        }
    };

    if node.worker_p2p_id.is_none() || node.worker_p2p_addresses.is_none() {
        return HttpResponse::BadRequest().json(json!({
            "success": false,
            "error": "Node does not have p2p information"
        }));
    }

    let p2p_id = node.worker_p2p_id.as_ref().unwrap();
    let p2p_addresses = node.worker_p2p_addresses.as_ref().unwrap();

    match app_state
        .p2p_client
        .restart_task(node_address, p2p_id, p2p_addresses)
        .await
    {
        Ok(_) => HttpResponse::Ok().json(json!({
            "success": true,
            "message": "Task restarted successfully"
        })),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "success": false,
            "error": format!("Failed to restart task: {}", e)
        })),
    }
}

#[utoipa::path(
    get,
    path = "/nodes/{node_id}/logs",
    params(
        ("node_id" = String, Path, description = "Node address to get logs for")
    ),
    responses(
        (status = 200, description = "Node logs retrieved successfully"),
        (status = 400, description = "Bad request - invalid node address or node missing p2p information"),
        (status = 500, description = "Internal server error")
    ),
    tag = "nodes"
)]
async fn get_node_logs(node_id: web::Path<String>, app_state: Data<AppState>) -> HttpResponse {
    let node_address = match Address::from_str(&node_id) {
        Ok(address) => address,
        Err(_) => {
            return HttpResponse::BadRequest().json(json!({
                "success": false,
                "error": format!("Invalid node address: {}", node_id)
            }));
        }
    };

    let node = match app_state
        .store_context
        .node_store
        .get_node(&node_address)
        .await
    {
        Ok(Some(node)) => node,
        Ok(None) => {
            return HttpResponse::Ok().json(json!({"success": false, "logs": "Node not found"}));
        }
        Err(e) => {
            error!("Error getting node: {}", e);
            return HttpResponse::InternalServerError().json(json!({
                "success": false,
                "error": "Failed to get node"
            }));
        }
    };

    if node.worker_p2p_id.is_none() || node.worker_p2p_addresses.is_none() {
        return HttpResponse::BadRequest().json(json!({
            "success": false,
            "error": "Node does not have p2p information"
        }));
    }

    let p2p_id = node.worker_p2p_id.as_ref().unwrap();
    let p2p_addresses = node.worker_p2p_addresses.as_ref().unwrap();

    match app_state
        .p2p_client
        .get_task_logs(node_address, p2p_id, p2p_addresses)
        .await
    {
        Ok(logs) => HttpResponse::Ok().json(json!({
            "success": true,
            "logs": logs
        })),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "success": false,
            "error": format!("Failed to get logs: {}", e)
        })),
    }
}

#[utoipa::path(
    get,
    path = "/nodes/{node_id}/metrics",
    params(
        ("node_id" = String, Path, description = "Node address to get metrics for")
    ),
    responses(
        (status = 200, description = "Node metrics retrieved successfully"),
        (status = 400, description = "Bad request - invalid node address"),
        (status = 500, description = "Internal server error")
    ),
    tag = "nodes"
)]
async fn get_node_metrics(node_id: web::Path<String>, app_state: Data<AppState>) -> HttpResponse {
    let node_address = match Address::from_str(&node_id) {
        Ok(address) => address,
        Err(_) => {
            return HttpResponse::BadRequest().json(json!({
                "success": false,
                "error": format!("Invalid node address: {}", node_id)
            }));
        }
    };

    let metrics = match app_state
        .store_context
        .metrics_store
        .get_metrics_for_node(node_address)
        .await
    {
        Ok(metrics) => metrics,
        Err(e) => {
            error!("Error getting metrics for node: {}", e);
            Default::default()
        }
    };
    HttpResponse::Ok().json(json!({"success": true, "metrics": metrics}))
}

#[utoipa::path(
    post,
    path = "/nodes/{node_id}/ban",
    params(
        ("node_id" = String, Path, description = "Node address to ban")
    ),
    responses(
        (status = 200, description = "Node banned and ejected successfully"),
        (status = 400, description = "Bad request - invalid node address"),
        (status = 404, description = "Node not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "nodes"
)]
async fn ban_node(node_id: web::Path<String>, app_state: Data<AppState>) -> HttpResponse {
    info!("banning node: {}", node_id);
    let node_address = match Address::from_str(&node_id) {
        Ok(address) => address,
        Err(_) => {
            return HttpResponse::BadRequest().json(json!({
                "success": false,
                "error": format!("Invalid node address: {}", node_id)
            }));
        }
    };

    let node = match app_state
        .store_context
        .node_store
        .get_node(&node_address)
        .await
    {
        Ok(Some(node)) => node,
        Ok(None) => {
            return HttpResponse::NotFound().json(json!({
                "success": false,
                "error": format!("Node not found: {}", node_id)
            }));
        }
        Err(e) => {
            error!("Error getting node: {}", e);
            return HttpResponse::InternalServerError().json(json!({
                "success": false,
                "error": "Failed to get node"
            }));
        }
    };

    if let Err(e) = app_state
        .store_context
        .node_store
        .update_node_status(&node.address, crate::models::node::NodeStatus::Banned)
        .await
    {
        error!("Error updating node status: {}", e);
        return HttpResponse::InternalServerError().json(json!({
            "success": false,
            "error": "Failed to update node status"
        }));
    }

    // Attempt to eject from pool
    if let Some(contracts) = &app_state.contracts {
        match contracts
            .compute_pool
            .eject_node(app_state.pool_id, node.address)
            .await
        {
            Ok(_) => HttpResponse::Ok().json(json!({
                "success": true,
                "message": format!("Node {} successfully ejected", node_id)
            })),
            Err(e) => HttpResponse::InternalServerError().json(json!({
                "success": false,
                "error": format!("Failed to eject node from pool: {}", e)
            })),
        }
    } else {
        HttpResponse::InternalServerError().json(json!({
            "success": false,
            "error": "Contracts not found"
        }))
    }
}

pub fn nodes_routes() -> Scope {
    web::scope("/nodes")
        .route("", get().to(get_nodes))
        .route("/{node_id}/restart", post().to(restart_node_task))
        .route("/{node_id}/logs", get().to(get_node_logs))
        .route("/{node_id}/metrics", get().to(get_node_metrics))
        .route("/{node_id}/ban", post().to(ban_node))
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
            ..Default::default()
        };
        app_state
            .store_context
            .node_store
            .add_node(node.clone())
            .await
            .unwrap();

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
}
