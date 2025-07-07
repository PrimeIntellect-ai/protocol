use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

#[derive(Debug)]
pub struct IncomingMessage {
    pub peer: PeerId,
    pub message: libp2p::request_response::Message<Request, Response>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum OutgoingMessage {
    Request((PeerId, Request)),
    Response(
        (
            libp2p::request_response::ResponseChannel<Response>,
            Response,
        ),
    ),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    ValidatorAuthentication(ValidatorAuthenticationRequest),
    HardwareChallenge(HardwareChallengeRequest),
    Invite(InviteRequest),
    GetTaskLogs,
    Restart,
}

impl Request {
    pub fn into_outgoing_message(self, peer: PeerId) -> OutgoingMessage {
        OutgoingMessage::Request((peer, self))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    ValidatorAuthentication(ValidatorAuthenticationResponse),
    HardwareChallenge(HardwareChallengeResponse),
    Invite(InviteResponse),
    GetTaskLogs(GetTaskLogsResponse),
    Restart(RestartResponse),
}

impl Response {
    pub fn into_outgoing_message(
        self,
        channel: libp2p::request_response::ResponseChannel<Response>,
    ) -> OutgoingMessage {
        OutgoingMessage::Response((channel, self))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidatorAuthenticationRequest {
    Initiation(ValidatorAuthenticationInitiationRequest),
    Solution(ValidatorAuthenticationSolutionRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidatorAuthenticationResponse {
    Initiation(ValidatorAuthenticationInitiationResponse),
    Solution(ValidatorAuthenticationSolutionResponse),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorAuthenticationInitiationRequest {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorAuthenticationInitiationResponse {
    pub signed_message: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorAuthenticationSolutionRequest {
    pub signed_message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidatorAuthenticationSolutionResponse {
    Granted,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareChallengeRequest {
    pub challenge: String, // TODO
    pub timestamp: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareChallengeResponse {
    pub response: String, // TODO
    pub timestamp: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InviteRequestUrl {
    MasterUrl(String),
    MasterIpPort(String, u16),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteRequest {
    pub invite: String,
    pub pool_id: u32,
    pub url: InviteRequestUrl,
    pub timestamp: u64,
    pub expiration: [u8; 32],
    pub nonce: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InviteResponse {
    Ok,
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTaskLogsResponse {
    pub logs: Result<Vec<String>, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestartResponse {
    pub result: Result<(), String>,
}
