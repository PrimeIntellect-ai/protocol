use actix_web::FromRequest;
use std::future::{ready, Ready};
use actix_web::dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform};
use futures_util::future::LocalBoxFuture;
use std::task::{Context, Poll};
use serde_json::json;
use actix_web::{Error, HttpResponse, HttpRequest, Result}; 
use actix_web::error::ErrorBadRequest;
use shared::web3::wallet::Wallet;
use alloy::primitives::PrimitiveSignature;
use std::str::FromStr;
use alloy::primitives::Address;
use futures_util::FutureExt;
use futures_util::StreamExt;
use actix_web::web::BytesMut;
use actix_web::web::Bytes;
pub struct ValidateSignature;

impl<S, B> Transform<S, ServiceRequest> for ValidateSignature
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = ValidateSignatureMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(ValidateSignatureMiddleware { service }))
    }
}

pub struct ValidateSignatureMiddleware<S> {
    service: S,
}

impl<S, B> Service<ServiceRequest> for ValidateSignatureMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let (http_request, payload) = req.into_parts();
        
        // Extract headers
        let x_address = http_request.headers().get("x-address").and_then(|h| h.to_str().ok()).map(|s| s.to_string());
        let x_signature = http_request.headers().get("x-signature").and_then(|h| h.to_str().ok()).map(|s| s.to_string()); 

        // TODO: Handle payload
        let msg = http_request.path().to_string();
        println!("msg: {}", msg);
        
        let fut = self.service.call(ServiceRequest::from_parts(http_request, payload)); 

        Box::pin(async {
            if let (Some(address), Some(signature)) = (x_address, x_signature) {
           
                let signature = signature.trim_start_matches("0x");
                let parsed_signature = match PrimitiveSignature::from_str(signature) {
                    Ok(sig) => sig,
                    Err(_) => {
                        return Err(ErrorBadRequest("Invalid signature format"));
                    }
                };

                let recovered_address = match parsed_signature.recover_address_from_msg(msg) {
                    Ok(addr) => addr,
                    Err(_) => {
                        return Err(ErrorBadRequest("Failed to recover address from message"));
                    }
                };
                
                let expected_address = match Address::from_str(&address) {
                    Ok(addr) => addr,
                    Err(_) => {
                        return Err(ErrorBadRequest("Invalid address format"));
                    }
                };
                
                if recovered_address != expected_address {
                    return Err(ErrorBadRequest("Invalid signature"));
                }
                
                // If we get here, signature is valid
                fut.await
            } else {
                return Err(ErrorBadRequest("Missing signature or address"));
            }
        })
    }
}