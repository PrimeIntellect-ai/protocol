use crate::web3::wallet::Wallet;
use anyhow::{bail, Context as _, Result};
use futures::stream::FuturesUnordered;
use p2p::{
    AuthenticationInitiationRequest, AuthenticationResponse, AuthenticationSolutionRequest,
    IncomingMessage, Libp2pIncomingMessage, Node, NodeBuilder, OutgoingMessage, PeerId, Protocol,
    Protocols, Response,
};
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

pub struct OutgoingRequest {
    pub peer_wallet_address: alloy::primitives::Address,
    pub request: p2p::Request,
    pub peer_id: String,
    pub multiaddrs: Vec<String>,
    pub response_tx: tokio::sync::oneshot::Sender<p2p::Response>,
}

/// A p2p service implementation that is used by the validator and the orchestrator.
/// It handles the authentication protocol used before sending
/// requests to the worker.
pub struct Service {
    node: Node,
    dial_tx: p2p::DialSender,
    incoming_messages_rx: Receiver<IncomingMessage>,
    outgoing_messages_rx: Receiver<OutgoingRequest>,
    cancellation_token: CancellationToken,
    context: Context,
}

impl Service {
    pub fn new(
        keypair: p2p::Keypair,
        port: u16,
        cancellation_token: CancellationToken,
        wallet: Wallet,
        protocols: Protocols,
    ) -> Result<(Self, Sender<OutgoingRequest>)> {
        let (node, dial_tx, incoming_messages_rx, outgoing_messages) =
            build_p2p_node(keypair, port, cancellation_token.clone(), protocols.clone())
                .context("failed to build p2p node")?;
        let (outgoing_messages_tx, outgoing_messages_rx) = tokio::sync::mpsc::channel(100);

        Ok((
            Self {
                node,
                dial_tx,
                incoming_messages_rx,
                outgoing_messages_rx,
                cancellation_token,
                context: Context::new(outgoing_messages, wallet, protocols),
            },
            outgoing_messages_tx,
        ))
    }

    pub async fn run(self) {
        use futures::StreamExt as _;

        let Self {
            node,
            dial_tx,
            mut incoming_messages_rx,
            mut outgoing_messages_rx,
            cancellation_token,
            context,
        } = self;
        tokio::task::spawn(node.run());

        let mut incoming_message_handlers = FuturesUnordered::new();
        let mut outgoing_message_handlers = FuturesUnordered::new();

        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => {
                    break;
                }
                Some(message) = outgoing_messages_rx.recv() => {
                    let handle = tokio::task::spawn(handle_outgoing_message(message, dial_tx.clone(), context.clone()));
                    outgoing_message_handlers.push(handle);
                }
                Some(message) = incoming_messages_rx.recv() => {
                    let context = context.clone();
                    let handle = tokio::task::spawn(
                            handle_incoming_message(message, context)
                    );
                    incoming_message_handlers.push(handle);
                }
                Some(res) = incoming_message_handlers.next() => {
                    if let Err(e) = res {
                        log::error!("failed to handle incoming message: {e}");
                    }
                }
                Some(res) = outgoing_message_handlers.next() => {
                    if let Err(e) = res {
                        log::error!("failed to handle outgoing message: {e}");
                    }
                }
            }
        }
    }
}

fn build_p2p_node(
    keypair: p2p::Keypair,
    port: u16,
    cancellation_token: CancellationToken,
    protocols: Protocols,
) -> Result<(
    Node,
    p2p::DialSender,
    Receiver<IncomingMessage>,
    Sender<OutgoingMessage>,
)> {
    NodeBuilder::new()
        .with_keypair(keypair)
        .with_port(port)
        .with_authentication()
        .with_protocols(protocols)
        .with_cancellation_token(cancellation_token)
        .try_build()
}

#[derive(Clone)]
struct Context {
    // outbound message channel; receiver is held by libp2p node
    outgoing_messages: Sender<OutgoingMessage>,

