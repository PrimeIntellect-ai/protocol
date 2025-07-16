use anyhow::Context;
use anyhow::Result;
use libp2p::kad::QueryId;
use libp2p::noise;
use libp2p::swarm::SwarmEvent;
use libp2p::tcp;
use libp2p::yamux;
use libp2p::Swarm;
use libp2p::SwarmBuilder;
use libp2p::{identity, Transport};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

mod behaviour;
mod discovery;
mod message;
mod protocol;

use behaviour::Behaviour;

pub use discovery::*;
pub use message::*;
pub use protocol::*;

pub type Libp2pIncomingMessage = libp2p::request_response::Message<Request, Response>;
pub type ResponseChannel = libp2p::request_response::ResponseChannel<Response>;
pub type PeerId = libp2p::PeerId;
pub type Multiaddr = libp2p::Multiaddr;
pub type Keypair = libp2p::identity::Keypair;
pub type KademliaQueryResult = libp2p::kad::QueryResult;
pub type KademliaGetProvidersOk = libp2p::kad::GetProvidersOk;
pub type KademliaGetRecordOk = libp2p::kad::GetRecordOk;

type MultiaddrProtocol<'a> = libp2p::multiaddr::Protocol<'a>;

pub const PRIME_STREAM_PROTOCOL: libp2p::StreamProtocol =
    libp2p::StreamProtocol::new("/prime/1.0.0");
// TODO: force this to be passed by the user
pub const DEFAULT_AGENT_VERSION: &str = "prime-node/0.1.0";

pub struct Node {
    peer_id: PeerId,
    listen_addrs: Vec<libp2p::Multiaddr>,
    swarm: Swarm<Behaviour>,
    bootnodes: Vec<Multiaddr>,
    cancellation_token: tokio_util::sync::CancellationToken,

    // channel for sending incoming messages to the consumer of this library
    incoming_message_tx: tokio::sync::mpsc::Sender<IncomingMessage>,

    // channel for receiving outgoing messages from the consumer of this library
    outgoing_message_rx: tokio::sync::mpsc::Receiver<OutgoingMessage>,

    // channel for receiving kademlia actions from the consumer of this library
    kademlia_action_rx: tokio::sync::mpsc::Receiver<discovery::KademliaActionWithChannel>,
    ongoing_kademlia_queries: Arc<Mutex<HashMap<QueryId, OngoingKademliaQuery>>>,
}

impl Node {
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    pub fn listen_addrs(&self) -> &[libp2p::Multiaddr] {
        &self.listen_addrs
    }

    /// Returns the multiaddresses that this node is listening on, with the peer ID included.
    pub fn multiaddrs(&self) -> Vec<libp2p::Multiaddr> {
        self.listen_addrs
            .iter()
            .map(|addr| {
                addr.clone()
                    .with_p2p(self.peer_id)
                    .expect("can add peer ID to multiaddr")
            })
            .collect()
    }

