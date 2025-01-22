use super::api::ApiResponse;
use super::task::Task;
use actix_web::HttpResponse;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HeartbeatResponse {
    pub current_task: Option<Task>,
}

impl From<HeartbeatResponse> for ApiResponse<HeartbeatResponse> {
    fn from(response: HeartbeatResponse) -> Self {
        ApiResponse::new(true, response)
    }
}

impl From<HeartbeatResponse> for HttpResponse {
    fn from(response: HeartbeatResponse) -> Self {
        ApiResponse::new(true, response).into()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HeartbeatRequest {
    pub address: String,
}