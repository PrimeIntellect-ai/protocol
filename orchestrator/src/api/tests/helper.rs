#[cfg(test)]
use crate::api::server::AppState;
#[cfg(test)]
use crate::store::redis::RedisStore;
#[cfg(test)]
use actix_web::web::Data;
#[cfg(test)]
use std::sync::Arc;

#[cfg(test)]
pub async fn create_test_app_state() -> Data<AppState> {
    let store = RedisStore::new_test();
    let mut con = store
        .client
        .get_connection()
        .expect("Should connect to test Redis instance");

    redis::cmd("PING")
        .query::<String>(&mut con)
        .expect("Redis should be responsive");

    Data::new(AppState {
        store: Arc::new(store),
    })
}
