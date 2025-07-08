pub(crate) mod service;

pub(crate) use service::P2PContext;
pub(crate) use service::P2PService;

use anyhow::Context as _;
use anyhow::Result;
use p2p::Node;
use p2p::NodeBuilder;
use p2p::PeerId;
use p2p::Response;
use p2p::{IncomingMessage, Libp2pIncomingMessage, OutgoingMessage};
use shared::web3::wallet::Wallet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::docker::DockerService;

pub(crate) struct Service {
    node: Node,
    incoming_messages: Receiver<IncomingMessage>,
    cancellation_token: CancellationToken,
    context: Context,
}

impl Service {
    pub(crate) fn new(
        port: u16,
        wallet: Wallet,
        validator_addresses: HashSet<alloy::primitives::Address>,
        docker_service: Arc<DockerService>,
        cancellation_token: CancellationToken,
    ) -> Result<Self> {
        let (node, incoming_messages, outgoing_messages) =
            build_p2p_node(port, cancellation_token.clone()).context("failed to build p2p node")?;
        Ok(Self {
            node,
            incoming_messages,
            cancellation_token,
            context: Context::new(
                wallet,
                outgoing_messages,
                validator_addresses,
                docker_service,
            ),
        })
    }

    pub(crate) async fn run(self) {
        let Self {
            node: _,
            mut incoming_messages,
            cancellation_token,
            context,
        } = self;

        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => {
                    break;
                }
                Some(message) = (&mut incoming_messages).recv() => {
                    // TODO: spawn and store handles
                    if let Err(e) = handle_incoming_message(message, context.clone())
                        .await {
                        tracing::error!("failed to handle incoming message: {e}");
                        }
                }
            }
        }
    }
}

fn build_p2p_node(
    port: u16,
    cancellation_token: CancellationToken,
) -> Result<(Node, Receiver<IncomingMessage>, Sender<OutgoingMessage>)> {
    NodeBuilder::new()
        .with_port(port)
        .with_validator_authentication()
        .with_hardware_challenge()
        .with_invite()
        .with_get_task_logs()
        .with_restart()
        .with_cancellation_token(cancellation_token)
        .try_build()
}

#[derive(Clone)]
struct Context {
    authorized_peers: Arc<RwLock<HashSet<PeerId>>>,
    ongoing_auth_challenges: Arc<RwLock<HashMap<PeerId, String>>>, // use request_id?
    nonce_cache: Arc<RwLock<HashMap<String, SystemTime>>>,
    wallet: Wallet,
    outgoing_messages: Sender<OutgoingMessage>,
    validator_addresses: Arc<HashSet<alloy::primitives::Address>>,
    docker_service: Arc<DockerService>,
}

impl Context {
    fn new(
        wallet: Wallet,
        outgoing_messages: Sender<OutgoingMessage>,
        validator_addresses: HashSet<alloy::primitives::Address>,
        docker_service: Arc<DockerService>,
    ) -> Self {
        Self {
            authorized_peers: Arc::new(RwLock::new(HashSet::new())),
            ongoing_auth_challenges: Arc::new(RwLock::new(HashMap::new())),
            nonce_cache: Arc::new(RwLock::new(HashMap::new())),
            wallet,
            outgoing_messages,
            validator_addresses: Arc::new(validator_addresses),
            docker_service,
        }
    }
}

async fn handle_incoming_message(message: IncomingMessage, context: Context) -> Result<()> {
    match message.message {
        Libp2pIncomingMessage::Request {
            request_id: _,
            request,
            channel,
        } => {
            tracing::debug!("received incoming request {request:?}");
            handle_incoming_request(message.peer, request, channel, context).await?;
        }
        Libp2pIncomingMessage::Response {
            request_id: _,
            response,
        } => {
            tracing::debug!("received incoming response {response:?}");
            handle_incoming_response(response).await?;
        }
    }
    Ok(())
}

async fn handle_incoming_request(
    from: PeerId,
    request: p2p::Request,
    channel: p2p::ResponseChannel,
    context: Context,
) -> Result<()> {
    let resp = match request {
        p2p::Request::ValidatorAuthentication(req) => {
            tracing::debug!("handling ValidatorAuthentication request");
            match req {
                p2p::ValidatorAuthenticationRequest::Initiation(req) => {
                    handle_validator_authentication_initiation_request(from, req, &context)
                        .await
                        .context("failed to handle ValidatorAuthenticationInitiationRequest")?
                }
                p2p::ValidatorAuthenticationRequest::Solution(req) => {
                    match handle_validator_authentication_initiation_solution(from, req, &context)
                        .await
                    {
                        Ok(resp) => p2p::ValidatorAuthenticationSolutionResponse::Granted.into(),
                        Err(e) => {
                            tracing::error!(
                                "failed to handle ValidatorAuthenticationSolutionRequest: {e}"
                            );
                            p2p::ValidatorAuthenticationSolutionResponse::Rejected.into()
                        }
                    }
                }
            }
        }
        p2p::Request::HardwareChallenge(_) => {
            tracing::debug!("handling HardwareChallenge request");
            todo!()
        }
        p2p::Request::Invite(_) => {
            tracing::debug!("handling Invite request");
            handle_invite_request(from, request, &context).await
        }
        p2p::Request::GetTaskLogs => {
            tracing::debug!("handling GetTaskLogs request");
            handle_get_task_logs_request(from, &context).await
        }
        p2p::Request::Restart => {
            tracing::debug!("handling Restart request");
            handle_restart_request(from, &context).await
        }
    };

    let outgoing_message = resp.into_outgoing_message(channel);
    context
        .outgoing_messages
        .send(outgoing_message)
        .await
        .context("failed to send ValidatorAuthentication response")?;

    Ok(())
}

