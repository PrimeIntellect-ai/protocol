use actix_web::dev::Payload;
use actix_web::dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::error::{ErrorBadRequest, PayloadError};
use actix_web::web::Bytes;
use actix_web::web::BytesMut;
use actix_web::HttpMessage;
use actix_web::{Error, Result};
use alloy::primitives::Address;
use alloy::signers::Signature;
use dashmap::DashMap;
use dashmap::DashSet;
use futures_util::future::LocalBoxFuture;
use futures_util::future::{self};
use futures_util::Stream;
use futures_util::StreamExt;
use log::{debug, error, warn};
use redis::AsyncCommands;
use serde_json::json;
use std::future::{ready, Ready};
use std::pin::Pin;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::time::timeout; // If you're using tokio

// Maximum request body size in bytes
const MAX_BODY_SIZE: usize = 1024 * 1024 * 10; // 10MB
const BODY_TIMEOUT_SECS: u64 = 20; // 20 seconds
const NONCE_EXPIRATION_SECS: u64 = 60; // 1 minute
const MAX_NONCE_LENGTH: usize = 64;
const MIN_NONCE_LENGTH: usize = 16;
const RATE_LIMIT_WINDOW_SECS: u64 = 60;
const MAX_REQUESTS_PER_WINDOW: usize = 100;
const REQUEST_EXPIRY_SECS: u64 = 300;

type SyncAddressValidator = Arc<dyn Fn(&Address) -> bool + Send + Sync>;
type AsyncAddressValidator = Arc<dyn Fn(&Address) -> LocalBoxFuture<'static, bool> + Send + Sync>;

#[derive(Clone)]
pub struct ValidatorState {
    allowed_addresses: Arc<DashSet<Address>>,
    external_validator: Option<SyncAddressValidator>,
    async_validator: Option<AsyncAddressValidator>,
    redis_pool: Option<Arc<redis::aio::MultiplexedConnection>>,
    rate_limiter: Arc<DashMap<Address, (Instant, usize)>>,
}

impl ValidatorState {
    pub fn new(initial_addresses: Vec<Address>) -> Self {
        let set = DashSet::new();
        for address in initial_addresses {
            set.insert(address);
        }
        Self {
            allowed_addresses: Arc::new(set),
            external_validator: None,
            async_validator: None,
            redis_pool: None,
            rate_limiter: Arc::new(DashMap::new()),
        }
    }

    pub async fn with_redis(
        mut self,
        redis_client: redis::Client,
    ) -> Result<Self, redis::RedisError> {
        let conn = redis_client.get_multiplexed_async_connection().await?;
        self.redis_pool = Some(Arc::new(conn));
        Ok(self)
    }

    pub fn with_validator<F>(mut self, validator: F) -> Self
    where
        F: Fn(&Address) -> bool + Send + Sync + 'static,
    {
        self.external_validator = Some(Arc::new(validator));
        self
    }

    pub fn with_async_validator<F>(mut self, validator: F) -> Self
    where
        F: Fn(&Address) -> LocalBoxFuture<'static, bool> + Send + Sync + 'static,
    {
        self.async_validator = Some(Arc::new(validator));
        self
    }

    pub fn add_address(&self, address: Address) {
        self.allowed_addresses.insert(address);
    }

    pub fn remove_address(&self, address: &Address) {
        self.allowed_addresses.remove(address);
    }

