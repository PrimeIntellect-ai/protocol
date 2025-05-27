use crate::plugins::traits::SchedulerPlugin;
use crate::plugins::traits::StatusUpdatePlugin;
use crate::{
    models::node::{NodeStatus, OrchestratorNode},
    plugins::node_groups::{
        NodeGroup, NodeGroupConfiguration, NodeGroupsPlugin, GROUP_KEY_PREFIX, NODE_GROUP_MAP_KEY,
    },
    store::core::{RedisStore, StoreContext},
};

use alloy::primitives::Address;
use redis::Commands;
use shared::models::{
    node::{ComputeRequirements, ComputeSpecs, GpuSpecs},
    task::{SchedulingConfig, Task, TaskState},
};
use std::collections::BTreeSet;
use std::{collections::HashMap, str::FromStr, sync::Arc};

use uuid::Uuid;

fn create_test_node(
    addr: &str,
    status: NodeStatus,
    compute_specs: Option<ComputeSpecs>,
) -> OrchestratorNode {
    // Generate a deterministic IP address from the Ethereum address
    let addr_bytes = Address::from_str(addr).unwrap().to_vec();
    let ip_address = format!(
        "{}.{}.{}.{}",
        addr_bytes[0], addr_bytes[1], addr_bytes[2], addr_bytes[3]
    );

    OrchestratorNode {
        address: Address::from_str(addr).unwrap(),
        ip_address,
        port: 8080,
        status,
        task_id: None,
        task_state: None,
        version: None,
        last_status_change: None,
        p2p_id: Some("test_p2p_id".to_string()),
        compute_specs,
    }
}