async fn handle_validator_authentication_initiation_request(
    from: PeerId,
    req: p2p::ValidatorAuthenticationInitiationRequest,
    context: &Context,
) -> Result<Response> {
    use rand_v8::Rng as _;
    use shared::security::request_signer::sign_message;

    // generate a fresh cryptographically secure challenge message for this auth attempt
    let challenge_bytes: [u8; 32] = rand_v8::rngs::OsRng.gen();
    let challenge_message = hex::encode(challenge_bytes);
    let signature = sign_message(&req.message, &context.wallet)
        .await
        .map_err(|e| anyhow::anyhow!("failed to sign message: {e:?}"))?;

    // store the challenge message in nonce cache to prevent replay
    let mut nonce_cache = context.nonce_cache.write().await;
    nonce_cache.insert(challenge_message.clone(), SystemTime::now());

    // store the current challenge for this peer
    let mut ongoing_auth_challenges = context.ongoing_auth_challenges.write().await;
    ongoing_auth_challenges.insert(from, challenge_message.clone());

    Ok(p2p::ValidatorAuthenticationInitiationResponse {
        message: challenge_message,
        signature,
    }
    .into())
}

async fn handle_validator_authentication_initiation_solution(
    from: PeerId,
    req: p2p::ValidatorAuthenticationSolutionRequest,
    context: &Context,
) -> Result<()> {
    use std::str::FromStr as _;

    let mut ongoing_auth_challenges = context.ongoing_auth_challenges.write().await;
    let challenge_message = ongoing_auth_challenges
        .remove(&from)
        .ok_or_else(|| anyhow::anyhow!("no ongoing authentication challenge for peer {from}"))?;

    let mut nonce_cache = context.nonce_cache.write().await;
    if nonce_cache.remove(&challenge_message).is_none() {
        anyhow::bail!("challenge message {challenge_message} not found in nonce cache");
    }

    let Ok(signature) = alloy::primitives::Signature::from_str(&req.signature) else {
        anyhow::bail!("failed to parse signature from message");
    };

    let Ok(recovered_address) = signature.recover_address_from_msg(challenge_message) else {
        anyhow::bail!("failed to recover address from signature and message");
    };

    if !context.validator_addresses.contains(&recovered_address) {
        anyhow::bail!("recovered address {recovered_address} is not in the list of authorized validator addresses");
    }

    Ok(())
}

async fn handle_invite_request(
    from: PeerId,
    _request: p2p::Request,
    context: &Context,
) -> Response {
    let authorized_peers = context.authorized_peers.read().await;
    if !authorized_peers.contains(&from) {
        return p2p::InviteResponse::Error("unauthorized".to_string()).into();
    }

    p2p::InviteResponse::Ok.into()
}

async fn handle_get_task_logs_request(from: PeerId, context: &Context) -> Response {
    let authorized_peers = context.authorized_peers.read().await;
    if !authorized_peers.contains(&from) {
        return p2p::GetTaskLogsResponse::Error("unauthorized".to_string()).into();
    }

    match context.docker_service.get_logs().await {
        Ok(logs) => p2p::GetTaskLogsResponse::Ok(logs).into(),
        Err(e) => {
            return p2p::GetTaskLogsResponse::Error(format!("failed to get task logs: {e:?}"))
                .into();
        }
    }
}

async fn handle_restart_request(from: PeerId, context: &Context) -> Response {
    let authorized_peers = context.authorized_peers.read().await;
    if !authorized_peers.contains(&from) {
        return p2p::RestartResponse::Error("unauthorized".to_string()).into();
    }

    match context.docker_service.restart_task().await {
        Ok(()) => p2p::RestartResponse::Ok.into(),
        Err(e) => p2p::RestartResponse::Error(format!("failed to restart task: {e:?}")).into(),
    }
}

async fn handle_incoming_response(response: p2p::Response) -> Result<()> {
    // critical developer error if any of these happen, could panic here
    match response {
        p2p::Response::ValidatorAuthentication(_) => {
            tracing::error!("worker should never receive ValidatorAuthentication responses");
        }
        p2p::Response::HardwareChallenge(_) => {
            tracing::error!("worker should never receive HardwareChallenge responses");
        }
        p2p::Response::Invite(_) => {
            tracing::error!("worker should never receive Invite responses");
        }
        p2p::Response::GetTaskLogs(_) => {
            tracing::error!("worker should never receive GetTaskLogs responses");
        }
        p2p::Response::Restart(_) => {
            tracing::error!("worker should never receive Restart responses");
        }
    }
    Ok(())
}
