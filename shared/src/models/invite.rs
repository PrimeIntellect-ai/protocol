use serde::Deserialize;
use serde::Serialize;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct InviteRequest {
    pub invite: String,
    pub pool_id: u32,
    // Either master url or ip and port
    pub master_url: Option<String>,
    pub master_ip: Option<String>,
    pub master_port: Option<u16>,
}

#[derive(Deserialize, Serialize)]
pub struct InviteResponse {
    pub status: String,
}