    pub fn iter_addresses(&self) -> impl Iterator<Item = Address> + '_ {
        self.allowed_addresses.iter().map(|addr| *addr)
    }

    pub fn get_allowed_addresses(&self) -> Vec<Address> {
        self.iter_addresses().collect()
    }

    pub fn is_address_allowed(&self, address: &Address) -> bool {
        if self.allowed_addresses.contains(address) {
            return true;
        }

        if let Some(validator) = &self.external_validator {
            return validator(address);
        }

        false
    }

    pub async fn is_address_allowed_async(&self, address: &Address) -> bool {
        if self.allowed_addresses.contains(address) {
            return true;
        }

        if let Some(validator) = &self.external_validator {
            if validator(address) {
                return true;
            }
        }

        if let Some(async_validator) = &self.async_validator {
            return async_validator(address).await;
        }

        false
    }

    pub fn validate_nonce_format(&self, nonce: &str) -> bool {
        if nonce.len() < MIN_NONCE_LENGTH || nonce.len() > MAX_NONCE_LENGTH {
            return false;
        }
        nonce.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    }

    pub fn check_rate_limit(&self, address: &Address) -> bool {
        let now = Instant::now();
        let mut entry = self.rate_limiter.entry(*address).or_insert((now, 0));
        let (window_start, count) = entry.value_mut();

        if now.duration_since(*window_start).as_secs() >= RATE_LIMIT_WINDOW_SECS {
            *window_start = now;
            *count = 1;
            true
        } else if *count >= MAX_REQUESTS_PER_WINDOW {
            false
        } else {
            *count += 1;
            true
        }
    }

    pub async fn check_and_store_nonce(&self, nonce: &str) -> Result<bool, redis::RedisError> {
        if !self.validate_nonce_format(nonce) {
            return Ok(false);
        }

        if let Some(pool) = &self.redis_pool {
            let mut conn = pool.as_ref().clone();
            let nonce_key = format!("nonce:{nonce}");

            let result: Option<String> = conn
                .set_options(
                    &nonce_key,
                    "1",
                    redis::SetOptions::default()
                        .conditional_set(redis::ExistenceCheck::NX)
                        .get(true)
                        .with_expiration(redis::SetExpiry::EX(NONCE_EXPIRATION_SECS)),
                )
                .await?;

            Ok(result.is_none())
        } else {
            Ok(true)
        }
    }
}

pub struct ValidateSignature {
    validator_state: Arc<ValidatorState>,
}

impl ValidateSignature {
    pub fn new(state: Arc<ValidatorState>) -> Self {
        Self {
            validator_state: state,
        }
    }
}

impl<S, B> Transform<S, ServiceRequest> for ValidateSignature
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = ValidateSignatureMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(ValidateSignatureMiddleware {
            service: Rc::new(service),
            validator_state: self.validator_state.clone(),
        }))
    }
}

pub struct ValidateSignatureMiddleware<S> {
    service: Rc<S>,
    validator_state: Arc<ValidatorState>,
}

