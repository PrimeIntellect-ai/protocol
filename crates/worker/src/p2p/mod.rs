use anyhow::Context as _;
use anyhow::Result;
use futures::stream::FuturesUnordered;
use p2p::InviteRequestUrl;
use p2p::Node;
use p2p::NodeBuilder;
use p2p::PeerId;
use p2p::Response;
use p2p::{IncomingMessage, Libp2pIncomingMessage, OutgoingMessage};
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::wallet::Wallet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::docker::DockerService;
use crate::operations::heartbeat::service::HeartbeatService;
use crate::state::system_state::SystemState;
use shared::web3::wallet::WalletProvider;

pub(crate) struct Service {
    node: Node,
    incoming_messages: Receiver<IncomingMessage>,
    cancellation_token: CancellationToken,
    context: Context,
}

impl Service {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        keypair: p2p::Keypair,
        port: u16,
        wallet: Wallet,
        validator_addresses: HashSet<alloy::primitives::Address>,
        docker_service: Arc<DockerService>,
        heartbeat_service: Arc<HeartbeatService>,
        system_state: Arc<SystemState>,
        contracts: Contracts<WalletProvider>,
        provider_wallet: Wallet,
        cancellation_token: CancellationToken,
    ) -> Result<Self> {
        let (node, incoming_messages, outgoing_messages) =
            build_p2p_node(keypair, port, cancellation_token.clone())
                .context("failed to build p2p node")?;
        Ok(Self {
            node,
            incoming_messages,
            cancellation_token,
            context: Context::new(
                wallet,
                outgoing_messages,
                validator_addresses,
                docker_service,
                heartbeat_service,
                system_state,
                contracts,
                provider_wallet,
            ),
        })
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
            mut incoming_messages,
            cancellation_token,
            context,
        } = self;

        let mut message_handlers = FuturesUnordered::new();

        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => {
                    break;
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
                        tracing::error!("failed to handle incoming message: {e}");
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
) -> Result<(Node, Receiver<IncomingMessage>, Sender<OutgoingMessage>)> {
    let (node, _, incoming_message_rx, outgoing_message_tx) = NodeBuilder::new()
        .with_keypair(keypair)
        .with_port(port)
        .with_authentication()
        .with_hardware_challenge()
        .with_invite()
        .with_get_task_logs()
        .with_restart()
        .with_cancellation_token(cancellation_token)
        .try_build()
        .context("failed to build p2p node")?;
    Ok((node, incoming_message_rx, outgoing_message_tx))
}

#[derive(Clone)]
struct Context {
    authorized_peers: Arc<RwLock<HashSet<PeerId>>>,
    wallet: Wallet,
    validator_addresses: Arc<HashSet<alloy::primitives::Address>>,

    // for validator authentication requests
    ongoing_auth_challenges: Arc<RwLock<HashMap<PeerId, String>>>, // use request_id?
    nonce_cache: Arc<RwLock<HashMap<String, SystemTime>>>,
    outgoing_messages: Sender<OutgoingMessage>,

    // for get_task_logs and restart requests
    docker_service: Arc<DockerService>,

    // for invite requests
    heartbeat_service: Arc<HeartbeatService>,
    system_state: Arc<SystemState>,
    contracts: Contracts<WalletProvider>,
    provider_wallet: Wallet,
}