    pub async fn run(self) -> Result<()> {
        use libp2p::futures::StreamExt as _;

        let Node {
            peer_id: _,
            listen_addrs,
            mut swarm,
            bootnodes,
            cancellation_token,
            incoming_message_tx,
            mut outgoing_message_rx,
            mut kademlia_action_rx,
            ongoing_kademlia_queries,
        } = self;

        for addr in listen_addrs {
            swarm
                .listen_on(addr)
                .context("swarm failed to listen on multiaddr")?;
        }

        for mut multiaddr in bootnodes {
            let Some(MultiaddrProtocol::P2p(peer_id)) = multiaddr.pop() else {
                warn!("bootnode {multiaddr} does not have a peer ID, skipping");
                continue;
            };

            swarm
                .behaviour_mut()
                .kademlia()
                .add_address(&peer_id, multiaddr.clone());

            log::debug!("dialing bootnode {peer_id} at {multiaddr}");

            match swarm.dial(multiaddr.clone()) {
                Ok(()) => {}
                Err(e) => {
                    debug!("failed to dial bootnode {multiaddr}: {e:?}");
                }
            }
        }

        // this will only error if we have no known peers
        let (bootstrap_result_tx, mut bootstrap_result_rx) = tokio::sync::mpsc::channel(100);
        match swarm.behaviour_mut().kademlia().bootstrap() {
            Ok(query_id) => {
                let mut ongoing_kademlia_queries = ongoing_kademlia_queries.lock().await;
                ongoing_kademlia_queries.insert(
                    query_id,
                    OngoingKademliaQuery {
                        result_tx: bootstrap_result_tx,
                    },
                );
            }
            Err(e) => {
                warn!("failed to bootstrap kademlia: {e:?}");
            }
        };

        let connected_peers_check_interval = Duration::from_secs(60);

        loop {
            let sleep = tokio::time::sleep(connected_peers_check_interval);
            tokio::select! {
                biased;
                _ = sleep => {
                    let peer_count = swarm.connected_peers().count();
                    info!("connected peers: {peer_count}");
                }
                _ = cancellation_token.cancelled() => {
                    debug!("cancellation token triggered, shutting down node");
                    break Ok(());
                }
                Some(res) = bootstrap_result_rx.recv() => {
                    match res {
                        Ok(libp2p::kad::QueryResult::Bootstrap(_)) => {
                            debug!("kademlia bootstrap progressed successfully");
                        }
                        Ok(res) => {
                            warn!("kademlia bootstrap query returned unexpected result: {res:?}");
                        }
                        Err(e) => {
                            warn!("kademlia bootstrap query failed: {e:?}");
                        }
                    }
                }
                Some(message) = outgoing_message_rx.recv() => {
                    match message {
                        OutgoingMessage::Request((peer, addrs, request)) => {
                            for addr in addrs {
                                swarm.add_peer_address(peer, addr);
                            }
                            swarm.behaviour_mut().request_response().send_request(&peer, request);
                        }
                        OutgoingMessage::Response((channel, response)) => {
                            if let Err(e) = swarm.behaviour_mut().request_response().send_response(channel, response) {
                                debug!("failed to send response: {e:?}");
                            }
                        }
                    }
                }
                Some(kademlia_action) = kademlia_action_rx.recv() => {
                    let result_tx = kademlia_action.result_tx();
                    if let Err(e) = discovery::handle_kademlia_action(swarm.behaviour_mut().kademlia(), kademlia_action, ongoing_kademlia_queries.clone()).await {
                        debug!("failed to handle kademlia action: {e:?}");
                        let _ = result_tx.send(Err(e)).await;
                    }
                }
                event = swarm.select_next_some() => {
                    match event {
                        SwarmEvent::NewListenAddr {
                            address,
                            ..
                        } => {
                            debug!("new listen address: {address}");
                        }
                        SwarmEvent::ExternalAddrConfirmed { address } => {
                            debug!("external address confirmed: {address}");
                        }
                        SwarmEvent::ConnectionEstablished {
                            peer_id,
                            ..
                        } => {
                            debug!("connection established with peer {peer_id}");
                        }
                        SwarmEvent::ConnectionClosed {
                            peer_id,
                            cause,
                            ..
                        } => {
                            debug!("connection closed with peer {peer_id}: {cause:?}");
                        }
                        SwarmEvent::Behaviour(event) => {
                            let discovered_peers = event.handle(incoming_message_tx.clone(), ongoing_kademlia_queries.clone()).await;
                            for (peer_id, addr) in discovered_peers {
                                swarm.add_peer_address(peer_id, addr.clone());
                                swarm.behaviour_mut().kademlia().add_address(&peer_id, addr.clone());
                            }
                        }
                        _ => continue,
                    }
                },
            }
        }
    }
}

pub struct NodeBuilder {
    port: Option<u16>,
    listen_addrs: Vec<libp2p::Multiaddr>,
    keypair: Option<identity::Keypair>,
    agent_version: Option<String>,
    protocols: Protocols,
    bootnodes: Vec<Multiaddr>,
    cancellation_token: Option<tokio_util::sync::CancellationToken>,
}

