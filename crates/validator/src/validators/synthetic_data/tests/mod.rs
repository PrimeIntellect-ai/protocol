use crate::metrics::export_metrics;

use super::*;
use alloy::primitives::Address;
use anyhow::Ok;
use mockito::Server;
use shared::utils::MockStorageProvider;
use shared::web3::contracts::core::builder::{ContractBuilder, Contracts};
use shared::web3::wallet::Wallet;
use std::str::FromStr;
use url::Url;

fn test_store() -> RedisStore {
    let store = RedisStore::new_test();
    let mut con = store
        .client
        .get_connection()
        .expect("Should connect to test Redis instance");

    redis::cmd("PING")
        .query::<String>(&mut con)
        .expect("Redis should be responsive");
    redis::cmd("FLUSHALL")
        .query::<String>(&mut con)
        .expect("Redis should be flushed");
    store
}

fn setup_test_env() -> Result<(RedisStore, Contracts<WalletProvider>), Error> {
    let store = test_store();
    let url = Url::parse("http://localhost:8545").unwrap();

    let demo_wallet = Wallet::new(
        "0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97",
        url,
    )
    .map_err(|e| Error::msg(format!("Failed to create demo wallet: {}", e)))?;

    let contracts = ContractBuilder::new(demo_wallet.provider())
        .with_compute_registry()
        .with_ai_token()
        .with_prime_network()
        .with_compute_pool()
        .with_domain_registry()
        .with_stake_manager()
        .with_synthetic_data_validator(Some(Address::ZERO))
        .build()
        .map_err(|e| Error::msg(format!("Failed to build contracts: {}", e)))?;

    Ok((store, contracts))
}

#[tokio::test]
async fn test_build_validation_plan() -> Result<(), Error> {
    let (store, contracts) = setup_test_env()?;
    let metrics_context = MetricsContext::new("0".to_string(), Some("0".to_string()));
    let mock_storage = MockStorageProvider::new();

    let single_group_file_name = "Qwen3/dataset/samplingn-9999999-1-9-0.parquet";
    mock_storage.add_file(single_group_file_name, "file1").await;
    mock_storage
        .add_mapping_file(
            "9999999999999999999999999999999999999999999999999999999999999999",
            single_group_file_name,
        )
        .await;

    let single_unknown_file_name = "Qwen3/dataset/samplingn-8888888-1-9-0.parquet";
    mock_storage
        .add_file(single_unknown_file_name, "file1")
        .await;
    mock_storage
        .add_mapping_file(
            "8888888888888888888888888888888888888888888888888888888888888888",
            single_unknown_file_name,
        )
        .await;

    mock_storage
        .add_file(
            "Qwen3/dataset/samplingn-3450756714426841564-2-9-0.parquet",
            "file1",
        )
        .await;
    mock_storage
        .add_file(
            "Qwen3/dataset/samplingn-3450756714426841564-2-9-1.parquet",
            "file2",
        )
        .await;
    mock_storage
        .add_mapping_file(
            "c257e3d3fe866a00df1285f8bbbe601fed6b85229d983bbbb75e19a068346641",
            "Qwen3/dataset/samplingn-3450756714426841564-2-9-0.parquet",
        )
        .await;
    mock_storage
        .add_mapping_file(
            "88e4672c19e5a10bff2e23d223f8bfc38ae1425feaa18db9480e631a4fd98edf",
            "Qwen3/dataset/samplingn-3450756714426841564-2-9-1.parquet",
        )
        .await;

    let storage_provider = Arc::new(mock_storage);

    let validator = SyntheticDataValidator::new(
        "0".to_string(),
        contracts.synthetic_data_validator.clone().unwrap(),
        contracts.prime_network.clone(),
        vec![ToplocConfig {
            server_url: "http://localhost:8080".to_string(),
            ..Default::default()
        }],
        U256::from(0),
        storage_provider,
        store,
        CancellationToken::new(),
        10,
        60,
        1,
        10,
        true,
        false,
        0, // incomplete_group_grace_period_minutes (disabled)
        InvalidationType::Hard,
        InvalidationType::Hard,
        Some(metrics_context),
    );

    let work_keys = vec![
        "9999999999999999999999999999999999999999999999999999999999999999".to_string(),
        "c257e3d3fe866a00df1285f8bbbe601fed6b85229d983bbbb75e19a068346641".to_string(),
        "88e4672c19e5a10bff2e23d223f8bfc38ae1425feaa18db9480e631a4fd98edf".to_string(),
        "8888888888888888888888888888888888888888888888888888888888888888".to_string(),
    ];

    let work_info = WorkInfo {
        node_id: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
        work_units: U256::from(1000),
        ..Default::default()
    };
    for work_key in work_keys.clone() {
        validator
            .update_work_info_in_redis(&work_key, &work_info)
            .await?;
    }
    validator
        .update_work_validation_status(&work_keys[3], &ValidationResult::Unknown)
        .await?;

    let validation_plan = validator.build_validation_plan(work_keys.clone()).await?;
    assert_eq!(validation_plan.single_trigger_tasks.len(), 0);
    assert_eq!(validation_plan.group_trigger_tasks.len(), 2);
    assert_eq!(validation_plan.status_check_tasks.len(), 0);
    assert_eq!(validation_plan.group_status_check_tasks.len(), 1);

    let metrics = export_metrics().unwrap();
    assert!(metrics.contains("validator_work_keys_to_process{pool_id=\"0\",validator_id=\"0\"} 4"));

    Ok(())
}

