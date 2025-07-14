use anyhow::Context as _;
use anyhow::Result;
use libp2p::autonat;
use libp2p::connection_limits;
use libp2p::connection_limits::ConnectionLimits;
use libp2p::identify;
use libp2p::identity;
use libp2p::kad::store::MemoryStore;
use libp2p::kad::{self, QueryId};
use libp2p::mdns;
use libp2p::ping;
use libp2p::request_response;
use libp2p::swarm::NetworkBehaviour;
use log::debug;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use crate::discovery::OngoingKademliaQuery;
use crate::message::IncomingMessage;
use crate::message::{Request, Response};
use crate::Protocols;
use crate::PRIME_STREAM_PROTOCOL;

const DEFAULT_MAX_PEER_COUNT: u32 = 100;

#[derive(NetworkBehaviour)]
#[behaviour(to_swarm = "BehaviourEvent")]
pub(crate) struct Behaviour {
    // connection gating
    connection_limits: connection_limits::Behaviour,

    // discovery
    mdns: mdns::tokio::Behaviour,
    kademlia: kad::Behaviour<MemoryStore>,

    // protocols
    identify: identify::Behaviour,
    ping: ping::Behaviour,
    request_response: request_response::cbor::Behaviour<Request, Response>,

    // nat traversal
    autonat: autonat::Behaviour,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub(crate) enum BehaviourEvent {
    Autonat(autonat::Event),
    Identify(identify::Event),
    Kademlia(kad::Event),
    Mdns(mdns::Event),
    Ping(ping::Event),
    RequestResponse(request_response::Event<Request, Response>),
}

impl From<void::Void> for BehaviourEvent {
    fn from(_: void::Void) -> Self {
        unreachable!("void::Void cannot be converted to BehaviourEvent")
    }
}

impl From<autonat::Event> for BehaviourEvent {
    fn from(event: autonat::Event) -> Self {
        BehaviourEvent::Autonat(event)
    }
}

impl From<kad::Event> for BehaviourEvent {
    fn from(event: kad::Event) -> Self {
        BehaviourEvent::Kademlia(event)
    }
}

impl From<libp2p::mdns::Event> for BehaviourEvent {
    fn from(event: libp2p::mdns::Event) -> Self {
        BehaviourEvent::Mdns(event)
    }
}

impl From<ping::Event> for BehaviourEvent {
    fn from(event: ping::Event) -> Self {
        BehaviourEvent::Ping(event)
    }
}

impl From<identify::Event> for BehaviourEvent {
    fn from(event: identify::Event) -> Self {
        BehaviourEvent::Identify(event)
    }
}

impl From<request_response::Event<Request, Response>> for BehaviourEvent {
    fn from(event: request_response::Event<Request, Response>) -> Self {
        BehaviourEvent::RequestResponse(event)
    }
}

impl Behaviour {
    pub(crate) fn new(
        keypair: &identity::Keypair,
        protocols: Protocols,
        agent_version: String,
    ) -> Result<Self> {
        let peer_id = keypair.public().to_peer_id();

        let protocols = protocols.into_iter().map(|protocol| {
            (
                protocol.as_stream_protocol(),
                request_response::ProtocolSupport::Full, // TODO: configure inbound/outbound based on node role and protocol
            )
        });

        let autonat = autonat::Behaviour::new(peer_id, autonat::Config::default());
        let connection_limits = connection_limits::Behaviour::new(
            ConnectionLimits::default().with_max_established(Some(DEFAULT_MAX_PEER_COUNT)),
        );

        let mdns = mdns::tokio::Behaviour::new(mdns::Config::default(), peer_id)
            .context("failed to create mDNS behaviour")?;
        let kademlia = kad::Behaviour::new(peer_id, MemoryStore::new(peer_id));

        let identify = identify::Behaviour::new(
            identify::Config::new(PRIME_STREAM_PROTOCOL.to_string(), keypair.public())
                .with_agent_version(agent_version),
        );
        let ping = ping::Behaviour::new(ping::Config::new().with_interval(Duration::from_secs(10)));

        Ok(Self {
            autonat,
            connection_limits,
            kademlia,
            mdns,
            identify,
            ping,
            request_response: request_response::cbor::Behaviour::new(
                protocols,
                request_response::Config::default(),
            ),
        })
    }

    pub(crate) fn request_response(
        &mut self,
    ) -> &mut request_response::cbor::Behaviour<Request, Response> {
        &mut self.request_response
    }

    pub(crate) fn kademlia(&mut self) -> &mut kad::Behaviour<MemoryStore> {
        &mut self.kademlia
    }
}

impl BehaviourEvent {
    pub(crate) async fn handle(
        self,
        message_tx: tokio::sync::mpsc::Sender<IncomingMessage>,
        ongoing_kademlia_queries: Arc<Mutex<HashMap<QueryId, OngoingKademliaQuery>>>,
    ) {
        match self {
            BehaviourEvent::Autonat(_event) => {}
            BehaviourEvent::Identify(_event) => {}
            BehaviourEvent::Kademlia(event) => {
                match event {
                    kad::Event::OutboundQueryProgressed {
                        id,
                        result,
                        stats: _,
                        step,
                    } => {
                        debug!("kademlia query {id:?} progressed with step {step:?} and result {result:?}");

                        let mut ongoing_queries = ongoing_kademlia_queries.lock().await;
                        if let Some(query) = ongoing_queries.get_mut(&id) {
                            let _ = query.result_tx.send(Ok(result)).await;
                        }

                        if step.last {
                            ongoing_queries.remove(&id);
                        }
                    }
                    _ => {}
                }
            }
            BehaviourEvent::Mdns(_event) => {}
            BehaviourEvent::Ping(_event) => {}
            BehaviourEvent::RequestResponse(event) => match event {
                request_response::Event::Message { peer, message } => {
                    debug!("received message from peer {peer:?}: {message:?}");

                    // if this errors, user dropped their incoming message channel
                    let _ = message_tx.send(IncomingMessage { peer, message }).await;
                }
                request_response::Event::ResponseSent { peer, request_id } => {
                    debug!("response sent to peer {peer:?} for request ID {request_id:?}");
                }
                request_response::Event::InboundFailure {
                    peer,
                    request_id,
                    error,
                } => {
                    debug!(
                        "inbound failure from peer {peer:?} for request ID {request_id:?}: {error}"
                    );
                }
                request_response::Event::OutboundFailure {
                    peer,
                    request_id,
                    error,
                } => {
                    debug!(
                        "outbound failure to peer {peer:?} for request ID {request_id:?}: {error}"
                    );
                }
            },
        }
    }
}
