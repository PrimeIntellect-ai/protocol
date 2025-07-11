use anyhow::{bail, Context as _, Result};
use futures::stream::FuturesUnordered;
use futures::FutureExt;
use p2p::{Keypair, Protocols};
use shared::p2p::OutgoingRequest;
use shared::p2p::Service as P2PService;
use shared::web3::wallet::Wallet;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_util::sync::CancellationToken;

pub struct Service {
    inner: P2PService,
    outgoing_message_tx: Sender<OutgoingRequest>,
    invite_rx: Receiver<InviteRequest>,
    get_task_logs_rx: Receiver<GetTaskLogsRequest>,
    restart_task_rx: Receiver<RestartTaskRequest>,
}

impl Service {
    #[allow(clippy::type_complexity)]
    pub fn new(
        keypair: Keypair,
        port: u16,
        cancellation_token: CancellationToken,
        wallet: Wallet,
    ) -> Result<(
        Self,
        Sender<InviteRequest>,
        Sender<GetTaskLogsRequest>,
        Sender<RestartTaskRequest>,
    )> {
        let (invite_tx, invite_rx) = tokio::sync::mpsc::channel(100);
        let (get_task_logs_tx, get_task_logs_rx) = tokio::sync::mpsc::channel(100);
        let (restart_task_tx, restart_task_rx) = tokio::sync::mpsc::channel(100);
        let (inner, outgoing_message_tx) = P2PService::new(
            keypair,
            port,
            cancellation_token.clone(),
            wallet,
            Protocols::new()
                .with_invite()
                .with_get_task_logs()
                .with_restart()
                .with_authentication(),
        )
        .context("failed to create p2p service")?;
        Ok((
            Self {
                inner,
                outgoing_message_tx,
                invite_rx,
                get_task_logs_rx,
                restart_task_rx,
            },
            invite_tx,
            get_task_logs_tx,
            restart_task_tx,
        ))
    }

    pub async fn run(self) -> Result<()> {
        use futures::StreamExt as _;

        let Self {
            inner,
            outgoing_message_tx,
            mut invite_rx,
            mut get_task_logs_rx,
            mut restart_task_rx,
        } = self;

        tokio::task::spawn(inner.run());

        let mut futures = FuturesUnordered::new();

        loop {
            tokio::select! {
                Some(request) = invite_rx.recv() => {
                    let (incoming_resp_tx, incoming_resp_rx) = tokio::sync::oneshot::channel();
                    let fut = async move {
                        let p2p::Response::Invite(resp) = incoming_resp_rx.await.context("outgoing request tx channel was dropped")? else {
                           bail!("unexpected response type for invite request");
                        };
                        request.response_tx.send(resp).map_err(|_|anyhow::anyhow!("caller dropped response channel"))?;
                        Ok(())
                    }.boxed();
                    futures.push(fut);

                    let outgoing_request = OutgoingRequest {
                        peer_wallet_address: request.worker_wallet_address,
                        peer_id: request.worker_p2p_id,
                        multiaddrs: request.worker_addresses,
                        request: request.invite.into(),
                        response_tx: incoming_resp_tx,
                    };
                    outgoing_message_tx.send(outgoing_request).await
                        .context("failed to send outgoing invite request")?;
                }
                Some(request) = get_task_logs_rx.recv() => {
                    let (incoming_resp_tx, incoming_resp_rx) = tokio::sync::oneshot::channel();
                    let fut = async move {
                        let p2p::Response::GetTaskLogs(resp) = incoming_resp_rx.await.context("outgoing request tx channel was dropped")? else {
                            bail!("unexpected response type for get task logs request");
                        };
                        request.response_tx.send(resp).map_err(|_|anyhow::anyhow!("caller dropped response channel"))?;
                        Ok(())
                    }.boxed();
                    futures.push(fut);

                    let outgoing_request = OutgoingRequest {
                        peer_wallet_address: request.worker_wallet_address,
                        peer_id: request.worker_p2p_id,
                        multiaddrs: request.worker_addresses,
                        request: p2p::Request::GetTaskLogs,
                        response_tx: incoming_resp_tx,
                    };
                    outgoing_message_tx.send(outgoing_request).await
                        .context("failed to send outgoing get task logs request")?;
                }
                Some(request) = restart_task_rx.recv() => {
                    let (incoming_resp_tx, incoming_resp_rx) = tokio::sync::oneshot::channel();
                    let fut = async move {
                        let p2p::Response::RestartTask(resp) = incoming_resp_rx.await.context("outgoing request tx channel was dropped")? else {
                            bail!("unexpected response type for restart task request");
                        };
                        request.response_tx.send(resp).map_err(|_|anyhow::anyhow!("caller dropped response channel"))?;
                        Ok(())
                    }.boxed();
                    futures.push(fut);

                    let outgoing_request = OutgoingRequest {
                        peer_wallet_address: request.worker_wallet_address,
                        peer_id: request.worker_p2p_id,
                        multiaddrs: request.worker_addresses,
                        request: p2p::Request::RestartTask,
                        response_tx: incoming_resp_tx,
                    };
                    outgoing_message_tx.send(outgoing_request).await
                        .context("failed to send outgoing restart task request")?;
                }
                Some(res) = futures.next() => {
                    if let Err(e) = res {
                        log::error!("failed to handle response conversion: {e}");
                    }
                }
            }
        }
    }
}

pub struct InviteRequest {
    pub(crate) worker_wallet_address: alloy::primitives::Address,
    pub(crate) worker_p2p_id: String,
    pub(crate) worker_addresses: Vec<String>,
    pub(crate) invite: p2p::InviteRequest,
    pub(crate) response_tx: tokio::sync::oneshot::Sender<p2p::InviteResponse>,
}

pub struct GetTaskLogsRequest {
    pub(crate) worker_wallet_address: alloy::primitives::Address,
    pub(crate) worker_p2p_id: String,
    pub(crate) worker_addresses: Vec<String>,
    pub(crate) response_tx: tokio::sync::oneshot::Sender<p2p::GetTaskLogsResponse>,
}

pub struct RestartTaskRequest {
    pub(crate) worker_wallet_address: alloy::primitives::Address,
    pub(crate) worker_p2p_id: String,
    pub(crate) worker_addresses: Vec<String>,
    pub(crate) response_tx: tokio::sync::oneshot::Sender<p2p::RestartTaskResponse>,
}
