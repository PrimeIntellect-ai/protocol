use crate::Protocol;
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
    Request((PeerId, Vec<libp2p::Multiaddr>, Request)),
    Response(
        (
            libp2p::request_response::ResponseChannel<Response>,
            Response,
        ),
    ),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    Authentication(AuthenticationRequest),
    HardwareChallenge(HardwareChallengeRequest),
    Invite(InviteRequest),
    GetTaskLogs,
    RestartTask,
    General(GeneralRequest),
}

impl Request {
    pub fn into_outgoing_message(
        self,
        peer: PeerId,
        multiaddrs: Vec<libp2p::Multiaddr>,
    ) -> OutgoingMessage {
        OutgoingMessage::Request((peer, multiaddrs, self))
    }

    pub fn protocol(&self) -> Protocol {
        match self {
            Request::Authentication(_) => Protocol::Authentication,
            Request::HardwareChallenge(_) => Protocol::HardwareChallenge,
            Request::Invite(_) => Protocol::Invite,
            Request::GetTaskLogs => Protocol::GetTaskLogs,
            Request::RestartTask => Protocol::Restart,
            Request::General(_) => Protocol::General,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    Authentication(AuthenticationResponse),
    HardwareChallenge(HardwareChallengeResponse),
    Invite(InviteResponse),
    GetTaskLogs(GetTaskLogsResponse),
    RestartTask(RestartTaskResponse),
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
pub enum AuthenticationRequest {
    Initiation(AuthenticationInitiationRequest),
    Solution(AuthenticationSolutionRequest),
}

impl From<AuthenticationRequest> for Request {
    fn from(request: AuthenticationRequest) -> Self {
        Request::Authentication(request)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthenticationResponse {
    Initiation(AuthenticationInitiationResponse),
    Solution(AuthenticationSolutionResponse),
}

impl From<AuthenticationResponse> for Response {
    fn from(response: AuthenticationResponse) -> Self {
        Response::Authentication(response)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticationInitiationRequest {
    pub message: String,
}

impl From<AuthenticationInitiationRequest> for Request {
    fn from(request: AuthenticationInitiationRequest) -> Self {
        Request::Authentication(AuthenticationRequest::Initiation(request))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticationInitiationResponse {
    pub signature: String,
    pub message: String,
}

impl From<AuthenticationInitiationResponse> for Response {
    fn from(response: AuthenticationInitiationResponse) -> Self {
        Response::Authentication(AuthenticationResponse::Initiation(response))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticationSolutionRequest {
    pub signature: String,
}

impl From<AuthenticationSolutionRequest> for Request {
    fn from(request: AuthenticationSolutionRequest) -> Self {
        Request::Authentication(AuthenticationRequest::Solution(request))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthenticationSolutionResponse {
    Granted,
    Rejected,
}

impl From<AuthenticationSolutionResponse> for Response {
    fn from(response: AuthenticationSolutionResponse) -> Self {
        Response::Authentication(AuthenticationResponse::Solution(response))
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
pub enum RestartTaskResponse {
    Ok,
    Error(String),
}

impl From<RestartTaskResponse> for Response {
    fn from(response: RestartTaskResponse) -> Self {
        Response::RestartTask(response)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralRequest {
    pub data: Vec<u8>,
}

impl From<GeneralRequest> for Request {
    fn from(request: GeneralRequest) -> Self {
        Request::General(request)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralResponse {
    pub data: Vec<u8>,
}

impl From<GeneralResponse> for Response {
    fn from(response: GeneralResponse) -> Self {
        Response::General(response)
    }
}
