use super::api::ApiResponse;
use super::metric::MetricEntry;
use super::task::Task;
use actix_web::HttpResponse;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
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

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default, ToSchema)]
pub struct TaskDetails {
    pub docker_image_id: Option<String>,
    pub container_id: Option<String>,
    pub container_status: Option<String>,
    pub container_created_at: Option<i64>,
    pub container_exit_code: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default, ToSchema)]
pub struct HeartbeatRequest {
    pub address: String,
    pub task_id: Option<String>,
    pub task_state: Option<String>,
    pub metrics: Option<Vec<MetricEntry>>,
    #[serde(default)]
    pub version: Option<String>,
    pub timestamp: Option<u64>,
    #[serde(default)]
    pub p2p_id: Option<String>,
    #[serde(default)]
    pub task_details: Option<TaskDetails>,
}