#[tokio::test]
async fn test_group_formation_and_dissolution() {
    let store = Arc::new(RedisStore::new_test());
    let context_store = store.clone();
    let store_context = Arc::new(StoreContext::new(context_store));

    let config = NodeGroupConfiguration {
        name: "test-config".to_string(),
        min_group_size: 2,
        max_group_size: 5,
        compute_requirements: None,
    };

    let plugin = NodeGroupsPlugin::new(vec![config], store.clone(), store_context);

    // Add first healthy node
    let node1 = create_test_node(
        "0x1234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    plugin.store_context.node_store.add_node(node1.clone());

    let _ = plugin
        .handle_status_change(&node1, &NodeStatus::Healthy)
        .await;

    // Add second healthy node to form group
    let node2 = create_test_node(
        "0x2234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    plugin.store_context.node_store.add_node(node2.clone());
    let _ = plugin
        .handle_status_change(&node2, &NodeStatus::Healthy)
        .await;

    // Verify group was created
    let mut conn = plugin.store.client.get_connection().unwrap();
    let group_id: Option<String> = conn
        .hget(NODE_GROUP_MAP_KEY, node1.address.to_string())
        .unwrap();
    assert!(group_id.is_some());

    // Make node unhealthy
    let node1_dead = create_test_node(
        "0x1234567890123456789012345678901234567890",
        NodeStatus::Dead,
        None,
    );
    plugin
        .store_context
        .node_store
        .update_node_status(&node1_dead.address, NodeStatus::Dead);
    let _ = plugin
        .handle_status_change(&node1_dead, &NodeStatus::Healthy)
        .await;

    // Verify group was dissolved
    let group_id: Option<String> = conn
        .hget(NODE_GROUP_MAP_KEY, node1.address.to_string())
        .unwrap();
    assert!(group_id.is_none());
}

#[tokio::test]
async fn test_group_formation_with_multiple_configs() {
    let store = Arc::new(RedisStore::new_test());
    let context_store = store.clone();
    let store_context = Arc::new(StoreContext::new(context_store));

    let config_s = NodeGroupConfiguration {
        name: "test-config-s".to_string(),
        min_group_size: 2,
        max_group_size: 2,
        compute_requirements: None,
    };

    let config_xs = NodeGroupConfiguration {
        name: "test-config-xs".to_string(),
        min_group_size: 1,
        max_group_size: 1,
        compute_requirements: None,
    };

    let plugin = NodeGroupsPlugin::new(vec![config_s, config_xs], store.clone(), store_context);

    // Add first healthy node
    let node1 = create_test_node(
        "0x1234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    plugin.store_context.node_store.add_node(node1.clone());

    // Add second healthy node to form group
    let node2 = create_test_node(
        "0x2234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    plugin.store_context.node_store.add_node(node2.clone());
    let _ = plugin
        .handle_status_change(&node1, &NodeStatus::Healthy)
        .await;

    let _ = plugin
        .handle_status_change(&node2, &NodeStatus::Healthy)
        .await;

    let node3 = create_test_node(
        "0x3234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    plugin.store_context.node_store.add_node(node3.clone());
    let _ = plugin
        .handle_status_change(&node3, &NodeStatus::Healthy)
        .await;

    let mut conn = plugin.store.client.get_connection().unwrap();
    let groups: Vec<String> = conn
        .keys(format!("{}*", GROUP_KEY_PREFIX).as_str())
        .unwrap();
    assert_eq!(groups.len(), 2);

    // Verify group was created
    let mut conn = plugin.store.client.get_connection().unwrap();
    let group_id: Option<String> = conn
        .hget(NODE_GROUP_MAP_KEY, node1.address.to_string())
        .unwrap();
    assert!(group_id.is_some());

    let group_id: Option<String> = conn
        .hget(NODE_GROUP_MAP_KEY, node2.address.to_string())
        .unwrap();
    assert!(group_id.is_some());

    let group_id: Option<String> = conn
        .hget(NODE_GROUP_MAP_KEY, node3.address.to_string())
        .unwrap();
    assert!(group_id.is_some());
}

#[tokio::test]
async fn test_group_formation_with_requirements_and_single_node() {
    let store = Arc::new(RedisStore::new_test());
    let context_store = store.clone();
    let store_context = Arc::new(StoreContext::new(context_store));

    let requirement_str = "gpu:count=8;gpu:model=RTX4090;";
    let requirements = ComputeRequirements::from_str(requirement_str).unwrap();

    let config = NodeGroupConfiguration {
        name: "test-config-with-requirements".to_string(),
        min_group_size: 1,
        max_group_size: 1,
        compute_requirements: Some(requirements),
    };

    let plugin = NodeGroupsPlugin::new(vec![config], store.clone(), store_context);

    let node1 = create_test_node(
        "0x1234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    plugin.store_context.node_store.add_node(node1.clone());
    let _ = plugin
        .handle_status_change(&node1, &NodeStatus::Healthy)
        .await;

    // Ensure node is not in a group since it does not meet requirements
    let group_id_node_1 = plugin.get_node_group(&node1.address.to_string()).unwrap();
    assert!(group_id_node_1.is_none());

    let node_2 = create_test_node(
        "0x2234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        Some(ComputeSpecs {
            gpu: Some(GpuSpecs {
                count: Some(8),
                model: Some("RTX4090".to_string()),
                memory_mb: Some(24),
                indices: Some(vec![0]),
            }),
            ..Default::default()
        }),
    );
    plugin.store_context.node_store.add_node(node_2.clone());
    let _ = plugin
        .handle_status_change(&node_2, &NodeStatus::Healthy)
        .await;

    let group_id_node_2 = plugin.get_node_group(&node_2.address.to_string()).unwrap();
    assert!(group_id_node_2.is_some());
}

#[tokio::test]
async fn test_group_formation_with_requirements_and_multiple_nodes() {
    let store = Arc::new(RedisStore::new_test());
    let context_store = store.clone();
    let store_context = Arc::new(StoreContext::new(context_store));

    let requirement_str = "gpu:count=8;gpu:model=RTX4090;";
    let requirements = ComputeRequirements::from_str(requirement_str).unwrap();

    let config = NodeGroupConfiguration {
        name: "test-config-with-requirements".to_string(),
        min_group_size: 2,
        max_group_size: 2,
        compute_requirements: Some(requirements),
    };

    let plugin = NodeGroupsPlugin::new(vec![config], store.clone(), store_context);

    let node1 = create_test_node(
        "0x1234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    plugin.store_context.node_store.add_node(node1.clone());
    let _ = plugin
        .handle_status_change(&node1, &NodeStatus::Healthy)
        .await;

    let node2 = create_test_node(
        "0x2234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        Some(ComputeSpecs {
            gpu: Some(GpuSpecs {
                count: Some(8),
                model: Some("RTX4090".to_string()),
                memory_mb: Some(24),
                indices: Some(vec![0]),
            }),
            ..Default::default()
        }),
    );
    plugin.store_context.node_store.add_node(node2.clone());
    let _ = plugin
        .handle_status_change(&node2, &NodeStatus::Healthy)
        .await;

    let group_id_node_1 = plugin.get_node_group(&node1.address.to_string()).unwrap();
    assert!(group_id_node_1.is_none());

    let group_id_node_2 = plugin.get_node_group(&node2.address.to_string()).unwrap();
    assert!(group_id_node_2.is_none());

    let node3 = create_test_node(
        "0x3234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        Some(ComputeSpecs {
            gpu: Some(GpuSpecs {
                count: Some(8),
                model: Some("RTX4090".to_string()),
                memory_mb: Some(24),
                indices: Some(vec![0]),
            }),
            ..Default::default()
        }),
    );
    plugin.store_context.node_store.add_node(node3.clone());
    let _ = plugin
        .handle_status_change(&node3, &NodeStatus::Healthy)
        .await;

    let group_id_node_3 = plugin.get_node_group(&node3.address.to_string()).unwrap();
    assert!(group_id_node_3.is_some());
    let group_id_node_2 = plugin.get_node_group(&node2.address.to_string()).unwrap();
    assert!(group_id_node_2.is_some());

    // Node 1 does not fullfill the requirements - hence it will not get added to the group
    let group_id_node_1 = plugin.get_node_group(&node1.address.to_string()).unwrap();
    assert!(group_id_node_1.is_none());
}

#[tokio::test]
async fn test_group_scheduling() {
    let store: Arc<RedisStore> = Arc::new(RedisStore::new_test());
    let context_store = store.clone();
    let store_context = Arc::new(StoreContext::new(context_store));
    let config = NodeGroupConfiguration {
        name: "test-config".to_string(),
        min_group_size: 2,
        max_group_size: 5,
        compute_requirements: None,
    };

    let plugin = NodeGroupsPlugin::new(vec![config], store.clone(), store_context);
    let node1 = create_test_node(
        "0x1234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    plugin.store_context.node_store.add_node(node1.clone());
    let node2 = create_test_node(
        "0x2234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    plugin.store_context.node_store.add_node(node2.clone());

    let mut env_vars = HashMap::new();
    env_vars.insert("LOCAL_RANK".to_string(), "0".to_string());
    env_vars.insert("RANK".to_string(), "${GROUP_INDEX}".to_string());
    env_vars.insert("WORLD_SIZE".to_string(), "${GROUP_SIZE}".to_string());
    env_vars.insert("GROUP_ID".to_string(), "${GROUP_ID}".to_string());

    let task1 = Task {
        id: Uuid::new_v4(),
        image: "prime-vllm".to_string(),
        name: "test-task".to_string(),
        env_vars: Some(env_vars),
        command: Some("uv".to_string()),
        args: Some(vec![
            "run".to_string(),
            "generate.py".to_string(),
            "--model".to_string(),
            "model/Qwen3-14B-${GROUP_INDEX}.${GROUP_SIZE}".to_string(),
            "--top-p".to_string(),
            "0.95".to_string(),
            "--group-id".to_string(),
            "${GROUP_ID}".to_string(),
        ]),
        state: TaskState::PENDING,
        created_at: 0,
        ..Default::default()
    };
    plugin.store_context.task_store.add_task(task1.clone());

    let mut task2 = task1.clone();
    task2.id = Uuid::new_v4();
    plugin.store_context.task_store.add_task(task2.clone());

    let mut task3 = task1.clone();
    task3.id = Uuid::new_v4();
    plugin.store_context.task_store.add_task(task3.clone());

    let tasks = vec![task1, task2, task3];

    let filtered_tasks = plugin.filter_tasks(&tasks, &node1.address);
    assert_eq!(filtered_tasks.len(), 0);

    let _ = plugin
        .handle_status_change(&node1, &NodeStatus::Healthy)
        .await;
    let mut tasks_clone = tasks.clone();
    tasks_clone.reverse();
    assert_ne!(tasks_clone[0].id, tasks[0].id);

    let (filtered_tasks_1, filtered_tasks_2) = tokio::join!(
        async { plugin.filter_tasks(&tasks, &node1.address) },
        async { plugin.filter_tasks(&tasks_clone, &node2.address) }
    );

    // Check both nodes get assigned valid and different indexes
    // Also ensure both nodes get the same task
    assert_eq!(filtered_tasks_1.len(), 1);
    let task_node_1 = &filtered_tasks_1[0];
    let env_vars_1 = task_node_1.env_vars.as_ref().unwrap();
    assert_eq!(env_vars_1.get("GROUP_INDEX").unwrap(), "0");
    assert_eq!(env_vars_1.get("RANK").unwrap(), "0");
    assert_eq!(env_vars_1.get("WORLD_SIZE").unwrap(), "2");
    assert_eq!(task_node_1.args.as_ref().unwrap()[3], "model/Qwen3-14B-0.2");
    assert_ne!(env_vars_1.get("GROUP_ID").unwrap(), "${GROUP_ID}");

    assert_eq!(filtered_tasks_2.len(), 1);
    let task_node_2 = &filtered_tasks_2[0];
    let env_vars_2 = task_node_2.env_vars.as_ref().unwrap();
    assert_eq!(env_vars_2.get("GROUP_INDEX").unwrap(), "1");
    assert_eq!(env_vars_2.get("RANK").unwrap(), "1");
    assert_eq!(env_vars_2.get("WORLD_SIZE").unwrap(), "2");
    assert_eq!(task_node_2.args.as_ref().unwrap()[3], "model/Qwen3-14B-1.2");
    assert_ne!(env_vars_2.get("GROUP_ID").unwrap(), "${GROUP_ID}");

    assert_eq!(task_node_1.id, task_node_2.id);
}

#[tokio::test]
async fn test_group_scheduling_without_tasks() {
    let store: Arc<RedisStore> = Arc::new(RedisStore::new_test());
    let context_store = store.clone();
    let store_context = Arc::new(StoreContext::new(context_store));

    let config = NodeGroupConfiguration {
        name: "test-config".to_string(),
        min_group_size: 2,
        max_group_size: 5,
        compute_requirements: None,
    };
    let plugin = NodeGroupsPlugin::new(vec![config], store.clone(), store_context);
    let node1 = create_test_node(
        "0x1234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    plugin.store_context.node_store.add_node(node1.clone());
    let node2 = create_test_node(
        "0x2234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    plugin.store_context.node_store.add_node(node2.clone());
    let tasks = vec![];

    let filtered_tasks = plugin.filter_tasks(&tasks, &node1.address);
    assert_eq!(filtered_tasks.len(), 0);

    let _ = plugin
        .handle_status_change(&node1, &NodeStatus::Healthy)
        .await;

    let filtered_tasks = plugin.filter_tasks(&tasks, &node1.address);
    assert_eq!(filtered_tasks.len(), 0);

    let filtered_tasks = plugin.filter_tasks(&tasks, &node2.address);
    assert_eq!(filtered_tasks.len(), 0);
}

#[tokio::test]
async fn test_group_formation_with_max_size() {
    let store = Arc::new(RedisStore::new_test());
    let context_store = store.clone();
    let store_context = Arc::new(StoreContext::new(context_store));

    // Set max group size to 2
    let config = NodeGroupConfiguration {
        name: "test-config".to_string(),
        min_group_size: 2,
        max_group_size: 2,
        compute_requirements: None,
    };
    let plugin = NodeGroupsPlugin::new(vec![config], store.clone(), store_context);

    // Create three healthy nodes
    let node1 = create_test_node(
        "0x1234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    plugin.store_context.node_store.add_node(node1.clone());

    let node2 = create_test_node(
        "0x2234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    plugin.store_context.node_store.add_node(node2.clone());

    let node3 = create_test_node(
        "0x3234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    plugin.store_context.node_store.add_node(node3.clone());

    // Handle status changes to trigger group formation
    let _ = plugin
        .handle_status_change(&node1, &NodeStatus::Healthy)
        .await;
    let _ = plugin
        .handle_status_change(&node2, &NodeStatus::Healthy)
        .await;
    let _ = plugin
        .handle_status_change(&node3, &NodeStatus::Healthy)
        .await;

    // Create a test task
    let mut env_vars = HashMap::new();
    env_vars.insert("RANK".to_string(), "${GROUP_INDEX}".to_string());
    env_vars.insert("WORLD_SIZE".to_string(), "${GROUP_SIZE}".to_string());

    let task = Task {
        id: Uuid::new_v4(),
        image: "test-image".to_string(),
        name: "test-task".to_string(),
        env_vars: Some(env_vars),
        command: Some("run".to_string()),
        args: Some(vec!["--index".to_string(), "${GROUP_INDEX}".to_string()]),
        state: TaskState::PENDING,
        created_at: 0,
        ..Default::default()
    };
    plugin.store_context.task_store.add_task(task.clone());

    let tasks = vec![task];

    // Check if node1 and node2 are in a group
    let group1 = plugin.get_node_group(&node1.address.to_string()).unwrap();
    let group2 = plugin.get_node_group(&node2.address.to_string()).unwrap();

    // Check if node3 is not in a group
    let group3 = plugin.get_node_group(&node3.address.to_string()).unwrap();

    // Either node1 and node2 are in a group, or node2 and node3 are in a group
    // But all three cannot be in the same group due to max_group_size=2
    assert!(
        (group1.is_some()
            && group2.is_some()
            && group1.as_ref() == group2.as_ref()
            && group3.is_none())
            || (group2.is_some()
                && group3.is_some()
                && group2.as_ref() == group3.as_ref()
                && group1.is_none())
            || (group1.is_some()
                && group3.is_some()
                && group1.as_ref() == group3.as_ref()
                && group2.is_none())
    );

    // Verify that tasks are only assigned to nodes in a group
    for node in [&node1, &node2, &node3] {
        let filtered_tasks = plugin.filter_tasks(&tasks, &node.address);
        let group = plugin.get_node_group(&node.address.to_string()).unwrap();

        if group.is_some() {
            assert_eq!(
                filtered_tasks.len(),
                1,
                "Node in group should receive tasks"
            );
        } else {
            assert_eq!(
                filtered_tasks.len(),
                0,
                "Node not in group should not receive tasks"
            );
        }
    }
}

#[tokio::test]
async fn test_node_groups_with_allowed_topologies() {
    let store = Arc::new(RedisStore::new_test());
    let context_store = store.clone();
    let store_context = Arc::new(StoreContext::new(context_store));

    let config = NodeGroupConfiguration {
        name: "test-config".to_string(),
        min_group_size: 1,
        max_group_size: 1,
        compute_requirements: None,
    };

    let plugin = NodeGroupsPlugin::new(vec![config], store.clone(), store_context);

    let node1 = create_test_node(
        "0x1234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    plugin.store_context.node_store.add_node(node1.clone());
    let _ = plugin
        .handle_status_change(&node1, &NodeStatus::Healthy)
        .await;

    let task_no_match = Task {
        id: Uuid::new_v4(),
        image: "test-image".to_string(),
        name: "test-task".to_string(),
        env_vars: None,
        command: Some("run".to_string()),
        args: Some(vec!["--index".to_string(), "${GROUP_INDEX}".to_string()]),
        scheduling_config: Some(SchedulingConfig {
            plugins: Some(HashMap::from([(
                "node_groups".to_string(),
                HashMap::from([(
                    "allowed_topologies".to_string(),
                    vec!["no-match-config".to_string()],
                )]),
            )])),
        }),
        state: TaskState::PENDING,
        created_at: 0,
        updated_at: None,
        ..Default::default()
    };
    plugin
        .store_context
        .task_store
        .add_task(task_no_match.clone());

    let mut tasks = vec![task_no_match];

    let filtered_tasks = plugin.filter_tasks(&tasks, &node1.address);
    assert_eq!(filtered_tasks.len(), 0);

    let task_match = Task {
        id: Uuid::new_v4(),
        image: "test-image".to_string(),
        name: "test-task".to_string(),
        env_vars: None,
        command: Some("run".to_string()),
        args: Some(vec!["--index".to_string(), "${GROUP_INDEX}".to_string()]),
        scheduling_config: Some(SchedulingConfig {
            plugins: Some(HashMap::from([(
                "node_groups".to_string(),
                HashMap::from([(
                    "allowed_topologies".to_string(),
                    vec!["test-config".to_string()],
                )]),
            )])),
        }),
        ..Default::default()
    };

    plugin.store_context.task_store.add_task(task_match.clone());
    tasks.push(task_match);
    let filtered_tasks = plugin.filter_tasks(&tasks, &node1.address);
    assert_eq!(filtered_tasks.len(), 1);
}

#[tokio::test]
async fn test_node_cannot_be_in_multiple_groups() {
    let store = Arc::new(RedisStore::new_test());
    let context_store = store.clone();
    let store_context = Arc::new(StoreContext::new(context_store));
    let config = NodeGroupConfiguration {
        name: "test-config".to_string(),
        min_group_size: 2,
        max_group_size: 2,
        compute_requirements: None,
    };

    // Set max_group_size to 2, so groups can only have 2 nodes
    let plugin = NodeGroupsPlugin::new(vec![config], store.clone(), store_context);

    let all_nodes = plugin.store_context.node_store.get_nodes();
    assert_eq!(all_nodes.len(), 0, "No nodes should be in the store");

    // Create three nodes
    let node1 = create_test_node(
        "0x1234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    let node2 = create_test_node(
        "0x2234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    let node3 = create_test_node(
        "0x3234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );

    // Add nodes to the store
    plugin.store_context.node_store.add_node(node1.clone());
    plugin.store_context.node_store.add_node(node2.clone());
    plugin.store_context.node_store.add_node(node3.clone());

    // Add nodes to groups through the normal flow
    let _ = plugin
        .handle_status_change(&node1, &NodeStatus::Healthy)
        .await;
    let _ = plugin
        .handle_status_change(&node2, &NodeStatus::Healthy)
        .await;
    let _ = plugin
        .handle_status_change(&node3, &NodeStatus::Healthy)
        .await;

    // Get connection to check Redis state
    let mut conn = plugin.store.client.get_connection().unwrap();

    // Verify each node's group assignment
    let node1_group_id: Option<String> = conn
        .hget(NODE_GROUP_MAP_KEY, node1.address.to_string())
        .unwrap();
    let node2_group_id: Option<String> = conn
        .hget(NODE_GROUP_MAP_KEY, node2.address.to_string())
        .unwrap();
    let node3_group_id: Option<String> = conn
        .hget(NODE_GROUP_MAP_KEY, node3.address.to_string())
        .unwrap();

    // With 3 nodes and max_group_size=2, one node MUST NOT be in a group
    let nodes_without_group = [&node1_group_id, &node2_group_id, &node3_group_id]
        .iter()
        .filter(|id| id.is_none())
        .count();

    assert_eq!(
        nodes_without_group, 1,
        "With 3 nodes and max_group_size=2, exactly one node must not be in a group"
    );

    // The other two nodes must be in the same group
    let group_ids: Vec<_> = [node1_group_id, node2_group_id, node3_group_id]
        .iter()
        .filter_map(|x| x.clone())
        .collect();
    assert_eq!(group_ids.len(), 2, "Exactly 2 nodes should have group IDs");
    assert_eq!(
        group_ids[0], group_ids[1],
        "The 2 nodes in groups should be in the same group"
    );

    // Get all group keys
    let group_keys: Vec<String> = conn.keys(format!("{}*", GROUP_KEY_PREFIX)).unwrap();
    let group_copy = group_keys.clone();

    // There should be exactly one group

    // Count how many groups each node appears in
    let mut node1_group_count = 0;
    let mut node2_group_count = 0;
    let mut node3_group_count = 0;

    for key in group_keys {
        let group_data: String = conn.get(&key).unwrap();
        let group: NodeGroup = serde_json::from_str(&group_data).unwrap();

        // Verify the group has exactly 2 nodes
        assert_eq!(group.nodes.len(), 2, "Group should have exactly 2 nodes");

        if group.nodes.contains(&node1.address.to_string()) {
            node1_group_count += 1;
        }
        if group.nodes.contains(&node2.address.to_string()) {
            node2_group_count += 1;
        }
        if group.nodes.contains(&node3.address.to_string()) {
            node3_group_count += 1;
        }
    }

    assert_eq!(group_copy.len(), 1, "There should be exactly one group");

    // Total group count should be 2 (exactly 2 nodes in groups)
    assert_eq!(
        node1_group_count + node2_group_count + node3_group_count,
        2,
        "Exactly 2 nodes should be in groups"
    );

    // Each node should appear in at most one group
    assert!(
        node1_group_count <= 1,
        "Node1 should be in at most one group"
    );
    assert!(
        node2_group_count <= 1,
        "Node2 should be in at most one group"
    );
    assert!(
        node3_group_count <= 1,
        "Node3 should be in at most one group"
    );

    // Add a fourth node and make it healthy
    let node4 = create_test_node(
        "0x4234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    plugin.store_context.node_store.add_node(node4.clone());
    let _ = plugin
        .handle_status_change(&node4, &NodeStatus::Healthy)
        .await;

    // Get updated group keys
    let group_keys: Vec<String> = conn.keys(format!("{}*", GROUP_KEY_PREFIX)).unwrap();

    // There should now be exactly two groups
    assert_eq!(
        group_keys.len(),
        2,
        "There should be exactly two groups after adding node4"
    );

    // Verify each node's updated group assignment
    let node1_group_id: Option<String> = conn
        .hget(NODE_GROUP_MAP_KEY, node1.address.to_string())
        .unwrap();
    let node2_group_id: Option<String> = conn
        .hget(NODE_GROUP_MAP_KEY, node2.address.to_string())
        .unwrap();
    let node3_group_id: Option<String> = conn
        .hget(NODE_GROUP_MAP_KEY, node3.address.to_string())
        .unwrap();
    let node4_group_id: Option<String> = conn
        .hget(NODE_GROUP_MAP_KEY, node4.address.to_string())
        .unwrap();

    // All nodes should now be in a group
    assert!(node1_group_id.is_some(), "Node1 should be in a group");
    assert!(node2_group_id.is_some(), "Node2 should be in a group");
    assert!(node3_group_id.is_some(), "Node3 should be in a group");
    assert!(node4_group_id.is_some(), "Node4 should be in a group");

    // Verify that we have exactly two distinct group IDs
    let all_group_ids = [
        node1_group_id.unwrap(),
        node2_group_id.unwrap(),
        node3_group_id.unwrap(),
        node4_group_id.unwrap(),
    ];
    let unique_group_ids: std::collections::HashSet<_> = all_group_ids.iter().collect();
    assert_eq!(
        unique_group_ids.len(),
        2,
        "There should be exactly two distinct group IDs"
    );
}

#[tokio::test]
async fn test_reformation_on_death() {
    let store = Arc::new(RedisStore::new_test());
    let context_store = store.clone();
    let store_context = Arc::new(StoreContext::new(context_store));
    let config = NodeGroupConfiguration {
        name: "test-config".to_string(),
        min_group_size: 2,
        max_group_size: 2,
        compute_requirements: None,
    };
    let plugin = NodeGroupsPlugin::new(vec![config], store.clone(), store_context);

    let all_nodes = plugin.store_context.node_store.get_nodes();
    assert_eq!(all_nodes.len(), 0, "No nodes should be in the store");

    // Create three nodes
    let node1 = create_test_node(
        "0x9234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    let mut node2 = create_test_node(
        "0x8234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );

    // Add nodes to the store
    plugin.store_context.node_store.add_node(node1.clone());
    plugin.store_context.node_store.add_node(node2.clone());

    // Add nodes to groups through the normal flow
    let _ = plugin
        .handle_status_change(&node1, &NodeStatus::Healthy)
        .await;
    let _ = plugin
        .handle_status_change(&node2, &NodeStatus::Healthy)
        .await;

    // Get connection to check Redis state
    let mut conn = plugin.store.client.get_connection().unwrap();

    // Verify each node's group assignment
    let node1_group_id: Option<String> = conn
        .hget(NODE_GROUP_MAP_KEY, node1.address.to_string())
        .unwrap();
    let node2_group_id: Option<String> = conn
        .hget(NODE_GROUP_MAP_KEY, node2.address.to_string())
        .unwrap();

    assert!(node1_group_id.is_some(), "Node1 should be in a group");
    assert!(node2_group_id.is_some(), "Node2 should be in a group");

    let node_3 = create_test_node(
        "0x3234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    plugin.store_context.node_store.add_node(node_3.clone());

    let node_3_group_id: Option<String> = conn
        .hget(NODE_GROUP_MAP_KEY, node_3.address.to_string())
        .unwrap();

    assert!(node_3_group_id.is_none(), "Node3 should not be in a group");

    node2.status = NodeStatus::Dead;
    plugin
        .store_context
        .node_store
        .update_node_status(&node2.address, NodeStatus::Dead);
    let _ = plugin
        .handle_status_change(&node2, &NodeStatus::Healthy)
        .await;

    let node_2_group_id: Option<String> = conn
        .hget(NODE_GROUP_MAP_KEY, node2.address.to_string())
        .unwrap();
    let node_1_group_id: Option<String> = conn
        .hget(NODE_GROUP_MAP_KEY, node1.address.to_string())
        .unwrap();
    let node_3_group_id: Option<String> = conn
        .hget(NODE_GROUP_MAP_KEY, node_3.address.to_string())
        .unwrap();

    assert!(node_2_group_id.is_none(), "Node2 should not be in a group");
    assert!(node_1_group_id.is_some(), "Node1 should be in a group");
    assert!(node_3_group_id.is_some(), "Node3 should be in a group");
}

#[tokio::test]
#[should_panic(expected = "Configuration names must be unique")]
async fn ensure_config_names_are_unique() {
    let store = Arc::new(RedisStore::new_test());
    let context_store = store.clone();
    let store_context = Arc::new(StoreContext::new(context_store));

    let config1 = NodeGroupConfiguration {
        name: "test-config".to_string(),
        min_group_size: 2,
        max_group_size: 2,
        compute_requirements: None,
    };
    let config2 = NodeGroupConfiguration {
        name: "test-config".to_string(),
        min_group_size: 2,
        max_group_size: 2,
        compute_requirements: None,
    };

    let _plugin = NodeGroupsPlugin::new(vec![config1, config2], store.clone(), store_context);
}

#[tokio::test]
#[should_panic(expected = "Plugin configuration is invalid")]
async fn ensure_config_validation() {
    let store = Arc::new(RedisStore::new_test());
    let context_store = store.clone();
    let store_context = Arc::new(StoreContext::new(context_store));

    let config = NodeGroupConfiguration {
        name: "test-config".to_string(),
        min_group_size: 3,
        max_group_size: 2, // Invalid: max < min
        compute_requirements: None,
    };

    let _plugin = NodeGroupsPlugin::new(vec![config], store.clone(), store_context);
}

#[tokio::test]
async fn test_get_idx_in_group() {
    let store = Arc::new(RedisStore::new_test());
    let context_store = store.clone();
    let store_context = Arc::new(StoreContext::new(context_store));

    let plugin = NodeGroupsPlugin::new(vec![], store.clone(), store_context);

    let node1 = create_test_node(
        "0x1234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    let node2 = create_test_node(
        "0x2234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );
    let node3 = create_test_node(
        "0x3234567890123456789012345678901234567890",
        NodeStatus::Healthy,
        None,
    );

    plugin.store_context.node_store.add_node(node1.clone());
    plugin.store_context.node_store.add_node(node2.clone());
    plugin.store_context.node_store.add_node(node3.clone());

    let group = NodeGroup {
        id: "test-group".to_string(),
        nodes: BTreeSet::from([
            node1.address.to_string(),
            node2.address.to_string(),
            node3.address.to_string(),
        ]),
        created_at: chrono::Utc::now(),
        configuration_name: "test-config".to_string(),
    };

    let idx = plugin
        .get_idx_in_group(&group, &node1.address.to_string())
        .unwrap();
    assert_eq!(idx, 0);

    let idx = plugin
        .get_idx_in_group(&group, &node2.address.to_string())
        .unwrap();
    assert_eq!(idx, 1);

    let idx = plugin
        .get_idx_in_group(&group, &node3.address.to_string())
        .unwrap();
    assert_eq!(idx, 2);
}

#[tokio::test]
async fn test_get_idx_in_group_not_found() {
    let store = Arc::new(RedisStore::new_test());
    let context_store = store.clone();
    let store_context = Arc::new(StoreContext::new(context_store));

    let plugin = NodeGroupsPlugin::new(vec![], store.clone(), store_context);

    let group = NodeGroup {
        id: "test-group".to_string(),
        nodes: BTreeSet::from(["0x1234567890123456789012345678901234567890".to_string()]),
        created_at: chrono::Utc::now(),
        configuration_name: "test-config".to_string(),
    };

    let result = plugin.get_idx_in_group(&group, "0x2234567890123456789012345678901234567890");
    assert!(result.is_err());
}