    // ongoing authentication requests
    ongoing_auth_requests: Arc<RwLock<HashMap<PeerId, OngoingAuthChallenge>>>,
    is_authenticated_with_peer: Arc<RwLock<HashSet<PeerId>>>,

    // this assumes that there is only one outbound request per protocol per peer at a time,
    // is this a correct assumption?
    // response channel is for sending the response back to the caller who initiated the request
    #[allow(clippy::type_complexity)]
    ongoing_outbound_requests:
        Arc<RwLock<HashMap<(PeerId, Protocol), tokio::sync::oneshot::Sender<Response>>>>,

    wallet: Wallet,
    protocols: Protocols,
}

#[derive(Debug)]
struct OngoingAuthChallenge {
    peer_wallet_address: alloy::primitives::Address,
    auth_challenge_request_message: String,
    outgoing_message: p2p::Request,
    response_tx: tokio::sync::oneshot::Sender<Response>,
}

impl Context {
    fn new(
        outgoing_messages: Sender<OutgoingMessage>,
        wallet: Wallet,
        protocols: Protocols,
    ) -> Self {
        Self {
            outgoing_messages,
            ongoing_auth_requests: Arc::new(RwLock::new(HashMap::new())),
            is_authenticated_with_peer: Arc::new(RwLock::new(HashSet::new())),
            ongoing_outbound_requests: Arc::new(RwLock::new(HashMap::new())),
            wallet,
            protocols,
        }
    }
}

async fn handle_outgoing_message(
    message: OutgoingRequest,
    dial_tx: p2p::DialSender,
    context: Context,
) -> Result<()> {
    use rand_v8::rngs::OsRng;
    use rand_v8::Rng as _;
    use std::str::FromStr as _;

    let OutgoingRequest {
        peer_wallet_address,
        request,
        peer_id,
        multiaddrs,
        response_tx,
    } = message;

    let peer_id = PeerId::from_str(&peer_id).context("failed to parse peer id")?;

    // check if we're authenticated already
    let is_authenticated_with_peer = context.is_authenticated_with_peer.read().await;
    if is_authenticated_with_peer.contains(&peer_id) {
        log::debug!(
            "already authenticated with peer {peer_id}, skipping validation authentication"
        );
        // multiaddresses are already known, as we've connected to them previously
        context
            .outgoing_messages
            .send(request.into_outgoing_message(peer_id, vec![]))
            .await
            .context("failed to send outgoing message")?;
        return Ok(());
    }

    log::info!("sending validation authentication request to {peer_id}");

    // first, dial the worker
    // ensure there's no ongoing challenge
    // use write-lock to make this atomic until we finish sending the auth request and writing to the map
    let mut ongoing_auth_requests = context.ongoing_auth_requests.write().await;
    if ongoing_auth_requests.contains_key(&peer_id) {
        bail!("ongoing auth request for {} already exists", peer_id);
    }

    let multiaddrs = multiaddrs
        .iter()
        .filter_map(|addr| p2p::Multiaddr::from_str(addr).ok()?.with_p2p(peer_id).ok())
        .collect::<Vec<_>>();
    if multiaddrs.is_empty() {
        bail!("no valid multiaddrs for peer id {peer_id}");
    }

    // TODO: we can improve this by checking if we're already connected to the peer before dialing
    let (res_tx, res_rx) = tokio::sync::oneshot::channel();
    dial_tx
        .send((multiaddrs.clone(), res_tx))
        .await
        .context("failed to send dial request")?;
    log::info!("dialing worker {peer_id} with multiaddrs: {multiaddrs:?}");
    res_rx
        .await
        .context("failed to receive dial response")?
        .context("failed to dial worker")?;
    log::info!("dialed worker {peer_id} with multiaddrs: {multiaddrs:?}");

    // create the authentication challenge request message
    let challenge_bytes: [u8; 32] = OsRng.gen();
    let auth_challenge_message: String = hex::encode(challenge_bytes);

    let req: p2p::Request = AuthenticationInitiationRequest {
        message: auth_challenge_message.clone(),
    }
    .into();
    let outgoing_message = req.into_outgoing_message(peer_id, multiaddrs);
    log::debug!("sending ValidatorAuthenticationInitiationRequest to {peer_id}");
    context
        .outgoing_messages
        .send(outgoing_message)
        .await
        .context("failed to send outgoing message")?;

    // store the ongoing auth challenge
    let ongoing_challenge = OngoingAuthChallenge {
        peer_wallet_address,
        auth_challenge_request_message: auth_challenge_message.clone(),
        outgoing_message: request,
        response_tx,
    };

    ongoing_auth_requests.insert(peer_id, ongoing_challenge);
    Ok(())
}