#[tokio::test]
async fn test_status_update() -> Result<(), Error> {
    let (store, contracts) = setup_test_env()?;
    let mock_storage = MockStorageProvider::new();
    let storage_provider = Arc::new(mock_storage);

    let validator = SyntheticDataValidator::new(
        "0".to_string(),
        contracts.synthetic_data_validator.clone().unwrap(),
        contracts.prime_network.clone(),
        vec![ToplocConfig {
            server_url: "http://localhost:8080".to_string(),
            ..Default::default()
        }],
        U256::from(0),
        storage_provider,
        store,
        CancellationToken::new(),
        10,
        60,
        1,
        10,
        false,
        false,
        0, // incomplete_group_grace_period_minutes (disabled)
        InvalidationType::Hard,
        InvalidationType::Hard,
        None,
    );
    validator
        .update_work_validation_status(
            "0x0000000000000000000000000000000000000000",
            &ValidationResult::Accept,
        )
        .await
        .map_err(|e| {
            error!("Failed to update work validation status: {}", e);
            Error::msg(format!("Failed to update work validation status: {}", e))
        })?;

    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    let status = validator
        .get_work_validation_status_from_redis("0x0000000000000000000000000000000000000000")
        .await
        .map_err(|e| {
            error!("Failed to get work validation status: {}", e);
            Error::msg(format!("Failed to get work validation status: {}", e))
        })?;
    assert_eq!(status, Some(ValidationResult::Accept));
    Ok(())
}

#[tokio::test]
async fn test_group_filename_parsing() -> Result<(), Error> {
    // Test case 1: Valid filename with all components
    let name = "Qwen3/dataset/samplingn-3450756714426841564-2-9-1.parquet";
    let group_info = GroupInformation::from_str(name)?;

    assert_eq!(
        group_info.group_file_name,
        "Qwen3/dataset/samplingn-3450756714426841564-2-9.parquet"
    );
    assert_eq!(group_info.prefix, "Qwen3/dataset/samplingn");
    assert_eq!(group_info.group_id, "3450756714426841564");
    assert_eq!(group_info.group_size, 2);
    assert_eq!(group_info.file_number, 9);
    assert_eq!(group_info.idx, "1");

    // Test case 2: Invalid filename format
    let invalid_name = "invalid-filename.parquet";
    assert!(GroupInformation::from_str(invalid_name).is_err());

    // Test case 3: Filename with different numbers
    let name2 = "test/dataset/data-123-5-10-3.parquet";
    let group_info2 = GroupInformation::from_str(name2)?;

    assert_eq!(
        group_info2.group_file_name,
        "test/dataset/data-123-5-10.parquet"
    );
    assert_eq!(group_info2.prefix, "test/dataset/data");
    assert_eq!(group_info2.group_id, "123");
    assert_eq!(group_info2.group_size, 5);
    assert_eq!(group_info2.file_number, 10);
    assert_eq!(group_info2.idx, "3");

    Ok(())
}

#[tokio::test]
async fn test_group_build() -> Result<(), Error> {
    let (store, contracts) = setup_test_env()?;

    let config = ToplocConfig {
        server_url: "http://localhost:8080".to_string(),
        ..Default::default()
    };

    let mock_storage = MockStorageProvider::new();
    mock_storage
        .add_file(
            "Qwen3/dataset/samplingn-3450756714426841564-2-9-1.parquet",
            "file1",
        )
        .await;
    mock_storage
        .add_mapping_file(
            "c257e3d3fe866a00df1285f8bbbe601fed6b85229d983bbbb75e19a068346641",
            "Qwen3/dataset/samplingn-3450756714426841564-2-9-1.parquet",
        )
        .await;
    mock_storage
        .add_file(
            "Qwen3/dataset/samplingn-3450756714426841564-2-9-0.parquet",
            "file2",
        )
        .await;
    mock_storage
        .add_mapping_file(
            "88e4672c19e5a10bff2e23d223f8bfc38ae1425feaa18db9480e631a4fd98edf",
            "Qwen3/dataset/samplingn-3450756714426841564-2-9-0.parquet",
        )
        .await;

    let storage_provider = Arc::new(mock_storage);

    let validator = SyntheticDataValidator::new(
        "0".to_string(),
        contracts.synthetic_data_validator.clone().unwrap(),
        contracts.prime_network.clone(),
        vec![config],
        U256::from(0),
        storage_provider,
        store,
        CancellationToken::new(),
        10,
        60,
        1,
        10,
        false,
        false,
        0, // incomplete_group_grace_period_minutes (disabled)
        InvalidationType::Hard,
        InvalidationType::Hard,
        None,
    );

    let group = validator
        .get_group("c257e3d3fe866a00df1285f8bbbe601fed6b85229d983bbbb75e19a068346641")
        .await?;
    assert!(group.is_none());

    let group = validator
        .get_group("88e4672c19e5a10bff2e23d223f8bfc38ae1425feaa18db9480e631a4fd98edf")
        .await?;
    assert!(group.is_some());
    let group = group.unwrap();
    assert_eq!(&group.sorted_work_keys.len(), &2);
    assert_eq!(
        &group.sorted_work_keys[0],
        "88e4672c19e5a10bff2e23d223f8bfc38ae1425feaa18db9480e631a4fd98edf"
    );
    Ok(())
}

