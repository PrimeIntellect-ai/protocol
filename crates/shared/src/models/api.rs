use actix_web::HttpResponse;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct ApiResponse<T: Serialize> {
    pub success: bool,
    pub data: T,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn new(success: bool, data: T) -> Self {
        ApiResponse { success, data }
    }
}

impl<T: Serialize> From<ApiResponse<T>> for HttpResponse {
    fn from(response: ApiResponse<T>) -> Self {
        HttpResponse::Ok().json(response)
    }
}
