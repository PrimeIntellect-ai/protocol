use actix_web::HttpResponse;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
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