async fn handle_incoming_message(message: IncomingMessage, context: Context) -> Result<()> {
    match message.message {
        Libp2pIncomingMessage::Request {
            request_id: _,
            request,
            channel: _,
        } => {
            log::error!(
                "node should not receive incoming requests: {request:?} from {}",
                message.peer
            );
        }
        Libp2pIncomingMessage::Response {
            request_id: _,
            response,
        } => {
            log::debug!("received incoming response {response:?}");
            handle_incoming_response(message.peer, response, context)
                .await
                .context("failed to handle incoming response")?;
        }
    }
    Ok(())
}

async fn handle_incoming_response(
    from: PeerId,
    response: p2p::Response,
    context: Context,
) -> Result<()> {
    match response {
        p2p::Response::Authentication(resp) => {
            log::debug!("received ValidatorAuthenticationSolutionResponse from {from}: {resp:?}");
            handle_validation_authentication_response(from, resp, context)
                .await
                .context("failed to handle validator authentication response")?;
        }
        p2p::Response::HardwareChallenge(ref resp) => {
            if !context.protocols.has_hardware_challenge() {
                bail!("received HardwareChallengeResponse from {from}, but hardware challenge protocol is not enabled");
            }

            log::debug!("received HardwareChallengeResponse from {from}: {resp:?}");
            let mut ongoing_outbound_requests = context.ongoing_outbound_requests.write().await;
            let Some(response_tx) =
                ongoing_outbound_requests.remove(&(from, Protocol::HardwareChallenge))
            else {
                bail!(
                    "no ongoing hardware challenge for peer {from}, cannot handle HardwareChallengeResponse"
                );
            };
            let _ = response_tx.send(response);
        }
        p2p::Response::Invite(ref resp) => {
            if !context.protocols.has_invite() {
                bail!("received InviteResponse from {from}, but invite protocol is not enabled");
            }

            log::debug!("received InviteResponse from {from}: {resp:?}");
            let mut ongoing_outbound_requests = context.ongoing_outbound_requests.write().await;
            let Some(response_tx) = ongoing_outbound_requests.remove(&(from, Protocol::Invite))
            else {
                bail!("no ongoing invite for peer {from}, cannot handle InviteResponse");
            };
            let _ = response_tx.send(response);
        }
        p2p::Response::GetTaskLogs(ref resp) => {
            if !context.protocols.has_get_task_logs() {
                bail!("received GetTaskLogsResponse from {from}, but get task logs protocol is not enabled");
            }

            log::debug!("received GetTaskLogsResponse from {from}: {resp:?}");
            let mut ongoing_outbound_requests = context.ongoing_outbound_requests.write().await;
            let Some(response_tx) =
                ongoing_outbound_requests.remove(&(from, Protocol::GetTaskLogs))
            else {
                bail!("no ongoing GetTaskLogs for peer {from}, cannot handle GetTaskLogsResponse");
            };
            let _ = response_tx.send(response);
        }
        p2p::Response::RestartTask(ref resp) => {
            if !context.protocols.has_restart() {
                bail!("received RestartResponse from {from}, but restart protocol is not enabled");
            }

            log::debug!("received RestartResponse from {from}: {resp:?}");
            let mut ongoing_outbound_requests = context.ongoing_outbound_requests.write().await;
            let Some(response_tx) = ongoing_outbound_requests.remove(&(from, Protocol::Restart))
            else {
                bail!("no ongoing Restart for peer {from}, cannot handle RestartResponse");
            };
            let _ = response_tx.send(response);
        }
        p2p::Response::General(ref resp) => {
            if !context.protocols.has_general() {
                bail!("received GeneralResponse from {from}, but general protocol is not enabled");
            }

            log::debug!("received GeneralResponse from {from}: {resp:?}");
            let mut ongoing_outbound_requests = context.ongoing_outbound_requests.write().await;
            let Some(response_tx) = ongoing_outbound_requests.remove(&(from, Protocol::General))
            else {
                bail!("no ongoing General for peer {from}, cannot handle GeneralResponse");
            };
            let _ = response_tx.send(response);
        }
    }

    Ok(())
}

