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
use std::sync::Arc;
#[cfg(test)]
use url::Url;

#[cfg(test)]
pub async fn create_test_app_state() -> Data<AppState> {
    use crate::{scheduler::Scheduler, utils::loop_heartbeats::LoopHeartbeats, ServerMode};

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
    let s3_credentials = std::env::var("S3_CREDENTIALS").ok();
    let bucket_name = std::env::var("BUCKET_NAME").ok();

    Data::new(AppState {
        store_context: store_context.clone(),
        contracts: None,
        pool_id: 1,
        wallet: Arc::new(
            Wallet::new(
                "0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97",
                Url::parse("http://localhost:8545").unwrap(),
            )
            .unwrap(),
        ),
        s3_credentials,
        bucket_name,
        heartbeats: Arc::new(LoopHeartbeats::new(&mode)),
        hourly_upload_limit: 12,
        redis_store: store.clone(),
        scheduler,
        node_groups_plugin: None,
    })
}

#[cfg(test)]
pub fn setup_contract() -> Contracts {
    let coordinator_key = "0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97";
    let rpc_url: Url = Url::parse("http://localhost:8545").unwrap();

    let coordinator_wallet = Arc::new(Wallet::new(coordinator_key, rpc_url).unwrap());

    ContractBuilder::new(&coordinator_wallet.clone())
        .with_compute_registry()
        .with_ai_token()
        .with_prime_network()
        .with_compute_pool()
        .build()
        .unwrap()
}
