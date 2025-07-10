use anyhow::{bail, Context as _, Result};
use futures::stream::FuturesUnordered;
use p2p::{Keypair, Protocols};
use shared::p2p::OutgoingRequest;
use shared::p2p::Service as P2PService;
use shared::web3::wallet::Wallet;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_util::sync::CancellationToken;

pub struct Service {
    inner: P2PService,

    // converts incoming hardware challenges to outgoing requests
    outgoing_message_tx: Sender<OutgoingRequest>,
    hardware_challenge_rx: Receiver<HardwareChallengeRequest>,
}

impl Service {
    pub fn new(
        keypair: Keypair,
        port: u16,
        cancellation_token: CancellationToken,
        wallet: Wallet,
    ) -> Result<(Self, Sender<HardwareChallengeRequest>)> {
        let (hardware_challenge_tx, hardware_challenge_rx) = tokio::sync::mpsc::channel(100);
        let (inner, outgoing_message_tx) = P2PService::new(
            keypair,
            port,
            cancellation_token.clone(),
            wallet,
            Protocols::new()
                .with_hardware_challenge()
                .with_validator_authentication(),
        )
        .context("failed to create p2p service")?;
        Ok((
            Self {
                inner,
                outgoing_message_tx,
                hardware_challenge_rx,
            },
            hardware_challenge_tx,
        ))
    }

    pub async fn run(self) -> Result<()> {
        use futures::StreamExt as _;

        let Self {
            inner,
            outgoing_message_tx,
            mut hardware_challenge_rx,
        } = self;

        tokio::task::spawn(inner.run());

        let mut futures = FuturesUnordered::new();

        loop {
            tokio::select! {
                Some(request) = hardware_challenge_rx.recv() => {
                    let (incoming_resp_tx, incoming_resp_rx) = tokio::sync::oneshot::channel();
                    let fut = async move {
                        let resp = match incoming_resp_rx.await.context("outgoing request tx channel was dropped")? {
                            p2p::Response::HardwareChallenge(resp) => resp.response,
                            _ => bail!("unexpected response type for hardware challenge request"),
                        };
                        request.response_tx.send(resp).map_err(|_|anyhow::anyhow!("caller dropped response channel"))?;
                        Ok(())
                    };
                    futures.push(fut);

                    let outgoing_request = OutgoingRequest {
                        peer_wallet_address: request.worker_wallet_address,
                        peer_id: request.worker_p2p_id,
                        multiaddrs: request.worker_addresses,
                        request: p2p::HardwareChallengeRequest {
                            challenge: request.challenge,
                            timestamp: std::time::SystemTime::now(),
                        }.into(),
                        response_tx: incoming_resp_tx,
                    };
                    outgoing_message_tx.send(outgoing_request).await
                        .context("failed to send outgoing hardware challenge request")?;
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

pub struct HardwareChallengeRequest {
    pub(crate) worker_wallet_address: alloy::primitives::Address,
    pub(crate) worker_p2p_id: String,
    pub(crate) worker_addresses: Vec<String>,
    pub(crate) challenge: p2p::ChallengeRequest,
    pub(crate) response_tx: tokio::sync::oneshot::Sender<p2p::ChallengeResponse>,
}
