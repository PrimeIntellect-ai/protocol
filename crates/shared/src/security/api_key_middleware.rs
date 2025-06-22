use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    error::ErrorUnauthorized,
    http::header::AUTHORIZATION,
    Error,
};
use futures_util::future::{ready, LocalBoxFuture, Ready};
use subtle::ConstantTimeEq;

pub struct ApiKeyMiddleware {
    api_key: String,
}

impl ApiKeyMiddleware {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

impl<S, B> Transform<S, ServiceRequest> for ApiKeyMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = ApiKeyMiddlewareService<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(ApiKeyMiddlewareService {
            service,
            api_key: self.api_key.clone(),
        }))
    }
}

pub struct ApiKeyMiddlewareService<S> {
    service: S,
    api_key: String,
}

impl<S, B> Service<ServiceRequest> for ApiKeyMiddlewareService<S>
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
        if let Some(auth_header) = req.headers().get(AUTHORIZATION) {
            if let Ok(auth_str) = auth_header.to_str() {
                if auth_str.len() > 7 {
                    let (scheme, key) = auth_str.split_at(7);
                    if scheme.eq_ignore_ascii_case("Bearer ") {
                        let provided_key_bytes = key.as_bytes();
                        let expected_key_bytes = self.api_key.as_bytes();

                        if provided_key_bytes.len() == expected_key_bytes.len()
                            && provided_key_bytes.ct_eq(expected_key_bytes).into()
                        {
                            let fut = self.service.call(req);
                            return Box::pin(async move {
                                let res = fut.await?;
                                Ok(res)
                            });
                        }
                    }
                }
            }
        }

        Box::pin(async move { Err(ErrorUnauthorized("Invalid API key")) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{test, web, App, HttpResponse};

    async fn test_handler() -> HttpResponse {
        HttpResponse::Ok().body("Success")
    }

    #[actix_web::test]
    async fn test_valid_api_key() {
        let api_key = "test-api-key";
        let app = test::init_service(
            App::new()
                .wrap(ApiKeyMiddleware::new(api_key.to_string()))
                .route("/", web::get().to(test_handler)),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/")
            .insert_header(("Authorization", "Bearer test-api-key"))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
    }

    #[actix_web::test]
    async fn test_invalid_api_key() {
        let api_key = "test-api-key";
        let app = test::init_service(
            App::new()
                .wrap(ApiKeyMiddleware::new(api_key.to_string()))
                .route("/", web::get().to(test_handler)),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/")
            .insert_header(("Authorization", "Bearer wrong-key"))
            .to_request();

        let resp = app.call(req).await;
        assert!(resp.is_err());
        assert_eq!(resp.unwrap_err().to_string(), "Invalid API key");
    }

    #[actix_web::test]
    async fn test_missing_auth_header() {
        let api_key = "test-api-key";
        let app = test::init_service(
            App::new()
                .wrap(ApiKeyMiddleware::new(api_key.to_string()))
                .route("/", web::get().to(test_handler)),
        )
        .await;

        let req = test::TestRequest::get().uri("/").to_request();

        let resp = app.call(req).await;
        assert!(resp.is_err());
        assert_eq!(resp.unwrap_err().to_string(), "Invalid API key");
    }

    #[actix_web::test]
    async fn test_lowercase_bearer_accepted() {
        let api_key = "test-api-key";
        let app = test::init_service(
            App::new()
                .wrap(ApiKeyMiddleware::new(api_key.to_string()))
                .route("/", web::get().to(test_handler)),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/")
            .insert_header(("Authorization", "bearer test-api-key"))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
    }

    #[actix_web::test]
    async fn test_mixed_case_bearer_accepted() {
        let api_key = "test-api-key";
        let app = test::init_service(
            App::new()
                .wrap(ApiKeyMiddleware::new(api_key.to_string()))
                .route("/", web::get().to(test_handler)),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/")
            .insert_header(("Authorization", "BeArEr test-api-key"))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());
    }

    #[actix_web::test]
    async fn test_malformed_auth_header() {
        let api_key = "test-api-key";
        let app = test::init_service(
            App::new()
                .wrap(ApiKeyMiddleware::new(api_key.to_string()))
                .route("/", web::get().to(test_handler)),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/")
            .insert_header(("Authorization", "InvalidFormat"))
            .to_request();

        let resp = app.call(req).await;
        assert!(resp.is_err());
        assert_eq!(resp.unwrap_err().to_string(), "Invalid API key");
    }
}
