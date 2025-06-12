#[cfg(test)]
use crate::api::server::AppState;
#[cfg(test)]
use crate::store::core::RedisStore;
#[cfg(test)]
use crate::store::core::StoreContext;
#[cfg(test)]
use actix_web::web::Data;
#[cfg(test)]
use shared::web3::contracts::core::builder::{ContractBuilder, Contracts};
#[cfg(test)]
use shared::web3::wallet::Wallet;
#[cfg(test)]
use shared::web3::wallet::WalletProvider;
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use url::Url;

#[cfg(test)]
pub async fn create_test_app_state() -> Data<AppState> {
    use shared::utils::MockStorageProvider;

    use crate::{
        metrics::MetricsContext, p2p::client::P2PClient, scheduler::Scheduler,
        utils::loop_heartbeats::LoopHeartbeats, ServerMode,
    };

    let store = Arc::new(RedisStore::new_test());
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

    let store_context = Arc::new(StoreContext::new(store.clone()));
    let mode = ServerMode::Full;
    let scheduler = Scheduler::new(store_context.clone(), vec![]);

    let mock_storage = MockStorageProvider::new();
    let storage_provider = Arc::new(mock_storage);
    let metrics = Arc::new(MetricsContext::new(1.to_string()));
    let p2p_client = Arc::new(P2PClient::new().await.unwrap());

    Data::new(AppState {
        store_context: store_context.clone(),
        contracts: None,
        pool_id: 1,
        wallet: Wallet::new(
            "0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97",
            Url::parse("http://localhost:8545").unwrap(),
        )
        .unwrap(),

        storage_provider,
        heartbeats: Arc::new(LoopHeartbeats::new(&mode)),
        hourly_upload_limit: 12,
        redis_store: store.clone(),
        scheduler,
        node_groups_plugin: None,
        metrics,
        http_client: reqwest::Client::new(),
        p2p_client: p2p_client.clone(),
    })
}

#[cfg(test)]
pub async fn create_test_app_state_with_nodegroups() -> Data<AppState> {
    use shared::utils::MockStorageProvider;

    use crate::{
        metrics::MetricsContext,
        p2p::client::P2PClient,
        plugins::node_groups::{NodeGroupConfiguration, NodeGroupsPlugin},
        scheduler::Scheduler,
        utils::loop_heartbeats::LoopHeartbeats,
        ServerMode,
    };

    let store = Arc::new(RedisStore::new_test());
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

    let store_context = Arc::new(StoreContext::new(store.clone()));
    let mode = ServerMode::Full;
    let scheduler = Scheduler::new(store_context.clone(), vec![]);

    let config = NodeGroupConfiguration {
        name: "test-config".to_string(),
        min_group_size: 1,
        max_group_size: 1,
        compute_requirements: None,
    };

    let node_groups_plugin = Some(Arc::new(NodeGroupsPlugin::new(
        vec![config],
        store.clone(),
        store_context.clone(),
        None,
        None,
    )));

    let mock_storage = MockStorageProvider::new();
    let storage_provider = Arc::new(mock_storage);
    let metrics = Arc::new(MetricsContext::new(1.to_string()));
    let p2p_client = Arc::new(P2PClient::new().await.unwrap());

    Data::new(AppState {
        store_context: store_context.clone(),
        contracts: None,
        pool_id: 1,
        wallet: Wallet::new(
            "0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97",
            Url::parse("http://localhost:8545").unwrap(),
        )
        .unwrap(),

        storage_provider,
        heartbeats: Arc::new(LoopHeartbeats::new(&mode)),
        hourly_upload_limit: 12,
        redis_store: store.clone(),
        scheduler,
        node_groups_plugin,
        metrics,
        http_client: reqwest::Client::new(),
        p2p_client: p2p_client.clone(),
    })
}

#[cfg(test)]
pub fn setup_contract() -> Contracts<WalletProvider> {
    let coordinator_key = "0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97";
    let rpc_url: Url = Url::parse("http://localhost:8545").unwrap();
    let wallet = Wallet::new(coordinator_key, rpc_url).unwrap();

    ContractBuilder::new(wallet.provider)
        .with_compute_registry()
        .with_ai_token()
        .with_prime_network()
        .with_compute_pool()
        .build()
        .unwrap()
}

#[cfg(test)]
pub async fn create_test_app_state_with_metrics() -> Data<AppState> {
    use shared::utils::MockStorageProvider;

    use crate::{
        metrics::MetricsContext, p2p::client::P2PClient, scheduler::Scheduler,
        utils::loop_heartbeats::LoopHeartbeats, ServerMode,
    };

    let store = Arc::new(RedisStore::new_test());
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

    let store_context = Arc::new(StoreContext::new(store.clone()));
    let mode = ServerMode::Full;
    let scheduler = Scheduler::new(store_context.clone(), vec![]);

    let mock_storage = MockStorageProvider::new();
    let storage_provider = Arc::new(mock_storage);
    let metrics = Arc::new(MetricsContext::new("0".to_string()));
    let p2p_client = Arc::new(P2PClient::new().await.unwrap());

    Data::new(AppState {
        store_context: store_context.clone(),
        contracts: None,
        pool_id: 1,
        wallet: Wallet::new(
            "0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97",
            Url::parse("http://localhost:8545").unwrap(),
        )
        .unwrap(),
        storage_provider,
        heartbeats: Arc::new(LoopHeartbeats::new(&mode)),
        hourly_upload_limit: 12,
        redis_store: store.clone(),
        scheduler,
        node_groups_plugin: None,
        metrics,
        http_client: reqwest::Client::new(),
        p2p_client: p2p_client.clone(),
    })
}