#[tokio::test]
async fn test_group_e2e_accept() -> Result<(), Error> {
    let mut server = Server::new_async().await;
    let (store, contracts) = setup_test_env()?;

    let config = ToplocConfig {
        server_url: server.url(),
        file_prefix_filter: Some("Qwen/Qwen0.6".to_string()),
        ..Default::default()
    };

    const FILE_SHA: &str = "c257e3d3fe866a00df1285f8bbbe601fed6b85229d983bbbb75e19a068346641";
    const GROUP_ID: &str = "3450756714426841564";
    const NODE_ADDRESS: &str = "0xA1DDe6E4d2F127960e7C61f90a8b354Bc306bd2a";

    let mock_storage = MockStorageProvider::new();
    mock_storage
        .add_file(
            &format!("Qwen/Qwen0.6/dataset/samplingn-{}-1-0-0.parquet", GROUP_ID),
            "file1",
        )
        .await;
    mock_storage
        .add_mapping_file(
            FILE_SHA,
            &format!("Qwen/Qwen0.6/dataset/samplingn-{}-1-0-0.parquet", GROUP_ID),
        )
        .await;
    server
        .mock(
            "POST",
            format!("/validategroup/dataset/samplingn-{}-1-0.parquet", GROUP_ID).as_str(),
        )
        .match_body(mockito::Matcher::Json(serde_json::json!({
            "file_shas": [FILE_SHA],
            "group_id": GROUP_ID,
            "file_number": 0,
            "group_size": 1
        })))
        .with_status(200)
        .with_body(r#"ok"#)
        .create();
    server
        .mock(
            "GET",
            format!("/statusgroup/dataset/samplingn-{}-1-0.parquet", GROUP_ID).as_str(),
        )
        .with_status(200)
        .with_body(r#"{"status": "accept", "input_flops": 1, "output_flops": 1000}"#)
        .create();

    let storage_provider = Arc::new(mock_storage);
    let metrics_context = MetricsContext::new("0".to_string(), Some("0".to_string()));

    let validator = SyntheticDataValidator::new(
        "0".to_string(),
        contracts.synthetic_data_validator.clone().unwrap(),
        contracts.prime_network.clone(),
        vec![config],
        U256::from(0),
        storage_provider,
        store,
        CancellationToken::new(),
        10,
        60,
        1,
        10,
        true,
        false,
        0, // incomplete_group_grace_period_minutes (disabled)
        InvalidationType::Hard,
        InvalidationType::Hard,
        Some(metrics_context),
    );

    let work_keys: Vec<String> = vec![FILE_SHA.to_string()];

    let work_info = WorkInfo {
        node_id: Address::from_str(NODE_ADDRESS).unwrap(),
        work_units: U256::from(1000),
        ..Default::default()
    };
    for work_key in work_keys.clone() {
        validator
            .update_work_info_in_redis(&work_key, &work_info)
            .await?;
    }

    let plan = validator.build_validation_plan(work_keys.clone()).await?;
    assert_eq!(plan.group_trigger_tasks.len(), 1);
    assert_eq!(plan.group_trigger_tasks[0].group_id, GROUP_ID);
    let metrics_0 = export_metrics().unwrap();
    assert!(
        metrics_0.contains("validator_work_keys_to_process{pool_id=\"0\",validator_id=\"0\"} 1")
    );

    let group = validator.get_group(FILE_SHA).await?;
    assert!(group.is_some());
    let group = group.unwrap();
    assert_eq!(group.group_id, GROUP_ID);
    assert_eq!(group.group_size, 1);
    assert_eq!(group.file_number, 0);

    let result = validator.process_group_task(group).await;
    assert!(result.is_ok());

    let cache_status = validator
        .get_work_validation_status_from_redis(FILE_SHA)
        .await?;
    assert_eq!(cache_status, Some(ValidationResult::Unknown));

    let plan_2 = validator.build_validation_plan(work_keys.clone()).await?;
    assert_eq!(plan_2.group_trigger_tasks.len(), 0);
    assert_eq!(plan_2.group_status_check_tasks.len(), 1);

    let metrics = export_metrics().unwrap();
    assert!(metrics.contains("validator_work_keys_to_process{pool_id=\"0\",validator_id=\"0\"} 1"));

    let result = validator
        .process_group_status_check(plan_2.group_status_check_tasks[0].clone())
        .await;
    assert!(result.is_ok());

    let cache_status = validator
        .get_work_validation_status_from_redis(FILE_SHA)
        .await?;
    assert_eq!(cache_status, Some(ValidationResult::Accept));

    let plan_3 = validator.build_validation_plan(work_keys.clone()).await?;
    assert_eq!(plan_3.group_trigger_tasks.len(), 0);
    assert_eq!(plan_3.group_status_check_tasks.len(), 0);
    let metrics_2 = export_metrics().unwrap();
    assert!(
        metrics_2.contains("validator_work_keys_to_process{pool_id=\"0\",validator_id=\"0\"} 0")
    );
    assert!(metrics_2.contains("toploc_config_name=\"Qwen/Qwen0.6\""));
    assert!(metrics_2.contains(&format!("validator_group_work_units_check_total{{group_id=\"{}\",pool_id=\"0\",result=\"match\",toploc_config_name=\"Qwen/Qwen0.6\",validator_id=\"0\"}} 1", GROUP_ID)));

    Ok(())
}

#[tokio::test]
async fn test_group_e2e_work_unit_mismatch() -> Result<(), Error> {
    let mut server = Server::new_async().await;
    let (store, contracts) = setup_test_env()?;

    let config = ToplocConfig {
        server_url: server.url(),
        file_prefix_filter: Some("Qwen/Qwen0.6".to_string()),
        ..Default::default()
    };

    const HONEST_NODE_ADDRESS: &str = "0x0000000000000000000000000000000000000001";
    const HONEST_FILE_SHA: &str =
        "c257e3d3fe866a00df1285f8bbbe601fed6b85229d983bbbb75e19a068346641";
    const EXCESSIVE_FILE_SHA: &str =
        "88e4672c19e5a10bff2e23d223f8bfc38ae1425feaa18db9480e631a4fd98edf";
    const EXCESSIVE_NODE_ADDRESS: &str = "0x0000000000000000000000000000000000000002";
    const GROUP_ID: &str = "3456714426841564";

    let mock_storage = MockStorageProvider::new();
    mock_storage
        .add_file(
            &format!("Qwen/Qwen0.6/dataset/samplingn-{}-2-0-0.parquet", GROUP_ID),
            "file1",
        )
        .await;
    mock_storage
        .add_file(
            &format!("Qwen/Qwen0.6/dataset/samplingn-{}-2-0-1.parquet", GROUP_ID),
            "file2",
        )
        .await;
    mock_storage
        .add_mapping_file(
            HONEST_FILE_SHA,
            &format!("Qwen/Qwen0.6/dataset/samplingn-{}-2-0-0.parquet", GROUP_ID),
        )
        .await;
    mock_storage
        .add_mapping_file(
            EXCESSIVE_FILE_SHA,
            &format!("Qwen/Qwen0.6/dataset/samplingn-{}-2-0-1.parquet", GROUP_ID),
        )
        .await;
    server
        .mock(
            "POST",
            format!("/validategroup/dataset/samplingn-{}-2-0.parquet", GROUP_ID).as_str(),
        )
        .match_body(mockito::Matcher::Json(serde_json::json!({
            "file_shas": [HONEST_FILE_SHA, EXCESSIVE_FILE_SHA],
            "group_id": GROUP_ID,
            "file_number": 0,
            "group_size": 2
        })))
        .with_status(200)
        .with_body(r#"ok"#)
        .create();
    server
        .mock(
            "GET",
            format!("/statusgroup/dataset/samplingn-{}-2-0.parquet", GROUP_ID).as_str(),
        )
        .with_status(200)
        .with_body(r#"{"status": "accept", "input_flops": 1, "output_flops": 2000}"#)
        .create();

    let storage_provider = Arc::new(mock_storage);
    let metrics_context = MetricsContext::new("0".to_string(), Some("0".to_string()));

    let validator = SyntheticDataValidator::new(
        "0".to_string(),
        contracts.synthetic_data_validator.clone().unwrap(),
        contracts.prime_network.clone(),
        vec![config],
        U256::from(0),
        storage_provider,
        store,
        CancellationToken::new(),
        10,
        60,
        1,
        10,
        true,
        false,
        0, // incomplete_group_grace_period_minutes (disabled)
        InvalidationType::Hard,
        InvalidationType::Hard,
        Some(metrics_context),
    );

    let work_keys: Vec<String> = vec![HONEST_FILE_SHA.to_string(), EXCESSIVE_FILE_SHA.to_string()];

    const EXPECTED_WORK_UNITS: u64 = 1000;
    const EXCESSIVE_WORK_UNITS: u64 = 1500;
    let work_info_1 = WorkInfo {
        node_id: Address::from_str(HONEST_NODE_ADDRESS).unwrap(),
        work_units: U256::from(EXPECTED_WORK_UNITS),
        ..Default::default()
    };
    let work_info_2 = WorkInfo {
        node_id: Address::from_str(EXCESSIVE_NODE_ADDRESS).unwrap(),
        work_units: U256::from(EXCESSIVE_WORK_UNITS),
        ..Default::default()
    };

    validator
        .update_work_info_in_redis(HONEST_FILE_SHA, &work_info_1)
        .await?;
    validator
        .update_work_info_in_redis(EXCESSIVE_FILE_SHA, &work_info_2)
        .await?;

    let plan = validator.build_validation_plan(work_keys.clone()).await?;
    assert_eq!(plan.group_trigger_tasks.len(), 1);
    assert_eq!(plan.group_trigger_tasks[0].group_id, GROUP_ID);
    let metrics_0 = export_metrics().unwrap();
    assert!(
        metrics_0.contains("validator_work_keys_to_process{pool_id=\"0\",validator_id=\"0\"} 2")
    );

    let group = validator.get_group(HONEST_FILE_SHA).await?;
    assert!(group.is_some());
    let group = group.unwrap();
    assert_eq!(group.group_id, GROUP_ID);
    assert_eq!(group.group_size, 2);
    assert_eq!(group.file_number, 0);

    let result = validator.process_group_task(group).await;
    assert!(result.is_ok());

    let cache_status_1 = validator
        .get_work_validation_status_from_redis(HONEST_FILE_SHA)
        .await?;
    assert_eq!(cache_status_1, Some(ValidationResult::Unknown));
    let cache_status_2 = validator
        .get_work_validation_status_from_redis(EXCESSIVE_FILE_SHA)
        .await?;
    assert_eq!(cache_status_2, Some(ValidationResult::Unknown));

    let plan_2 = validator.build_validation_plan(work_keys.clone()).await?;
    assert_eq!(plan_2.group_trigger_tasks.len(), 0);
    assert_eq!(plan_2.group_status_check_tasks.len(), 1);

    let metrics = export_metrics().unwrap();
    assert!(metrics.contains("validator_work_keys_to_process{pool_id=\"0\",validator_id=\"0\"} 2"));

    let result = validator
        .process_group_status_check(plan_2.group_status_check_tasks[0].clone())
        .await;
    assert!(result.is_ok());

    let cache_status_1 = validator
        .get_work_validation_status_from_redis(HONEST_FILE_SHA)
        .await?;
    assert_eq!(cache_status_1, Some(ValidationResult::Accept));

    let cache_status_2 = validator
        .get_work_validation_status_from_redis(EXCESSIVE_FILE_SHA)
        .await?;
    assert_eq!(cache_status_2, Some(ValidationResult::Reject));

    let plan_3 = validator.build_validation_plan(work_keys.clone()).await?;
    assert_eq!(plan_3.group_trigger_tasks.len(), 0);
    assert_eq!(plan_3.group_status_check_tasks.len(), 0);
    let metrics_2 = export_metrics().unwrap();
    assert!(metrics_2.contains(&format!("validator_group_validations_total{{group_id=\"{}\",pool_id=\"0\",result=\"accept\",toploc_config_name=\"Qwen/Qwen0.6\",validator_id=\"0\"}} 1", GROUP_ID)));
    assert!(
        metrics_2.contains("validator_work_keys_to_process{pool_id=\"0\",validator_id=\"0\"} 0")
    );
    assert!(metrics_2.contains("toploc_config_name=\"Qwen/Qwen0.6\""));
    assert!(metrics_2.contains(&format!("validator_group_work_units_check_total{{group_id=\"{}\",pool_id=\"0\",result=\"mismatch\",toploc_config_name=\"Qwen/Qwen0.6\",validator_id=\"0\"}} 1", GROUP_ID)));

    Ok(())
}

#[tokio::test]
async fn test_process_group_status_check_reject() -> Result<(), Error> {
    let mut server = Server::new_async().await;

    let _status_mock = server
        .mock(
            "GET",
            "/statusgroup/dataset/samplingn-3450756714426841564-1-9.parquet",
        )
        .with_status(200)
        .with_body(r#"{"status": "reject", "flops": 0.0, "failing_indices": [0]}"#)
        .create();

    let (store, contracts) = setup_test_env()?;

    let config = ToplocConfig {
        server_url: server.url(),
        file_prefix_filter: Some("Qwen3".to_string()),
        ..Default::default()
    };

    let mock_storage = MockStorageProvider::new();
    mock_storage
        .add_file(
            "Qwen3/dataset/samplingn-3450756714426841564-1-9-0.parquet",
            "file1",
        )
        .await;
    mock_storage
        .add_mapping_file(
            "c257e3d3fe866a00df1285f8bbbe601fed6b85229d983bbbb75e19a068346641",
            "Qwen3/dataset/samplingn-3450756714426841564-1-9-0.parquet",
        )
        .await;

    let storage_provider = Arc::new(mock_storage);

    let validator = SyntheticDataValidator::new(
        "0".to_string(),
        contracts.synthetic_data_validator.clone().unwrap(),
        contracts.prime_network.clone(),
        vec![config],
        U256::from(0),
        storage_provider,
        store,
        CancellationToken::new(),
        10,
        60,
        1,
        10,
        true,
        true,
        0, // incomplete_group_grace_period_minutes (disabled)
        InvalidationType::Hard,
        InvalidationType::Hard,
        None,
    );

    let group = validator
        .get_group("c257e3d3fe866a00df1285f8bbbe601fed6b85229d983bbbb75e19a068346641")
        .await?;
    let group = group.unwrap();

    let result = validator.process_group_status_check(group).await;
    assert!(result.is_ok());

    let work_info = validator
        .get_work_info_from_redis(
            "c257e3d3fe866a00df1285f8bbbe601fed6b85229d983bbbb75e19a068346641",
        )
        .await?;
    assert!(work_info.is_none(), "Work should be invalidated");

    Ok(())
}

#[tokio::test]
async fn test_incomplete_group_recovery() -> Result<(), Error> {
    let (store, contracts) = setup_test_env()?;
    let mock_storage = MockStorageProvider::new();

    // Create an incomplete group with only 1 of 2 expected files
    const GROUP_ID: &str = "1234567890123456";
    const FILE_SHA_1: &str = "a1b2c3d4e5f6789012345678901234567890123456789012345678901234abcd";
    const FILE_SHA_2: &str = "b2c3d4e5f6789012345678901234567890123456789012345678901234bcde";

    mock_storage
        .add_file(
            &format!("TestModel/dataset/test-{}-2-0-0.parquet", GROUP_ID),
            "file1",
        )
        .await;
    mock_storage
        .add_file(
            &format!("TestModel/dataset/test-{}-2-0-1.parquet", GROUP_ID),
            "file2",
        )
        .await;
    mock_storage
        .add_mapping_file(
            FILE_SHA_1,
            &format!("TestModel/dataset/test-{}-2-0-0.parquet", GROUP_ID),
        )
        .await;
    mock_storage
        .add_mapping_file(
            FILE_SHA_2,
            &format!("TestModel/dataset/test-{}-2-0-1.parquet", GROUP_ID),
        )
        .await;

    let storage_provider = Arc::new(mock_storage);

    // Create validator with 1 minute grace period
    let validator = SyntheticDataValidator::new(
        "0".to_string(),
        contracts.synthetic_data_validator.clone().unwrap(),
        contracts.prime_network.clone(),
        vec![ToplocConfig {
            server_url: "http://localhost:8080".to_string(),
            ..Default::default()
        }],
        U256::from(0),
        storage_provider,
        store,
        CancellationToken::new(),
        10,
        60,
        1,
        10,
        true,
        true, // disable_chain_invalidation for testing
        1,    // 1 minute grace period
        InvalidationType::Hard,
        InvalidationType::Hard,
        None,
    );

    // Add work info for only the first file (making the group incomplete)
    let work_info = WorkInfo {
        node_id: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
        work_units: U256::from(1000),
        ..Default::default()
    };
    validator
        .update_work_info_in_redis(FILE_SHA_1, &work_info)
        .await?;

    // Try to get the group - should be None (incomplete) and should start tracking
    let group = validator.get_group(FILE_SHA_1).await?;
    assert!(group.is_none(), "Group should be incomplete");

    // Check that the incomplete group is being tracked
    let group_key = format!("group:{}:2:0", GROUP_ID);
    let is_tracked = validator
        .is_group_being_tracked_as_incomplete(&group_key)
        .await?;
    assert!(is_tracked, "Group should be tracked as incomplete");

    // Simulate the group becoming complete by adding the second file
    validator
        .update_work_info_in_redis(FILE_SHA_2, &work_info)
        .await?;

    // Now the group should be complete (check from either file)
    let group = validator.get_group(FILE_SHA_2).await?;
    assert!(group.is_some(), "Group should now be complete");
    let group = group.unwrap();
    assert_eq!(group.sorted_work_keys.len(), 2);

    // Should also work when checking from the first file
    let group_from_first = validator.get_group(FILE_SHA_1).await?;
    assert!(
        group_from_first.is_some(),
        "Group should also be complete when checked from first file"
    );

    // The incomplete group tracking should be removed
    let is_still_tracked = validator
        .is_group_being_tracked_as_incomplete(&group_key)
        .await?;
    assert!(
        !is_still_tracked,
        "Group should no longer be tracked as incomplete"
    );

    Ok(())
}

#[tokio::test]
async fn test_expired_incomplete_group_soft_invalidation() -> Result<(), Error> {
    let (store, contracts) = setup_test_env()?;
    let mock_storage = MockStorageProvider::new();

    // Create an incomplete group
    const GROUP_ID: &str = "9876543210987654";
    const FILE_SHA_1: &str = "c1d2e3f4567890123456789012345678901234567890123456789012345cdef";

    mock_storage
        .add_file(
            &format!("TestModel/dataset/test-{}-2-0-0.parquet", GROUP_ID),
            "file1",
        )
        .await;
    mock_storage
        .add_mapping_file(
            FILE_SHA_1,
            &format!("TestModel/dataset/test-{}-2-0-0.parquet", GROUP_ID),
        )
        .await;

    let storage_provider = Arc::new(mock_storage);

    // Create validator with very short grace period for testing
    let validator = SyntheticDataValidator::new(
        "0".to_string(),
        contracts.synthetic_data_validator.clone().unwrap(),
        contracts.prime_network.clone(),
        vec![ToplocConfig {
            server_url: "http://localhost:8080".to_string(),
            ..Default::default()
        }],
        U256::from(0),
        storage_provider,
        store,
        CancellationToken::new(),
        10,
        60,
        1,
        10,
        true,
        true, // disable_chain_invalidation for testing
        1,    // 1 minute grace period (but we'll simulate expiry)
        InvalidationType::Hard,
        InvalidationType::Hard,
        Some(MetricsContext::new("0".to_string(), Some("0".to_string()))),
    );

    // Add work info for only the first file (making the group incomplete)
    let work_info = WorkInfo {
        node_id: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
        work_units: U256::from(1000),
        ..Default::default()
    };
    validator
        .update_work_info_in_redis(FILE_SHA_1, &work_info)
        .await?;

    // Try to get the group - should be None (incomplete) and should start tracking
    let group = validator.get_group(FILE_SHA_1).await?;
    assert!(group.is_none(), "Group should be incomplete");

    // Manually expire the incomplete group tracking by removing it and simulating expiry
    // In a real test, you would wait for the actual expiry, but for testing we simulate it
    let group_key = format!("group:{}:2:0", GROUP_ID);
    validator.track_incomplete_group(&group_key).await?;

    // Process groups past grace period (this would normally find groups past deadline)
    // Since we can't easily simulate time passage in tests, we'll test the method exists

    let tracking = validator
        .is_group_being_tracked_as_incomplete(&group_key)
        .await?;
    assert!(tracking, "Group should still be tracked as incomplete");

    // Update the deadline to simulate time passage
    validator
        .update_incomplete_group_deadline_relative(&group_key, -2)
        .await?;

    let result = validator.process_groups_past_grace_period().await;
    assert!(
        result.is_ok(),
        "Should process groups past grace period without error"
    );
    let new_tracking = validator
        .is_group_being_tracked_as_incomplete(&group_key)
        .await?;
    assert!(
        !new_tracking,
        "Group should no longer be tracked as incomplete"
    );
    let key_status = validator
        .get_work_validation_status_from_redis(FILE_SHA_1)
        .await?;
    assert_eq!(key_status, Some(ValidationResult::IncompleteGroup));

    let metrics = export_metrics().unwrap();
    assert!(metrics.contains(&format!("validator_work_keys_soft_invalidated_total{{group_key=\"group:{}:2:0\",pool_id=\"0\",validator_id=\"0\"}} 1", GROUP_ID)));

    Ok(())
}

#[tokio::test]
async fn test_incomplete_group_status_tracking() -> Result<(), Error> {
    let (store, contracts) = setup_test_env()?;
    let mock_storage = MockStorageProvider::new();

    // Create an incomplete group scenario
    const GROUP_ID: &str = "1111111111111111";
    const FILE_SHA_1: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    mock_storage
        .add_file(
            &format!("TestModel/dataset/test-{}-3-0-0.parquet", GROUP_ID),
            "file1",
        )
        .await;
    mock_storage
        .add_mapping_file(
            FILE_SHA_1,
            &format!("TestModel/dataset/test-{}-3-0-0.parquet", GROUP_ID),
        )
        .await;

    let storage_provider = Arc::new(mock_storage);

    let validator = SyntheticDataValidator::new(
        "0".to_string(),
        contracts.synthetic_data_validator.clone().unwrap(),
        contracts.prime_network.clone(),
        vec![ToplocConfig {
            server_url: "http://localhost:8080".to_string(),
            ..Default::default()
        }],
        U256::from(0),
        storage_provider,
        store,
        CancellationToken::new(),
        10,
        60,
        1,
        10,
        true,
        true, // disable_chain_invalidation for testing
        1,    // 1 minute grace period
        InvalidationType::Hard,
        InvalidationType::Hard,
        None,
    );

    // Add work info for only 1 of 3 expected files
    let work_info = WorkInfo {
        node_id: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
        work_units: U256::from(1000),
        ..Default::default()
    };
    validator
        .update_work_info_in_redis(FILE_SHA_1, &work_info)
        .await?;

    // Trigger incomplete group tracking
    let group = validator.get_group(FILE_SHA_1).await?;
    assert!(group.is_none(), "Group should be incomplete");

    // Manually process groups past grace period to simulate what would happen
    // after the grace period expires (we simulate this since we can't wait in tests)
    let group_key = format!("group:{}:3:0", GROUP_ID);

    // Manually add the group to tracking and then process it
    validator.track_incomplete_group(&group_key).await?;

    // Get work keys and manually soft invalidate to simulate expired processing
    let work_keys = validator.get_group_work_keys_from_redis(&group_key).await?;
    assert_eq!(work_keys.len(), 1);

    // Soft invalidate and set status to IncompleteGroup
    validator.soft_invalidate_work(FILE_SHA_1).await?;
    validator
        .update_work_validation_status(FILE_SHA_1, &ValidationResult::IncompleteGroup)
        .await?;

    // Verify the status is set correctly
    let status = validator
        .get_work_validation_status_from_redis(FILE_SHA_1)
        .await?;
    assert_eq!(
        status,
        Some(ValidationResult::IncompleteGroup),
        "Work should be marked as IncompleteGroup"
    );

    // Verify that IncompleteGroup status is treated as "already processed"
    // by checking it's not included in validation plans
    let validation_plan = validator
        .build_validation_plan(vec![FILE_SHA_1.to_string()])
        .await?;
    assert_eq!(validation_plan.single_trigger_tasks.len(), 0);
    assert_eq!(validation_plan.group_trigger_tasks.len(), 0);
    assert_eq!(validation_plan.status_check_tasks.len(), 0);
    assert_eq!(validation_plan.group_status_check_tasks.len(), 0);

    Ok(())
}
#[tokio::test]
async fn test_filename_resolution_retries_and_soft_invalidation() -> Result<(), Error> {
    let (store, contracts) = setup_test_env()?;
    let mock_storage = MockStorageProvider::new();
    let storage_provider = Arc::new(mock_storage);

    const WORK_KEY: &str = "1234567890123456789012345678901234567890123456789012345678901234";

    let validator = SyntheticDataValidator::new(
        "0".to_string(),
        contracts.synthetic_data_validator.clone().unwrap(),
        contracts.prime_network.clone(),
        vec![ToplocConfig {
            server_url: "http://localhost:8080".to_string(),
            ..Default::default()
        }],
        U256::from(0),
        storage_provider,
        store,
        CancellationToken::new(),
        10,
        60,
        1,
        10,
        true,
        true, // disable_chain_invalidation for testing
        1,    // 1 minute grace period
        InvalidationType::Hard,
        InvalidationType::Hard,
        None,
    );

    // Try 59 times to reach just before max attempts (60)
    for _ in 0..59 {
        let result = validator.get_file_name_for_work_key(WORK_KEY).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ProcessWorkKeyError::FileNameResolutionError(_)
        ));
    }

    // 60th attempt should trigger soft invalidation and return MaxAttemptsReached
    let result = validator.get_file_name_for_work_key(WORK_KEY).await;
    assert!(matches!(
        result.unwrap_err(),
        ProcessWorkKeyError::MaxAttemptsReached(_)
    ));

    // Verify work was marked as invalidated
    let status = validator
        .get_work_validation_status_from_redis(WORK_KEY)
        .await?;
    assert!(status.is_some());

    Ok(())
}

#[tokio::test]
async fn test_get_all_rejections() -> Result<(), Error> {
    let (store, contracts) = setup_test_env()?;
    let mock_storage = MockStorageProvider::new();
    let storage_provider = Arc::new(mock_storage);
    let validator = SyntheticDataValidator::new(
        "0".to_string(),
        contracts.synthetic_data_validator.unwrap(),
        contracts.prime_network,
        vec![],
        U256::from(0),
        storage_provider,
        store,
        CancellationToken::new(),
        10,
        60,
        1,
        10,
        true,
        true, // disable_chain_invalidation for testing
        1,    // 1 minute grace period
        InvalidationType::Hard,
        InvalidationType::Hard,
        None,
    );

    // Add some test rejection data
    validator
        .update_work_validation_info(
            "test_work_key_1",
            &WorkValidationInfo {
                status: ValidationResult::Reject,
                reason: Some("Validation failed due to timeout".to_string()),
            },
        )
        .await?;

    validator
        .update_work_validation_info(
            "test_work_key_2",
            &WorkValidationInfo {
                status: ValidationResult::Accept,
                reason: None,
            },
        )
        .await?;

    validator
        .update_work_validation_info(
            "test_work_key_3",
            &WorkValidationInfo {
                status: ValidationResult::Reject,
                reason: Some("Output mismatch detected".to_string()),
            },
        )
        .await?;

    // Get all rejections
    let rejections = validator.get_all_rejections().await?;

    // Should only return the 2 rejected items
    assert_eq!(rejections.len(), 2);

    let rejection_keys: Vec<&str> = rejections.iter().map(|r| r.work_key.as_str()).collect();
    assert!(rejection_keys.contains(&"test_work_key_1"));
    assert!(rejection_keys.contains(&"test_work_key_3"));

    // Check reasons are preserved
    for rejection in &rejections {
        if rejection.work_key == "test_work_key_1" {
            assert_eq!(
                rejection.reason,
                Some("Validation failed due to timeout".to_string())
            );
            assert!(rejection.timestamp.is_some());
        } else if rejection.work_key == "test_work_key_3" {
            assert_eq!(
                rejection.reason,
                Some("Output mismatch detected".to_string())
            );
            assert!(rejection.timestamp.is_some());
        }
    }

    Ok(())
}
