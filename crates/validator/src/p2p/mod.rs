pub(crate) mod client;

pub use client::P2PClient;

use anyhow::{bail, Context as _, Result};
use futures::stream::FuturesUnordered;
use p2p::{
    IncomingMessage, Libp2pIncomingMessage, Node, NodeBuilder, OutgoingMessage, PeerId,
    ValidatorAuthenticationInitiationRequest, ValidatorAuthenticationResponse,
    ValidatorAuthenticationSolutionRequest,
};
use shared::web3::wallet::Wallet;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

pub(crate) struct Service {
    node: Node,
    dial_tx: p2p::DialSender,
    incoming_messages: Receiver<IncomingMessage>,
    hardware_challenge_rx: Receiver<HardwareChallengeRequest>,
    cancellation_token: CancellationToken,
    context: Context,
}

impl Service {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        keypair: p2p::Keypair,
        port: u16,
        cancellation_token: CancellationToken,
        wallet: Wallet,
    ) -> Result<(Self, Sender<HardwareChallengeRequest>)> {
        let (node, dial_tx, incoming_messages, outgoing_messages) =
            build_p2p_node(keypair, port, cancellation_token.clone())
                .context("failed to build p2p node")?;
        let (hardware_challenge_tx, hardware_challenge_rx) = tokio::sync::mpsc::channel(100);

        Ok((
            Self {
                node,
                dial_tx,
                incoming_messages,
                hardware_challenge_rx,
                cancellation_token,
                context: Context::new(outgoing_messages, wallet),
            },
            hardware_challenge_tx,
        ))
    }

    pub(crate) fn peer_id(&self) -> PeerId {
        self.node.peer_id()
    }

    pub(crate) fn listen_addrs(&self) -> &[p2p::Multiaddr] {
        self.node.listen_addrs()
    }

    pub(crate) async fn run(self) {
        use futures::StreamExt as _;

        let Self {
            node: _,
            dial_tx,
            mut incoming_messages,
            mut hardware_challenge_rx,
            cancellation_token,
            context,
        } = self;

        let mut message_handlers = FuturesUnordered::new();

        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => {
                    break;
                }
                Some(message) = hardware_challenge_rx.recv() => {
                    if let Err(e) = handle_outgoing_hardware_challenge(message, dial_tx.clone(), context.clone())
                        .await {
                        log::error!("failed to handle outgoing hardware challenge: {e}");
                        }
                }
                Some(message) = incoming_messages.recv() => {
                    let context = context.clone();
                    let handle = tokio::task::spawn(
                            handle_incoming_message(message, context)
                    );
                    message_handlers.push(handle);
                }
                Some(res) = message_handlers.next() => {
                    if let Err(e) = res {
                        log::error!("failed to handle incoming message: {e}");
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
) -> Result<(
    Node,
    p2p::DialSender,
    Receiver<IncomingMessage>,
    Sender<OutgoingMessage>,
)> {
    NodeBuilder::new()
        .with_keypair(keypair)
        .with_port(port)
        .with_validator_authentication()
        .with_hardware_challenge()
        .with_cancellation_token(cancellation_token)
        .try_build()
}

pub(crate) struct HardwareChallengeRequest {
    worker_wallet_address: alloy::primitives::Address,
    worker_p2p_id: String,
    worker_addresses: Vec<String>,
    challenge: p2p::ChallengeRequest,
    response_tx: tokio::sync::oneshot::Sender<p2p::ChallengeResponse>,
}

#[derive(Clone)]
struct Context {
    outgoing_messages: Sender<OutgoingMessage>,
    ongoing_auth_requests: Arc<RwLock<HashMap<PeerId, OngoingAuthChallenge>>>,
    ongoing_hardware_challenges:
        Arc<RwLock<HashMap<PeerId, tokio::sync::oneshot::Sender<p2p::ChallengeResponse>>>>,
    wallet: Wallet,
}

#[derive(Debug)]
struct OngoingAuthChallenge {
    worker_wallet_address: alloy::primitives::Address,
    auth_challenge_request_message: String,
    hardware_challenge: p2p::ChallengeRequest,
    hardware_challenge_response_tx: tokio::sync::oneshot::Sender<p2p::ChallengeResponse>,
}

impl Context {
    fn new(outgoing_messages: Sender<OutgoingMessage>, wallet: Wallet) -> Self {
        Self {
            outgoing_messages,
            ongoing_auth_requests: Arc::new(RwLock::new(HashMap::new())),
            ongoing_hardware_challenges: Arc::new(RwLock::new(HashMap::new())),
            wallet,
        }
    }
}

async fn handle_outgoing_hardware_challenge(
    request: HardwareChallengeRequest,
    dial_tx: p2p::DialSender,
    context: Context,
) -> Result<()> {
    use rand_v8::rngs::OsRng;
    use rand_v8::Rng as _;
    use std::str::FromStr as _;

    let HardwareChallengeRequest {
        worker_wallet_address,
        worker_p2p_id,
        worker_addresses,
        challenge,
        response_tx,
    } = request;

    log::debug!(
        "sending hardware challenge to {} with addresses {:?}",
        worker_p2p_id,
        worker_addresses
    );

    // first, dial the worker
    let worker_p2p_id =
        PeerId::from_str(&worker_p2p_id).context("failed to parse worker p2p id")?;

    // ensure there's no ongoing challenge
    // use write-lock to make this atomic until we finish sending the auth request and writing to the map
    let mut ongoing_auth_requests = context.ongoing_auth_requests.write().await;
    if ongoing_auth_requests.contains_key(&worker_p2p_id) {
        bail!(
            "ongoing hardware challenge for {} already exists",
            worker_p2p_id
        );
    }

    let multiaddrs = worker_addresses
        .iter()
        .filter_map(|addr| {
            Some(
                p2p::Multiaddr::from_str(addr)
                    .ok()?
                    .with_p2p(worker_p2p_id.clone())
                    .ok()?,
            )
        })
        .collect::<Vec<_>>();
    if multiaddrs.is_empty() {
        bail!("no valid multiaddrs for worker p2p id {worker_p2p_id}");
    }

    // TODO: we can improve this by checking if we're already connected to the peer before dialing
    let (res_tx, res_rx) = tokio::sync::oneshot::channel();
    dial_tx
        .send((multiaddrs, res_tx))
        .await
        .context("failed to send dial request")?;
    res_rx
        .await
        .context("failed to receive dial response")?
        .context("failed to dial worker")?;

    // create the authentication challenge request message
    let challenge_bytes: [u8; 32] = OsRng.gen();
    let auth_challenge_message: String = hex::encode(challenge_bytes);

    let req: p2p::Request = ValidatorAuthenticationInitiationRequest {
        message: auth_challenge_message.clone(),
    }
    .into();
    let outgoing_message = req.into_outgoing_message(worker_p2p_id.clone());
    log::debug!(
        "sending ValidatorAuthenticationInitiationRequest to {}",
        worker_p2p_id
    );
    context
        .outgoing_messages
        .send(outgoing_message)
        .await
        .context("failed to send outgoing message")?;

    // store the ongoing hardware challenge
    let ongoing_challenge = OngoingAuthChallenge {
        worker_wallet_address,
        auth_challenge_request_message: auth_challenge_message.clone(),
        hardware_challenge: challenge,
        hardware_challenge_response_tx: response_tx,
    };

    ongoing_auth_requests.insert(worker_p2p_id.clone(), ongoing_challenge);
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
                "validator should not receive incoming requests: {request:?} from {}",
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
        p2p::Response::ValidatorAuthentication(resp) => {
            log::debug!("received ValidatorAuthenticationSolutionResponse from {from}: {resp:?}");
            handle_validation_authentication_response(from, resp, context)
                .await
                .context("failed to handle validator authentication response")?;
        }
        p2p::Response::HardwareChallenge(resp) => {
            log::debug!("received HardwareChallengeResponse from {from}: {resp:?}");
            let mut ongoing_hardware_challenges = context.ongoing_hardware_challenges.write().await;
            let Some(response_tx) = ongoing_hardware_challenges.remove(&from) else {
                bail!(
                    "no ongoing hardware challenge for peer {from}, cannot handle HardwareChallengeResponse"
                );
            };
            let _ = response_tx.send(resp.response); // timestamp is silently dropped, is it actually used anywhere?
        }
        p2p::Response::Invite(_) => {
            log::error!("validator should not receive `Invite` responses: from {from}");
        }
        p2p::Response::GetTaskLogs(_) => {
            log::error!("validator should not receive `GetTaskLogs` responses: from {from}");
        }
        p2p::Response::Restart(_) => {
            log::error!("validator should not receive `Restart` responses: from {from}");
        }
        p2p::Response::General(_) => {
            todo!()
        }
    }

    Ok(())
}

async fn handle_validation_authentication_response(
    from: PeerId,
    response: p2p::ValidatorAuthenticationResponse,
    context: Context,
) -> Result<()> {
    use shared::security::request_signer::sign_message;
    use std::str::FromStr as _;

    match response {
        ValidatorAuthenticationResponse::Initiation(req) => {
            let ongoing_auth_requests = context.ongoing_auth_requests.read().await;
            let Some(ongoing_challenge) = ongoing_auth_requests.get(&from) else {
                bail!(
                    "no ongoing hardware challenge for peer {from}, cannot handle ValidatorAuthenticationInitiationResponse"
                );
            };

            let Ok(parsed_signature) = alloy::primitives::Signature::from_str(&req.signature)
            else {
                bail!("Failed to parse signature from server");
            };

            // recover address from the challenge message that the server signed
            let Ok(recovered_address) = parsed_signature
                .recover_address_from_msg(&ongoing_challenge.auth_challenge_request_message)
            else {
                bail!("Failed to recover address from server signature")
            };

            // verify the recovered address matches the expected worker wallet address
            if recovered_address != ongoing_challenge.worker_wallet_address {
                bail!(
                    "Server address verification failed: expected {}, got {recovered_address}",
                    ongoing_challenge.worker_wallet_address,
                )
            }

            log::debug!("auth challenge initiation response received from node: {from}");
            let signature = sign_message(&req.message, &context.wallet).await.unwrap();

            let req: p2p::Request = ValidatorAuthenticationSolutionRequest { signature }.into();
            let req = req.into_outgoing_message(from);
            context
                .outgoing_messages
                .send(req)
                .await
                .context("failed to send outgoing message")?;
        }
        ValidatorAuthenticationResponse::Solution(req) => {
            let mut ongoing_auth_requests = context.ongoing_auth_requests.write().await;
            let Some(ongoing_challenge) = ongoing_auth_requests.remove(&from) else {
                bail!(
                    "no ongoing hardware challenge for peer {from}, cannot handle ValidatorAuthenticationSolutionResponse"
                );
            };

            match req {
                p2p::ValidatorAuthenticationSolutionResponse::Granted => {}
                p2p::ValidatorAuthenticationSolutionResponse::Rejected => {
                    log::debug!("auth challenge rejected by node: {from}");
                    return Ok(());
                }
            }

            // auth was granted, finally send the hardware challenge
            let req: p2p::Request = p2p::HardwareChallengeRequest {
                challenge: ongoing_challenge.hardware_challenge,
                timestamp: std::time::SystemTime::now(),
            }
            .into();
            let req = req.into_outgoing_message(from);
            context
                .outgoing_messages
                .send(req)
                .await
                .context("failed to send outgoing message")?;

            let mut ongoing_hardware_challenges = context.ongoing_hardware_challenges.write().await;
            ongoing_hardware_challenges
                .insert(from, ongoing_challenge.hardware_challenge_response_tx);
        }
    }
    Ok(())
}
