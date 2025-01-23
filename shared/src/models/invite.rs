use serde::Deserialize;
use serde::Serialize;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct InviteRequest {
    pub invite: String,
    pub pool_id: u32,
    pub master_ip: String,
    pub master_port: u16,
}

#[derive(Deserialize, Serialize)]
pub struct InviteResponse {
    pub status: String,
}
