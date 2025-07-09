use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

mod hardware_challenge;

pub use hardware_challenge::*;

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
    General(GeneralRequest),
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
    General(GeneralResponse),
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

impl From<ValidatorAuthenticationRequest> for Request {
    fn from(request: ValidatorAuthenticationRequest) -> Self {
        Request::ValidatorAuthentication(request)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidatorAuthenticationResponse {
    Initiation(ValidatorAuthenticationInitiationResponse),
    Solution(ValidatorAuthenticationSolutionResponse),
}

impl From<ValidatorAuthenticationResponse> for Response {
    fn from(response: ValidatorAuthenticationResponse) -> Self {
        Response::ValidatorAuthentication(response)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorAuthenticationInitiationRequest {
    pub message: String,
}

impl From<ValidatorAuthenticationInitiationRequest> for Request {
    fn from(request: ValidatorAuthenticationInitiationRequest) -> Self {
        Request::ValidatorAuthentication(ValidatorAuthenticationRequest::Initiation(request))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorAuthenticationInitiationResponse {
    pub signature: String,
    pub message: String,
}

impl From<ValidatorAuthenticationInitiationResponse> for Response {
    fn from(response: ValidatorAuthenticationInitiationResponse) -> Self {
        Response::ValidatorAuthentication(ValidatorAuthenticationResponse::Initiation(response))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorAuthenticationSolutionRequest {
    pub signature: String,
}

impl From<ValidatorAuthenticationSolutionRequest> for Request {
    fn from(request: ValidatorAuthenticationSolutionRequest) -> Self {
        Request::ValidatorAuthentication(ValidatorAuthenticationRequest::Solution(request))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidatorAuthenticationSolutionResponse {
    Granted,
    Rejected,
}

impl From<ValidatorAuthenticationSolutionResponse> for Response {
    fn from(response: ValidatorAuthenticationSolutionResponse) -> Self {
        Response::ValidatorAuthentication(ValidatorAuthenticationResponse::Solution(response))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareChallengeRequest {
    pub challenge: ChallengeRequest,
    pub timestamp: SystemTime,
}

impl From<HardwareChallengeRequest> for Request {
    fn from(request: HardwareChallengeRequest) -> Self {
        Request::HardwareChallenge(request)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareChallengeResponse {
    pub response: ChallengeResponse,
    pub timestamp: SystemTime,
}

impl From<HardwareChallengeResponse> for Response {
    fn from(response: HardwareChallengeResponse) -> Self {
        Response::HardwareChallenge(response)
    }
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

impl From<InviteRequest> for Request {
    fn from(request: InviteRequest) -> Self {
        Request::Invite(request)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InviteResponse {
    Ok,
    Error(String),
}

impl From<InviteResponse> for Response {
    fn from(response: InviteResponse) -> Self {
        Response::Invite(response)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GetTaskLogsResponse {
    Ok(String),
    Error(String),
}

impl From<GetTaskLogsResponse> for Response {
    fn from(response: GetTaskLogsResponse) -> Self {
        Response::GetTaskLogs(response)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RestartResponse {
    Ok,
    Error(String),
}

impl From<RestartResponse> for Response {
    fn from(response: RestartResponse) -> Self {
        Response::Restart(response)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralRequest {
    data: Vec<u8>,
}

impl From<GeneralRequest> for Request {
    fn from(request: GeneralRequest) -> Self {
        Request::General(request)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralResponse {
    data: Vec<u8>,
}

impl From<GeneralResponse> for Response {
    fn from(response: GeneralResponse) -> Self {
        Response::General(response)
    }
}