impl Default for NodeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeBuilder {
    pub fn new() -> Self {
        Self {
            port: None,
            listen_addrs: Vec::new(),
            keypair: None,
            agent_version: None,
            protocols: Protocols::new(),
            bootnodes: Vec::new(),
            cancellation_token: None,
        }
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    pub fn with_listen_addr(mut self, addr: libp2p::Multiaddr) -> Self {
        self.listen_addrs.push(addr);
        self
    }

    pub fn with_keypair(mut self, keypair: identity::Keypair) -> Self {
        self.keypair = Some(keypair);
        self
    }

    pub fn with_agent_version(mut self, agent_version: String) -> Self {
        self.agent_version = Some(agent_version);
        self
    }

    pub fn with_authentication(mut self) -> Self {
        self.protocols = self.protocols.with_authentication();
        self
    }

    pub fn with_hardware_challenge(mut self) -> Self {
        self.protocols = self.protocols.with_hardware_challenge();
        self
    }

    pub fn with_invite(mut self) -> Self {
        self.protocols = self.protocols.with_invite();
        self
    }

    pub fn with_get_task_logs(mut self) -> Self {
        self.protocols = self.protocols.with_get_task_logs();
        self
    }

    pub fn with_restart(mut self) -> Self {
        self.protocols = self.protocols.with_restart();
        self
    }

    pub fn with_general(mut self) -> Self {
        self.protocols = self.protocols.with_general();
        self
    }

    pub fn with_protocols(mut self, protocols: Protocols) -> Self {
        self.protocols.join(protocols);
        self
    }

    pub fn with_bootnode(mut self, bootnode: Multiaddr) -> Self {
        self.bootnodes.push(bootnode);
        self
    }

    pub fn with_bootnodes<I, T>(mut self, bootnodes: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<Multiaddr>,
    {
        for bootnode in bootnodes {
            self.bootnodes.push(bootnode.into());
        }
        self
    }

    pub fn with_cancellation_token(
        mut self,
        cancellation_token: tokio_util::sync::CancellationToken,
    ) -> Self {
        self.cancellation_token = Some(cancellation_token);
        self
    }

    #[allow(clippy::type_complexity)]
    pub fn try_build(
        self,
    ) -> Result<(
        Node,
        tokio::sync::mpsc::Receiver<IncomingMessage>,
        tokio::sync::mpsc::Sender<OutgoingMessage>,
        tokio::sync::mpsc::Sender<discovery::KademliaActionWithChannel>,
    )> {
        let Self {
            port,
            mut listen_addrs,
            keypair,
            agent_version,
            protocols,
            bootnodes,
            cancellation_token,
        } = self;

        let keypair = keypair.unwrap_or(identity::Keypair::generate_ed25519());
        let peer_id = keypair.public().to_peer_id();

        let transport = create_transport(&keypair)?;
        let behaviour = Behaviour::new(
            &keypair,
            protocols,
            agent_version.unwrap_or(DEFAULT_AGENT_VERSION.to_string()),
        )
        .context("failed to create behaviour")?;

        let swarm = SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_other_transport(|_| transport)?
            .with_behaviour(|_| behaviour)?
            .with_swarm_config(|cfg| {
                cfg.with_idle_connection_timeout(Duration::from_secs(u64::MAX)) // don't disconnect from idle peers
            })
            .build();

        if listen_addrs.is_empty() {
            let port = port.unwrap_or(0);
            let listen_addr = format!("/ip4/0.0.0.0/tcp/{port}")
                .parse()
                .expect("can parse valid multiaddr");
            listen_addrs.push(listen_addr);
        }

        let (incoming_message_tx, incoming_message_rx) = tokio::sync::mpsc::channel(100);
        let (outgoing_message_tx, outgoing_message_rx) = tokio::sync::mpsc::channel(100);
        let (kademlia_action_tx, kademlia_action_rx) = tokio::sync::mpsc::channel(100);
        let ongoing_kademlia_queries = Arc::new(Mutex::new(HashMap::new()));

        Ok((
            Node {
                peer_id,
                swarm,
                listen_addrs,
                bootnodes,
                incoming_message_tx,
                outgoing_message_rx,
                kademlia_action_rx,
                cancellation_token: cancellation_token.unwrap_or_default(),
                ongoing_kademlia_queries,
            },
            incoming_message_rx,
            outgoing_message_tx,
            kademlia_action_tx,
        ))
    }
}

fn create_transport(
    keypair: &identity::Keypair,
) -> Result<libp2p::core::transport::Boxed<(PeerId, libp2p::core::muxing::StreamMuxerBox)>> {
    let transport = tcp::tokio::Transport::new(tcp::Config::default())
        .upgrade(libp2p::core::upgrade::Version::V1)
        .authenticate(noise::Config::new(keypair)?)
        .multiplex(yamux::Config::default())
        .timeout(Duration::from_secs(20))
        .boxed();

    Ok(transport)
}

#[cfg(test)]
mod test {
    use libp2p::kad;
    use libp2p::kad::GetProvidersOk;
    use std::collections::HashSet;

    use super::NodeBuilder;
    use crate::message;
    use crate::KademliaAction;

    #[tokio::test]
    async fn two_nodes_can_connect_and_do_request_response() {
        let (node1, mut incoming_message_rx1, outgoing_message_tx1, _) =
            NodeBuilder::new().with_get_task_logs().try_build().unwrap();
        let node1_peer_id = node1.peer_id();

        let (node2, mut incoming_message_rx2, outgoing_message_tx2, _) = NodeBuilder::new()
            .with_get_task_logs()
            .with_bootnodes(node1.multiaddrs())
            .try_build()
            .unwrap();
        let node2_peer_id = node2.peer_id();

        tokio::spawn(async move { node1.run().await });
        tokio::spawn(async move { node2.run().await });

        // TODO: implement a way to get peer count (https://github.com/PrimeIntellect-ai/protocol/issues/628)
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // send request from node1->node2
        let request = message::Request::GetTaskLogs;
        outgoing_message_tx1
            .send(request.into_outgoing_message(node2_peer_id, vec![]))
            .await
            .unwrap();
        let message = incoming_message_rx2.recv().await.unwrap();
        assert_eq!(message.peer, node1_peer_id);
        let libp2p::request_response::Message::Request {
            request_id: _,
            request: message::Request::GetTaskLogs,
            channel,
        } = message.message
        else {
            panic!("expected a GetTaskLogs request message");
        };

        // send response from node2->node1
        let response =
            message::Response::GetTaskLogs(message::GetTaskLogsResponse::Ok("logs".to_string()));
        outgoing_message_tx2
            .send(response.into_outgoing_message(channel))
            .await
            .unwrap();
        let message = incoming_message_rx1.recv().await.unwrap();
        assert_eq!(message.peer, node2_peer_id);
        let libp2p::request_response::Message::Response {
            request_id: _,
            response: message::Response::GetTaskLogs(response),
        } = message.message
        else {
            panic!("expected a GetTaskLogs response message");
        };
        let message::GetTaskLogsResponse::Ok(logs) = response else {
            panic!("expected a successful GetTaskLogs response");
        };
        assert_eq!(logs, "logs");
    }

    #[tokio::test]
    async fn kademlia_get_providers_ok() {
        let (node1, _, _, _) = NodeBuilder::new().with_get_task_logs().try_build().unwrap();

        let (node2, _, _, kademlia_action_tx_2) = NodeBuilder::new()
            .with_get_task_logs()
            .with_bootnodes(node1.multiaddrs())
            .try_build()
            .unwrap();

        let (node3, _, _, kademlia_action_tx_3) = NodeBuilder::new()
            .with_get_task_logs()
            .with_bootnodes(node1.multiaddrs())
            .try_build()
            .unwrap();
        let node3_peer_id = node3.peer_id();

        tokio::spawn(async move { node1.run().await });
        tokio::spawn(async move { node2.run().await });
        tokio::spawn(async move { node3.run().await });

        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        let test_key = b"test_key".to_vec();
        let action = KademliaAction::StartProviding(test_key.clone());
        let (action, mut rx) = action.into_kademlia_action_with_channel();
        kademlia_action_tx_3.send(action).await.unwrap();

        let result = rx.recv().await.unwrap().unwrap();
        let kad::QueryResult::StartProviding(res) = result else {
            panic!("expected a QueryResult::StartProviding response");
        };
        res.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        let action = KademliaAction::GetProviders(test_key.clone());
        let (action, mut rx) = action.into_kademlia_action_with_channel();
        kademlia_action_tx_2.send(action).await.unwrap();

        let mut providers_set: HashSet<String> = HashSet::new();
        while let Some(res) = rx.recv().await {
            match res {
                Ok(kad::QueryResult::GetProviders(res)) => {
                    let ok = res.unwrap();
                    match ok {
                        GetProvidersOk::FoundProviders { key, providers } => {
                            assert_eq!(key, test_key.clone().into());
                            providers_set.insert(providers.iter().map(|p| p.to_string()).collect());
                        }
                        _ => {}
                    }
                }
                Ok(_) => panic!("expected a QueryResult::GetProviders response"),
                Err(e) => panic!("unexpected error: {e}"),
            }
        }
        assert!(!providers_set.is_empty(), "expected at least one provider");
        assert!(
            providers_set
                .iter()
                .any(|s| s.contains(&node3_peer_id.to_string())),
            "expected node3 to be a provider"
        );
    }
}