async fn handle_validation_authentication_response(
    from: PeerId,
    response: p2p::AuthenticationResponse,
    context: Context,
) -> Result<()> {
    use crate::security::request_signer::sign_message;
    use std::str::FromStr as _;

    match response {
        AuthenticationResponse::Initiation(req) => {
            let ongoing_auth_requests = context.ongoing_auth_requests.read().await;
            let Some(ongoing_challenge) = ongoing_auth_requests.get(&from) else {
                bail!(
                    "no ongoing hardware challenge for peer {from}, cannot handle ValidatorAuthenticationInitiationResponse"
                );
            };

            let Ok(parsed_signature) = alloy::primitives::Signature::from_str(&req.signature)
            else {
                bail!("failed to parse signature from response");
            };

            // recover address from the challenge message that the peer signed
            let Ok(recovered_address) = parsed_signature
                .recover_address_from_msg(&ongoing_challenge.auth_challenge_request_message)
            else {
                bail!("Failed to recover address from response signature")
            };

            // verify the recovered address matches the expected worker wallet address
            if recovered_address != ongoing_challenge.peer_wallet_address {
                bail!(
                    "peer address verification failed: expected {}, got {recovered_address}",
                    ongoing_challenge.peer_wallet_address,
                )
            }

            log::debug!("auth challenge initiation response received from node: {from}");
            let signature = sign_message(&req.message, &context.wallet).await.unwrap();

            let req: p2p::Request = AuthenticationSolutionRequest { signature }.into();
            let req = req.into_outgoing_message(from, vec![]);
            context
                .outgoing_messages
                .send(req)
                .await
                .context("failed to send outgoing message")?;
        }
        AuthenticationResponse::Solution(req) => {
            let mut ongoing_auth_requests = context.ongoing_auth_requests.write().await;
            let Some(ongoing_challenge) = ongoing_auth_requests.remove(&from) else {
                bail!(
                    "no ongoing hardware challenge for peer {from}, cannot handle ValidatorAuthenticationSolutionResponse"
                );
            };

            match req {
                p2p::AuthenticationSolutionResponse::Granted => {}
                p2p::AuthenticationSolutionResponse::Rejected => {
                    log::debug!("auth challenge rejected by node: {from}");
                    return Ok(());
                }
            }

            // auth was granted, finally send the hardware challenge
            let mut is_authenticated_with_peer = context.is_authenticated_with_peer.write().await;
            is_authenticated_with_peer.insert(from);

            let protocol = ongoing_challenge.outgoing_message.protocol();
            let req = ongoing_challenge
                .outgoing_message
                .into_outgoing_message(from, vec![]);
            context
                .outgoing_messages
                .send(req)
                .await
                .context("failed to send outgoing message")?;

            let mut ongoing_outbound_requests = context.ongoing_outbound_requests.write().await;
            ongoing_outbound_requests.insert((from, protocol), ongoing_challenge.response_tx);
        }
    }
    Ok(())
}
