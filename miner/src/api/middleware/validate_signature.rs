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
use futures_util::future::LocalBoxFuture;
use futures_util::future::{self};
use futures_util::FutureExt;
use futures_util::Stream;
use futures_util::StreamExt;
use std::future::{ready, Ready};
use std::pin::Pin;
use std::rc::Rc;
use std::str::FromStr;

pub struct ValidateSignature;

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
        }))
    }
}

pub struct ValidateSignatureMiddleware<S> {
    service: Rc<S>,
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
            let payload_value: serde_json::Value =
                serde_json::from_slice(&body).map_err(ErrorBadRequest)?;
            let mut payload_data = payload_value.clone();
            if let Some(obj) = payload_data.as_object_mut() {
                let sorted_keys: Vec<String> = obj.keys().cloned().collect();
                let sorted_obj: serde_json::Map<String, serde_json::Value> = sorted_keys
                    .into_iter()
                    .map(|key| (key.clone(), obj.remove(&key).unwrap()))
                    .collect();
                *obj = sorted_obj;
            }
            let payload_string = serde_json::to_string(&payload_data).map_err(ErrorBadRequest)?;

            // Combine path and payload
            let msg = format!("{}{}", path, payload_string);
            println!("msg to verify: {}", msg);

            // Validate signature
            if let (Some(address), Some(signature)) = (x_address, x_signature) {
                let signature = signature.trim_start_matches("0x");
                let parsed_signature = PrimitiveSignature::from_str(signature)
                    .map_err(|_| ErrorBadRequest("Invalid signature format"))?;

                let recovered_address = parsed_signature
                    .recover_address_from_msg(msg)
                    .map_err(|_| ErrorBadRequest("Failed to recover address from message"))?;

                let expected_address = Address::from_str(&address)
                    .map_err(|_| ErrorBadRequest("Invalid address format"))?;

                if recovered_address != expected_address {
                    println!("Recovered address: {:?}", recovered_address);
                    println!("Expected address: {:?}", expected_address);
                    return Err(ErrorBadRequest("Invalid signature"));
                }
                println!("Signature is valid");

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
