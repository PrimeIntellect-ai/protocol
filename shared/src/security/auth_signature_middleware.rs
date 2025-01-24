use actix_web::dev::Payload;
use actix_web::dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::error::ErrorBadRequest;
use actix_web::error::PayloadError;
use actix_web::web::Bytes;
use actix_web::web::BytesMut;
use actix_web::HttpMessage;
use actix_web::{Error, Result};
use alloy::primitives::Address;
use alloy::primitives::PrimitiveSignature;
use dashmap::DashSet;
use futures_util::future::LocalBoxFuture;
use futures_util::future::{self};
use futures_util::Stream;
use futures_util::StreamExt;
use std::future::{ready, Ready};
use std::pin::Pin;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;

type SyncAddressValidator = Arc<dyn Fn(&Address) -> bool + Send + Sync>;

#[derive(Clone)]
pub struct ValidatorState {
    allowed_addresses: Arc<DashSet<Address>>,
    external_validator: Option<SyncAddressValidator>,
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
        }
    }

    pub fn with_validator<F>(mut self, validator: F) -> Self
    where
        F: Fn(&Address) -> bool + Send + Sync + 'static,
    {
        self.external_validator = Some(Arc::new(validator));
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
            // Collect the full body
            let mut body = BytesMut::new();
            let mut payload = req.take_payload();
            while let Some(chunk) = payload.next().await {
                body.extend_from_slice(chunk?.as_ref());
            }

            // Parse and sort the payload
            let payload_value: serde_json::Value = match serde_json::from_slice(&body) {
                Ok(val) => val,
                Err(e) => return Err(ErrorBadRequest(e)),
            };
            let mut payload_data = payload_value.clone();
            if let Some(obj) = payload_data.as_object_mut() {
                let sorted_keys: Vec<String> = obj.keys().cloned().collect();
                let sorted_obj: serde_json::Map<String, serde_json::Value> = sorted_keys
                    .into_iter()
                    .map(|key| (key.clone(), obj.remove(&key).unwrap()))
                    .collect();
                *obj = sorted_obj;
            }

            let payload_string = match serde_json::to_string(&payload_data) {
                Ok(s) => s,
                Err(e) => return Err(ErrorBadRequest(e)),
            };

            // Combine path and payload
            let msg = format!("{}{}", path, payload_string);

            // Validate signature
            if let (Some(address), Some(signature)) = (x_address, x_signature) {
                let signature = signature.trim_start_matches("0x");
                let parsed_signature = match PrimitiveSignature::from_str(signature) {
                    Ok(sig) => sig,
                    Err(_) => return Err(ErrorBadRequest("Invalid signature format")),
                };

                let recovered_address = match parsed_signature.recover_address_from_msg(msg) {
                    Ok(addr) => addr,
                    Err(_) => {
                        return Err(ErrorBadRequest("Failed to recover address from message"))
                    }
                };

                let expected_address = match Address::from_str(&address) {
                    Ok(addr) => addr,
                    Err(_) => return Err(ErrorBadRequest("Invalid address format")),
                };

                if recovered_address != expected_address {
                    println!("Recovered address: {:?}", recovered_address);
                    println!("Expected address: {:?}", expected_address);
                    return Err(ErrorBadRequest("Invalid signature"));
                }

                if !validator_state.is_address_allowed(&recovered_address) {
                    println!(
                        "Request with valid signature but not authorized. Allowed addresses: {:?}",
                        validator_state.get_allowed_addresses()
                    );
                    return Err(ErrorBadRequest("Address not authorized"));
                }

                // Reconstruct request with the original body
                let stream =
                    futures_util::stream::once(future::ok::<Bytes, PayloadError>(body.freeze()));
                let boxed_stream: Pin<Box<dyn Stream<Item = Result<Bytes, PayloadError>>>> =
                    Box::pin(stream);
                req.set_payload(Payload::from(boxed_stream));

                service.call(req).await
            } else {
                Err(ErrorBadRequest("Missing signature or address"))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::request_signer::sign_request;
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
            .set_json(serde_json::json!({"test": "data"}))
            .to_request();

        let err = test::try_call_service(&app, req).await;
        match err {
            Err(e) => {
                assert_eq!(e.to_string(), "Missing signature or address");
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
            .set_json(serde_json::json!({"test": "data"}))
            .to_request();

        let err = test::try_call_service(&app, req).await;
        match err {
            Err(e) => {
                assert_eq!(e.to_string(), "Invalid signature format");
                let error_response = e.error_response();
                assert_eq!(error_response.status(), StatusCode::BAD_REQUEST);
            }
            Ok(_) => panic!("Expected an error"),
        }
    }

    #[actix_web::test]
    async fn test_valid_signature() {
        let private_key = "0000000000000000000000000000000000000000000000000000000000000001";
        let address = Address::from_str("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf").unwrap();
        let wallet = Wallet::new(
            private_key,
            Url::parse("http://localhost:8080").unwrap(),
        )
        .unwrap();

        let signature = sign_request("/test", &wallet, Some(&serde_json::json!({"test": "data"})))
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

        println!("Address: {}", wallet.wallet.default_signer().address());
        println!("Signature: {}", signature);
        let req = test::TestRequest::post()
            .uri("/test")
            .insert_header((
                "x-address",
                wallet.wallet.default_signer().address().to_string(),
            ))
            .insert_header(("x-signature", signature))
            .set_json(serde_json::json!({"test": "data"}))
            .to_request();

        let res = test::call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn test_valid_signature_but_not_allowed() {
        let private_key = "0000000000000000000000000000000000000000000000000000000000000001";
        let allowed_address =
            Address::from_str("0xeeFBd3F87405FdADa62de677492a805A8dA1B457").unwrap();
        let wallet = Wallet::new(
            private_key,
            Url::parse("https://mainnet.infura.io/v3/9aa3d95b3bc440fa88ea12eaa4456161").unwrap(),
        )
        .unwrap();

        let signature = sign_request("/test", &wallet, Some(&serde_json::json!({"test": "data"})))
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

        println!("Address: {}", wallet.wallet.default_signer().address());
        println!("Signature: {}", signature);
        let req = test::TestRequest::post()
            .uri("/test")
            .insert_header((
                "x-address",
                wallet.wallet.default_signer().address().to_string(),
            ))
            .insert_header(("x-signature", signature))
            .set_json(serde_json::json!({"test": "data"}))
            .to_request();

        let err = test::try_call_service(&app, req).await;
        match err {
            Err(e) => {
                assert_eq!(e.to_string(), "Address not authorized");
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
        let private_key = "0xf72df6ef6f7ff457e693f6acae8dfc289bd54225875e93d013c4aa27a8feec76";
        let address = Address::from_str("0xc1621E38E76E7355D1f9915a05d0BC29d2B09814").unwrap();
        let wallet = Wallet::new(
            private_key,
            Url::parse("https://mainnet.infura.io/v3/9aa3d95b3bc440fa88ea12eaa4456161").unwrap(),
        )
        .unwrap();

        let validator_state = Arc::new(ValidatorState::new(vec![]));

        let signature = sign_request("/test", &wallet, Some(&serde_json::json!({"test": "data"})))
            .await
            .unwrap();
        let signature_clone = signature.clone();
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
            .insert_header(("x-signature", signature))
            .set_json(serde_json::json!({"test": "data"}))
            .to_request();

        let err = test::try_call_service(&app, req).await;
        match err {
            Err(e) => {
                assert_eq!(e.to_string(), "Address not authorized");
                let error_response = e.error_response();
                assert_eq!(error_response.status(), StatusCode::BAD_REQUEST);
            }
            Ok(_) => panic!("Expected an error"),
        }

        validator_state.add_address(address);
        let allowed_addresses = validator_state.get_allowed_addresses();
        println!("Allowed addresses: {:?}", allowed_addresses);

        let req_after_address_add = test::TestRequest::post()
            .uri("/test")
            .insert_header((
                "x-address",
                wallet.wallet.default_signer().address().to_string(),
            ))
            .insert_header(("x-signature", signature_clone))
            .set_json(serde_json::json!({"test": "data"}))
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
                Address::from_str(&format!("0x{}000000000000000000000000000000000000000", i))
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
}
