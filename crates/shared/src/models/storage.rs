use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug)]
pub struct RequestUploadRequest {
    pub file_name: String,
    pub file_size: u64,
    pub file_type: String,
    pub sha256: String,
    pub task_id: String,
}
