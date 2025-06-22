use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Deserialize, Serialize, Debug, ToSchema)]
pub struct RequestUploadRequest {
    pub file_name: String,
    pub file_size: u64,
    pub file_type: String,
    pub sha256: String,
    pub task_id: String,
}