impl Context {
    #[allow(clippy::too_many_arguments)]
    fn new(
        wallet: Wallet,
        outgoing_messages: Sender<OutgoingMessage>,
        validator_addresses: HashSet<alloy::primitives::Address>,
        docker_service: Arc<DockerService>,
        heartbeat_service: Arc<HeartbeatService>,
        system_state: Arc<SystemState>,
        contracts: Contracts<WalletProvider>,
        provider_wallet: Wallet,
    ) -> Self {
        Self {
            authorized_peers: Arc::new(RwLock::new(HashSet::new())),
            ongoing_auth_challenges: Arc::new(RwLock::new(HashMap::new())),
            nonce_cache: Arc::new(RwLock::new(HashMap::new())),
            wallet,
            outgoing_messages,
            validator_addresses: Arc::new(validator_addresses),
            docker_service,
            heartbeat_service,
            system_state,
            contracts,
            provider_wallet,
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
            handle_incoming_response(response);
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
        p2p::Request::Authentication(req) => {
            tracing::debug!("handling ValidatorAuthentication request");
            match req {
                p2p::AuthenticationRequest::Initiation(req) => {
                    handle_validator_authentication_initiation_request(from, req, &context)
                        .await
                        .context("failed to handle ValidatorAuthenticationInitiationRequest")?
                }
                p2p::AuthenticationRequest::Solution(req) => {
                    match handle_validator_authentication_solution_request(from, req, &context)
                        .await
                    {
                        Ok(()) => p2p::AuthenticationSolutionResponse::Granted.into(),
                        Err(e) => {
                            tracing::error!(
                                "failed to handle ValidatorAuthenticationSolutionRequest: {e:?}"
                            );
                            p2p::AuthenticationSolutionResponse::Rejected.into()
                        }
                    }
                }
            }
        }
        p2p::Request::HardwareChallenge(req) => {
            tracing::debug!("handling HardwareChallenge request");
            handle_hardware_challenge_request(from, req, &context)
                .await
                .context("failed to handle HardwareChallenge request")?
        }
        p2p::Request::Invite(req) => {
            tracing::debug!("handling Invite request");
            match handle_invite_request(from, req, &context).await {
                Ok(()) => p2p::InviteResponse::Ok.into(),
                Err(e) => p2p::InviteResponse::Error(e.to_string()).into(),
            }
        }
        p2p::Request::GetTaskLogs => {
            tracing::debug!("handling GetTaskLogs request");
            handle_get_task_logs_request(from, &context).await
        }
        p2p::Request::RestartTask => {
            tracing::debug!("handling Restart request");
            handle_restart_request(from, &context).await
        }
        p2p::Request::General(_) => {
            todo!()
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
    req: p2p::AuthenticationInitiationRequest,
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

    Ok(p2p::AuthenticationInitiationResponse {
        message: challenge_message,
        signature,
    }
    .into())
}

async fn handle_validator_authentication_solution_request(
    from: PeerId,
    req: p2p::AuthenticationSolutionRequest,
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

    let mut authorized_peers = context.authorized_peers.write().await;
    authorized_peers.insert(from);
    Ok(())
}

async fn handle_hardware_challenge_request(
    from: PeerId,
    request: p2p::HardwareChallengeRequest,
    context: &Context,
) -> Result<Response> {
    let authorized_peers = context.authorized_peers.read().await;
    if !authorized_peers.contains(&from) {
        // TODO: error response variant?
        anyhow::bail!("unauthorized peer {from} attempted to access HardwareChallenge request");
    }

    let challenge_response = p2p::calc_matrix(&request.challenge);
    let response = p2p::HardwareChallengeResponse {
        response: challenge_response,
        timestamp: SystemTime::now(),
    };
    Ok(response.into())
}

async fn handle_get_task_logs_request(from: PeerId, context: &Context) -> Response {
    let authorized_peers = context.authorized_peers.read().await;
    if !authorized_peers.contains(&from) {
        return p2p::GetTaskLogsResponse::Error("unauthorized".to_string()).into();
    }

    match context.docker_service.get_logs().await {
        Ok(logs) => p2p::GetTaskLogsResponse::Ok(logs).into(),
        Err(e) => p2p::GetTaskLogsResponse::Error(format!("failed to get task logs: {e:?}")).into(),
    }
}

async fn handle_restart_request(from: PeerId, context: &Context) -> Response {
    let authorized_peers = context.authorized_peers.read().await;
    if !authorized_peers.contains(&from) {
        return p2p::RestartTaskResponse::Error("unauthorized".to_string()).into();
    }

    match context.docker_service.restart_task().await {
        Ok(()) => p2p::RestartTaskResponse::Ok.into(),
        Err(e) => p2p::RestartTaskResponse::Error(format!("failed to restart task: {e:?}")).into(),
    }
}

fn handle_incoming_response(response: p2p::Response) {
    // critical developer error if any of these happen, could panic here
    match response {
        p2p::Response::Authentication(_) => {
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
        p2p::Response::RestartTask(_) => {
            tracing::error!("worker should never receive Restart responses");
        }
        p2p::Response::General(_) => {
            todo!()
        }
    }
}

async fn handle_invite_request(
    from: PeerId,
    req: p2p::InviteRequest,
    context: &Context,
) -> Result<()> {
    use crate::console::Console;
    use shared::web3::contracts::helpers::utils::retry_call;
    use shared::web3::contracts::structs::compute_pool::PoolStatus;

    let authorized_peers = context.authorized_peers.read().await;
    if !authorized_peers.contains(&from) {
        return Err(anyhow::anyhow!(
            "unauthorized peer {from} attempted to send invite"
        ));
    }

    if context.system_state.is_running().await {
        anyhow::bail!("heartbeat is currently running and in a compute pool");
    }

    if req.pool_id != context.system_state.get_compute_pool_id() {
        anyhow::bail!(
            "pool ID mismatch: expected {}, got {}",
            context.system_state.get_compute_pool_id(),
            req.pool_id
        );
    }

    let invite_bytes = hex::decode(&req.invite).context("failed to decode invite hex")?;

    if invite_bytes.len() < 65 {
        anyhow::bail!("invite data is too short, expected at least 65 bytes");
    }

    let contracts = &context.contracts;
    let pool_id = alloy::primitives::U256::from(req.pool_id);

    let bytes_array: [u8; 65] = match invite_bytes[..65].try_into() {
        Ok(array) => array,
        Err(_) => {
            anyhow::bail!("failed to convert invite bytes to 65 byte array");
        }
    };

    let provider_address = context.provider_wallet.wallet.default_signer().address();

    let pool_info = match contracts.compute_pool.get_pool_info(pool_id).await {
        Ok(info) => info,
        Err(err) => {
            anyhow::bail!("failed to get pool info: {err:?}");
        }
    };

    if let PoolStatus::PENDING = pool_info.status {
        anyhow::bail!("invalid invite; pool is pending");
    }

    let node_address = vec![context.wallet.wallet.default_signer().address()];
    let signatures = vec![alloy::primitives::FixedBytes::from(&bytes_array)];
    let call = contracts
        .compute_pool
        .build_join_compute_pool_call(
            pool_id,
            provider_address,
            node_address,
            vec![req.nonce],
            vec![req.expiration],
            signatures,
        )
        .map_err(|e| anyhow::anyhow!("failed to build join compute pool call: {e:?}"))?;

    let provider = &context.provider_wallet.provider;
    match retry_call(call, 3, provider.clone(), None).await {
        Ok(result) => {
            Console::section("WORKER JOINED COMPUTE POOL");
            Console::success(&format!(
                "Successfully registered on chain with tx: {result}"
            ));
            Console::info(
                "Status",
                "Worker is now part of the compute pool and ready to receive tasks",
            );
        }
        Err(err) => {
            anyhow::bail!("failed to join compute pool: {err:?}");
        }
    }

    let heartbeat_endpoint = match req.url {
        InviteRequestUrl::MasterIpPort(ip, port) => {
            format!("http://{ip}:{port}/heartbeat")
        }
        InviteRequestUrl::MasterUrl(url) => format!("{url}/heartbeat"),
    };

    context
        .heartbeat_service
        .start(heartbeat_endpoint)
        .await
        .context("failed to start heartbeat service")?;
    Ok(())
}