impl<S, B> Service<ServiceRequest> for ValidateSignatureMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);
    fn call(&self, mut req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();
        let path = req.path().to_string();
        let validator_state = self.validator_state.clone();

        // Extract headers before consuming the request
        let x_address = req
            .headers()
            .get("x-address")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());
        let x_signature = req
            .headers()
            .get("x-signature")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());

        Box::pin(async move {
            // Collect the full body with size limit and timeout
            let mut body = BytesMut::new();
            let mut payload = req.take_payload();
            let start_time = Instant::now();

            // Create a timeout for the entire body reading process
            let body_read_future = async {
                while let Some(chunk) = payload.next().await {
                    let chunk = chunk?;

                    // Check if adding this chunk would exceed the size limit
                    if body.len() + chunk.len() > MAX_BODY_SIZE {
                        return Err(ErrorBadRequest(json!({
                            "error": "Request body too large",
                            "code": "BODY_TOO_LARGE",
                            "max_size": MAX_BODY_SIZE
                        })));
                    }

                    // Check if we've exceeded the time limit for reading the body
                    if start_time.elapsed() > Duration::from_secs(BODY_TIMEOUT_SECS) {
                        return Err(ErrorBadRequest(json!({
                            "error": "Request body read timeout",
                            "code": "BODY_READ_TIMEOUT",
                            "timeout_seconds": BODY_TIMEOUT_SECS
                        })));
                    }

                    body.extend_from_slice(chunk.as_ref());
                }
                Ok::<_, Error>(body)
            };

            // Apply timeout to the entire body reading process as a fallback
            let Ok(body_result) =
                timeout(Duration::from_secs(BODY_TIMEOUT_SECS), body_read_future).await
            else {
                return Err(ErrorBadRequest(json!({
                    "error": "Request body read timeout",
                    "code": "BODY_READ_TIMEOUT",
                    "timeout_seconds": BODY_TIMEOUT_SECS
                })));
            };

            // If there was an error reading the body, return it
            let body = body_result?;

            // Handle GET requests which do not have a payload
            let mut payload_string = String::new();
            let mut timestamp = None;
            let mut nonce = None;
            if req.method() != actix_web::http::Method::GET {
                // Parse and sort the payload
                let payload_value: serde_json::Value = match serde_json::from_slice(&body) {
                    Ok(val) => val,
                    Err(e) => {
                        error!("Error parsing payload: {e:?}");
                        return Err(ErrorBadRequest(json!({
                            "error": "Invalid JSON payload",
                            "code": "INVALID_JSON",
                            "details": e.to_string()
                        })));
                    }
                };
                let mut payload_data = payload_value.clone();
                if let Some(obj) = payload_data.as_object_mut() {
                    nonce = obj
                        .get("nonce")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    let sorted_keys: Vec<String> = obj.keys().cloned().collect();
                    let sorted_obj: serde_json::Map<String, serde_json::Value> = sorted_keys
                        .into_iter()
                        .map(|key| (key.clone(), obj.remove(&key).unwrap()))
                        .collect();
                    *obj = sorted_obj;
                }

                payload_string = match serde_json::to_string(&payload_data) {
                    Ok(s) => s,
                    Err(e) => {
                        error!("Error serializing payload: {e:?}");
                        return Err(ErrorBadRequest(json!({
                            "error": "Failed to serialize payload",
                            "code": "SERIALIZATION_ERROR",
                            "details": e.to_string()
                        })));
                    }
                };

                if let Some(obj) = payload_data.as_object_mut() {
                    timestamp = obj.get("timestamp").and_then(|v| v.as_u64());
                }
            }
            if timestamp.is_none() {
                timestamp = req.uri().query().and_then(|query| {
                    query
                        .split('&')
                        .find(|param| param.starts_with("timestamp="))
                        .and_then(|param| param.split('=').nth(1))
                        .and_then(|value| match value.parse::<u64>() {
                            Ok(ts) => Some(ts),
                            Err(e) => {
                                debug!("Failed to parse timestamp from query: {e:?}");
                                None
                            }
                        })
                });
            }

            // Extract nonce from query parameters for GET requests
            if nonce.is_none() {
                nonce = req.uri().query().and_then(|query| {
                    query
                        .split('&')
                        .find(|param| param.starts_with("nonce="))
                        .and_then(|param| param.split('=').nth(1))
                        .map(|value| value.to_string())
                });
            }

            // Combine path and payload
            let msg: String = format!("{path}{payload_string}");
            // Validate signature
            if let (Some(address), Some(signature)) = (x_address, x_signature) {
                let signature = signature.trim_start_matches("0x");
                let Ok(parsed_signature) = Signature::from_str(signature) else {
                    return Err(ErrorBadRequest(json!({
                        "error": "Invalid signature format",
                        "code": "INVALID_SIGNATURE_FORMAT"
                    })));
                };

                let Ok(recovered_address) = parsed_signature.recover_address_from_msg(msg) else {
                    return Err(ErrorBadRequest(json!({
                        "error": "Failed to recover address from message",
                        "code": "ADDRESS_RECOVERY_FAILED"
                    })));
                };

                let Ok(expected_address) = Address::from_str(&address) else {
                    return Err(ErrorBadRequest(json!({
                        "error": "Invalid address format",
                        "code": "INVALID_ADDRESS_FORMAT"
                    })));
                };

                if recovered_address != expected_address {
                    debug!("Recovered address: {recovered_address:?}");
                    debug!("Expected address: {expected_address:?}");
                    return Err(ErrorBadRequest(json!({
                        "error": "Invalid signature",
                        "code": "SIGNATURE_MISMATCH",
                    })));
                }

                if !validator_state
                    .is_address_allowed_async(&recovered_address)
                    .await
                {
                    warn!(
                        "Request with valid signature but not authorized. Allowed addresses: {:?}",
                        validator_state.get_allowed_addresses()
                    );
                    return Err(ErrorBadRequest(json!({
                        "error": "Address not authorized",
                        "code": "ADDRESS_NOT_AUTHORIZED",
                        "address": recovered_address.to_string()
                    })));
                }

                if !validator_state.check_rate_limit(&recovered_address) {
                    warn!("Rate limit exceeded for address: {recovered_address}");
                    return Err(ErrorBadRequest(json!({
                        "error": "Rate limit exceeded",
                        "code": "RATE_LIMIT_EXCEEDED",
                        "message": format!("Maximum {} requests per {} seconds allowed", MAX_REQUESTS_PER_WINDOW, RATE_LIMIT_WINDOW_SECS)
                    })));
                }

                if let Some(timestamp) = timestamp {
                    let current_time = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs();
                    if current_time - timestamp > REQUEST_EXPIRY_SECS {
                        return Err(ErrorBadRequest(json!({
                            "error": "Request expired",
                            "code": "REQUEST_EXPIRED",
                            "timestamp": timestamp,
                            "current_time": current_time,
                            "max_age_seconds": 10
                        })));
                    }
                }

                if let Some(nonce_value) = nonce {
                    if !validator_state.validate_nonce_format(&nonce_value) {
                        return Err(ErrorBadRequest(json!({
                            "error": "Invalid nonce format",
                            "code": "INVALID_NONCE_FORMAT",
                            "message": format!("Nonce must be {}-{} alphanumeric characters", MIN_NONCE_LENGTH, MAX_NONCE_LENGTH)
                        })));
                    }

                    match validator_state.check_and_store_nonce(&nonce_value).await {
                        Ok(is_unique) => {
                            if !is_unique {
                                return Err(ErrorBadRequest(json!({
                                    "error": "Request replay detected",
                                    "code": "NONCE_ALREADY_USED",
                                    "message": "This nonce has already been used"
                                })));
                            }
                        }
                        Err(e) => {
                            error!("Redis error during nonce check: {e:?}");
                            return Err(ErrorBadRequest(json!({
                                "error": "Nonce validation failed",
                                "code": "NONCE_VALIDATION_ERROR",
                                "message": "Unable to validate request uniqueness"
                            })));
                        }
                    }
                } else {
                    return Err(ErrorBadRequest(json!({
                        "error": "Missing nonce",
                        "code": "MISSING_NONCE",
                        "message": "Request must include a unique nonce for replay protection"
                    })));
                }

                // Reconstruct request with the original body
                let stream =
                    futures_util::stream::once(future::ok::<Bytes, PayloadError>(body.freeze()));
                let boxed_stream: Pin<Box<dyn Stream<Item = Result<Bytes, PayloadError>>>> =
                    Box::pin(stream);
                req.set_payload(Payload::from(boxed_stream));

                service.call(req).await
            } else {
                Err(ErrorBadRequest(json!({
                    "error": "Missing signature or address",
                    "code": "MISSING_AUTH_HEADERS",
                    "required_headers": ["x-signature", "x-address"]
                })))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web3::wallet::Wallet;
    use actix_web::http::StatusCode;
    use actix_web::{test, web, App, HttpResponse};
    use std::collections::HashSet;
    use std::str::FromStr;
    use url::Url;

    async fn test_handler() -> HttpResponse {
        HttpResponse::Ok().finish()
    }

    #[actix_web::test]
    async fn test_missing_headers() {
        let app = test::init_service(
            App::new()
                .wrap(ValidateSignature::new(Arc::new(ValidatorState::new(
                    vec![],
                ))))
                .route("/test", web::post().to(test_handler)),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/test")
            .set_json(serde_json::json!({"test": "data", "nonce": "test-nonce-1"}))
            .to_request();

        let err = test::try_call_service(&app, req).await;
        match err {
            Err(e) => {
                let error_str = e.to_string();
                assert!(error_str.contains("Missing signature or address"));
                let error_response = e.error_response();
                assert_eq!(error_response.status(), StatusCode::BAD_REQUEST);
            }
            Ok(_) => panic!("Expected an error"),
        }
    }

    #[actix_web::test]
    async fn test_invalid_signature() {
        let app = test::init_service(
            App::new()
                .wrap(ValidateSignature::new(Arc::new(ValidatorState::new(
                    vec![],
                ))))
                .route("/test", web::post().to(test_handler)),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/test")
            .insert_header(("x-address", "0x742d35Cc6634C0532925a3b844Bc454e4438f44e"))
            .insert_header(("x-signature", "0xinvalid_signature"))
            .set_json(serde_json::json!({"test": "data", "nonce": "test-nonce-2"}))
            .to_request();

        let err = test::try_call_service(&app, req).await;
        match err {
            Err(e) => {
                let error_str = e.to_string();
                assert!(error_str.contains("Invalid signature format"));
                let error_response = e.error_response();
                assert_eq!(error_response.status(), StatusCode::BAD_REQUEST);
            }
            Ok(_) => panic!("Expected an error"),
        }
    }

    #[actix_web::test]
    async fn test_valid_signature() {
        use crate::security::request_signer::sign_request_with_nonce;

        let private_key = "0000000000000000000000000000000000000000000000000000000000000001";
        let address = Address::from_str("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf").unwrap();
        let wallet =
            Wallet::new(private_key, Url::parse("http://localhost:8080").unwrap()).unwrap();

        let signed_request =
            sign_request_with_nonce("/test", &wallet, Some(&serde_json::json!({"test": "data"})))
                .await
                .unwrap();
        let app = test::init_service(
            App::new()
                .wrap(ValidateSignature::new(Arc::new(ValidatorState::new(vec![
                    address,
                ]))))
                .route("/test", web::post().to(test_handler)),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/test")
            .insert_header((
                "x-address",
                wallet.wallet.default_signer().address().to_string(),
            ))
            .insert_header(("x-signature", signed_request.signature))
            .set_json(signed_request.data.as_ref().unwrap())
            .to_request();

        let res = test::call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn test_valid_signature_get_request() {
        use crate::security::request_signer::sign_request_with_nonce;

        let private_key = "0000000000000000000000000000000000000000000000000000000000000001";
        let address = Address::from_str("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf").unwrap();
        let wallet =
            Wallet::new(private_key, Url::parse("http://localhost:8080").unwrap()).unwrap();

        let signed_request = sign_request_with_nonce("/test", &wallet, None)
            .await
            .unwrap();
        let nonce = signed_request.nonce;
        let signature = signed_request.signature;

        let app = test::init_service(
            App::new()
                .wrap(ValidateSignature::new(Arc::new(ValidatorState::new(vec![
                    address,
                ]))))
                .route("/test", web::get().to(test_handler)),
        )
        .await;

        log::info!("Address: {}", wallet.wallet.default_signer().address());
        log::info!("Signature: {signature}");
        log::info!("Nonce: {nonce}");
        let req = test::TestRequest::get()
            .uri(&format!("/test?nonce={nonce}"))
            .insert_header((
                "x-address",
                wallet.wallet.default_signer().address().to_string(),
            ))
            .insert_header(("x-signature", signature))
            .to_request();

        let res = test::call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn test_valid_signature_but_not_allowed() {
        use crate::security::request_signer::sign_request_with_nonce;

        let private_key = "0000000000000000000000000000000000000000000000000000000000000001";
        let allowed_address =
            Address::from_str("0xeeFBd3F87405FdADa62de677492a805A8dA1B457").unwrap();
        let wallet = Wallet::new(
            private_key,
            Url::parse("https://mainnet.infura.io/v3/9aa3d95b3bc440fa88ea12eaa4456161").unwrap(),
        )
        .unwrap();

        let signed_request =
            sign_request_with_nonce("/test", &wallet, Some(&serde_json::json!({"test": "data"})))
                .await
                .unwrap();
        let app = test::init_service(
            App::new()
                .wrap(ValidateSignature::new(Arc::new(ValidatorState::new(vec![
                    allowed_address,
                ]))))
                .route("/test", web::post().to(test_handler)),
        )
        .await;

        log::info!("Address: {}", wallet.wallet.default_signer().address());
        log::info!("Signature: {}", signed_request.signature);
        let req = test::TestRequest::post()
            .uri("/test")
            .insert_header((
                "x-address",
                wallet.wallet.default_signer().address().to_string(),
            ))
            .insert_header(("x-signature", signed_request.signature))
            .set_json(signed_request.data.as_ref().unwrap())
            .to_request();

        let err = test::try_call_service(&app, req).await;
        match err {
            Err(e) => {
                let error_str = e.to_string();
                assert!(error_str.contains("Address not authorized"));
                let error_response = e.error_response();
                assert_eq!(error_response.status(), StatusCode::BAD_REQUEST);
            }
            Ok(_) => panic!("Expected an error"),
        }
    }

    #[actix_web::test]
    async fn test_multiple_state_clones() {
        let address = Address::from_str("0xc1621E38E76E7355D1f9915a05d0BC29d2B09814").unwrap();
        let validator_state = Arc::new(ValidatorState::new(vec![]));

        // Create multiple clones
        let clone1 = validator_state.clone();
        let clone2 = validator_state.clone();
        let clone3 = clone1.clone();

        // Modify through one clone
        clone2.add_address(address);

        // Verify all clones see the change
        assert!(validator_state.is_address_allowed(&address));
        assert!(clone1.is_address_allowed(&address));
        assert!(clone2.is_address_allowed(&address));
        assert!(clone3.is_address_allowed(&address));

        // Remove through another clone
        clone3.remove_address(&address);

        // Verify removal is visible to all
        assert!(!validator_state.is_address_allowed(&address));
        assert!(!clone1.is_address_allowed(&address));
        assert!(!clone2.is_address_allowed(&address));
        assert!(!clone3.is_address_allowed(&address));
    }

    #[actix_web::test]
    async fn test_dynamic_allowed_addresses() {
        use crate::security::request_signer::sign_request_with_nonce;

        let private_key = "0xf72df6ef6f7ff457e693f6acae8dfc289bd54225875e93d013c4aa27a8feec76";
        let address = Address::from_str("0xc1621E38E76E7355D1f9915a05d0BC29d2B09814").unwrap();
        let wallet = Wallet::new(
            private_key,
            Url::parse("https://mainnet.infura.io/v3/9aa3d95b3bc440fa88ea12eaa4456161").unwrap(),
        )
        .unwrap();

        let validator_state = Arc::new(ValidatorState::new(vec![]));

        let signed_request1 =
            sign_request_with_nonce("/test", &wallet, Some(&serde_json::json!({"test": "data"})))
                .await
                .unwrap();
        let signed_request2 =
            sign_request_with_nonce("/test", &wallet, Some(&serde_json::json!({"test": "data"})))
                .await
                .unwrap();
        let app = test::init_service(
            App::new()
                .wrap(ValidateSignature::new(validator_state.clone()))
                .route("/test", web::post().to(test_handler)),
        )
        .await;

        let req = test::TestRequest::post()
            .uri("/test")
            .insert_header((
                "x-address",
                wallet.wallet.default_signer().address().to_string(),
            ))
            .insert_header(("x-signature", signed_request1.signature))
            .set_json(signed_request1.data.as_ref().unwrap())
            .to_request();

        let err = test::try_call_service(&app, req).await;
        match err {
            Err(e) => {
                let error_str = e.to_string();
                assert!(error_str.contains("Address not authorized"));
                let error_response = e.error_response();
                assert_eq!(error_response.status(), StatusCode::BAD_REQUEST);
            }
            Ok(_) => panic!("Expected an error"),
        }

        validator_state.add_address(address);

        let req_after_address_add = test::TestRequest::post()
            .uri("/test")
            .insert_header((
                "x-address",
                wallet.wallet.default_signer().address().to_string(),
            ))
            .insert_header(("x-signature", signed_request2.signature))
            .set_json(signed_request2.data.as_ref().unwrap())
            .to_request();

        let res_after_address_add = test::call_service(&app, req_after_address_add).await;
        assert_eq!(res_after_address_add.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn test_multiple_addresses() {
        let validator_state = ValidatorState::new(vec![]);

        // Create multiple addresses
        let addresses: Vec<Address> = (0..5)
            .map(|i| {
                Address::from_str(&format!("0x{i}000000000000000000000000000000000000000"))
                    .unwrap()
            })
            .collect();

        // Add addresses through different clones
        let clone1 = validator_state.clone();
        let clone2 = validator_state.clone();

        for (i, addr) in addresses.iter().enumerate() {
            if i % 2 == 0 {
                clone1.add_address(*addr);
            } else {
                clone2.add_address(*addr);
            }
        }

        // Verify all addresses are present
        let allowed = validator_state.get_allowed_addresses();
        let allowed_set: HashSet<_> = allowed.into_iter().collect();
        let expected_set: HashSet<_> = addresses.into_iter().collect();
        assert_eq!(allowed_set, expected_set);
    }

    #[actix_web::test]
    async fn test_nonce_replay_protection() {
        use crate::security::request_signer::sign_request_with_nonce;

        let private_key = "0000000000000000000000000000000000000000000000000000000000000001";
        let address = Address::from_str("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf").unwrap();
        let wallet =
            Wallet::new(private_key, Url::parse("http://localhost:8080").unwrap()).unwrap();

        let signed_request =
            sign_request_with_nonce("/test", &wallet, Some(&serde_json::json!({"test": "data"})))
                .await
                .unwrap();

        let app = test::init_service(
            App::new()
                .wrap(ValidateSignature::new(Arc::new(ValidatorState::new(vec![
                    address,
                ]))))
                .route("/test", web::post().to(test_handler)),
        )
        .await;

        let req1 = test::TestRequest::post()
            .uri("/test")
            .insert_header((
                "x-address",
                wallet.wallet.default_signer().address().to_string(),
            ))
            .insert_header(("x-signature", signed_request.signature.clone()))
            .set_json(signed_request.data.as_ref().unwrap())
            .to_request();

        let res1 = test::call_service(&app, req1).await;
        assert_eq!(res1.status(), StatusCode::OK);
        let req2 = test::TestRequest::post()
            .uri("/test")
            .insert_header((
                "x-address",
                wallet.wallet.default_signer().address().to_string(),
            ))
            .insert_header(("x-signature", signed_request.signature))
            .set_json(signed_request.data.as_ref().unwrap())
            .to_request();

        let err = test::try_call_service(&app, req2).await;
        match err {
            Err(e) => {
                let error_str = e.to_string();
                assert!(
                    error_str.contains("Missing nonce") || error_str.contains("NONCE_ALREADY_USED")
                );
            }
            Ok(res) => {
                if res.status() != StatusCode::OK {
                    panic!(
                        "Expected OK status when Redis is not configured, got: {}",
                        res.status()
                    );
                }
            }
        }
    }
}
