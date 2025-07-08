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
        cancellation_token: CancellationToken,
    ) -> Result<Self> {
        let (node, incoming_messages, outgoing_messages) =
            build_p2p_node(port, cancellation_token.clone()).context("failed to build p2p node")?;
        Ok(Self {
            node,
            incoming_messages,
            cancellation_token,
            context: Context::new(wallet, outgoing_messages, validator_addresses),
        })
    }

    pub(crate) async fn run(self) {
        let Self {
            node,
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

#[derive(Clone)]
struct Context {
    authorized_peers: Arc<RwLock<HashSet<PeerId>>>,
    ongoing_auth_challenges: Arc<RwLock<HashMap<PeerId, String>>>, // use request_id?
    nonce_cache: Arc<RwLock<HashMap<String, SystemTime>>>,
    wallet: Wallet,
    outgoing_messages: Sender<OutgoingMessage>,
    validator_addresses: Arc<RwLock<HashSet<alloy::primitives::Address>>>,
}

impl Context {
    fn new(
        wallet: Wallet,
        outgoing_messages: Sender<OutgoingMessage>,
        validator_addresses: HashSet<alloy::primitives::Address>,
    ) -> Self {
        Self {
            authorized_peers: Arc::new(RwLock::new(HashSet::new())),
            ongoing_auth_challenges: Arc::new(RwLock::new(HashMap::new())),
            nonce_cache: Arc::new(RwLock::new(HashMap::new())),
            wallet,
            outgoing_messages,
            validator_addresses: Arc::new(RwLock::new(validator_addresses)),
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
    match request {
        p2p::Request::ValidatorAuthentication(req) => {
            tracing::debug!("handling ValidatorAuthentication request");
            match req {
                p2p::ValidatorAuthenticationRequest::Initiation(req) => {
                    let resp =
                        handle_validator_authentication_initiation_request(from, req, &context)
                            .await
                            .context("failed to handle ValidatorAuthenticationInitiationRequest")?;
                    let outgoing_message = resp.into_outgoing_message(channel);
                    context
                        .outgoing_messages
                        .send(outgoing_message)
                        .await
                        .context("failed to send ValidatorAuthentication response")?;
                }
                p2p::ValidatorAuthenticationRequest::Solution(req) => {
                    let resp = match handle_validator_authentication_initiation_solution(
                        from, req, &context,
                    )
                    .await
                    {
                        Ok(resp) => resp,
                        Err(e) => {
                            tracing::error!(
                                "failed to handle ValidatorAuthenticationSolutionRequest: {e}"
                            );
                            p2p::ValidatorAuthenticationSolutionResponse::Rejected.into()
                        }
                    };
                    let outgoing_message = resp.into_outgoing_message(channel);
                    context
                        .outgoing_messages
                        .send(outgoing_message)
                        .await
                        .context("failed to send ValidatorAuthenticationSolution response")?;
                }
            }
        }
        p2p::Request::HardwareChallenge(req) => {
            tracing::debug!("handling HardwareChallenge request");
        }
        p2p::Request::Invite(_) => {
            tracing::debug!("handling Invite request");
        }
        p2p::Request::GetTaskLogs => {
            tracing::debug!("handling GetTaskLogs request");
        }
        p2p::Request::Restart => {
            tracing::debug!("handling Restart request");
        }
    }
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
) -> Result<Response> {
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

    let validator_addresses = context.validator_addresses.read().await;
    if !validator_addresses.contains(&recovered_address) {
        anyhow::bail!("recovered address {recovered_address} is not in the list of authorized validator addresses");
    }

    Ok(p2p::ValidatorAuthenticationSolutionResponse::Granted.into())
}

async fn handle_incoming_response(response: p2p::Response) -> Result<()> {
    match response {
        p2p::Response::ValidatorAuthentication(_) => {
            // critical developer error, could panic here
            tracing::error!("worker should never receive ValidatorAuthentication responses");
        }
        p2p::Response::HardwareChallenge(_) => {
            tracing::debug!("handling HardwareChallenge response");
        }
        p2p::Response::Invite(_) => {
            tracing::debug!("handling Invite response");
        }
        p2p::Response::GetTaskLogs(_) => {
            tracing::debug!("handling GetTaskLogs response");
        }
        p2p::Response::Restart(_) => {
            tracing::debug!("handling Restart response");
        }
    }
    Ok(())
}
