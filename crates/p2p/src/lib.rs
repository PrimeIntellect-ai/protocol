use anyhow::Context;
use anyhow::Result;
use libp2p::futures::stream::FuturesUnordered;
use libp2p::noise;
use libp2p::swarm::SwarmEvent;
use libp2p::tcp;
use libp2p::yamux;
use libp2p::Multiaddr;
use libp2p::Swarm;
use libp2p::SwarmBuilder;
use libp2p::{identity, PeerId, Transport};
use std::time::Duration;

mod behaviour;
mod message;
mod protocol;

use behaviour::Behaviour;
use message::{IncomingMessage, OutgoingMessage};
use protocol::Protocols;

pub const PRIME_STREAM_PROTOCOL: libp2p::StreamProtocol =
    libp2p::StreamProtocol::new("/prime/1.0.0");
// TODO: force this to be passed by the user
pub const DEFAULT_AGENT_VERSION: &str = "prime-node/0.1.0";

pub struct Node {
    peer_id: PeerId,
    listen_addrs: Vec<libp2p::Multiaddr>,
    swarm: Swarm<Behaviour>,
    bootnodes: Vec<Multiaddr>,

    // channel for sending incoming messages to the consumer of this library
    incoming_message_tx: tokio::sync::mpsc::Sender<IncomingMessage>,

    // channel for receiving outgoing messages from the consumer of this library
    outgoing_message_rx: tokio::sync::mpsc::Receiver<OutgoingMessage>,
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
            incoming_message_tx,
            mut outgoing_message_rx,
        } = self;

        for addr in listen_addrs {
            swarm
                .listen_on(addr)
                .context("swarm failed to listen on multiaddr")?;
        }

        let futures = FuturesUnordered::new();
        for bootnode in bootnodes {
            futures.push(swarm.dial(bootnode))
        }
        let results: Vec<_> = futures.into_iter().collect();
        for result in results {
            match result {
                Ok(_) => {}
                Err(e) => {
                    // TODO: log this error
                    println!("failed to dial bootnode: {e:?}");
                }
            }
        }

        loop {
            tokio::select! {
                Some(message) = outgoing_message_rx.recv() => {
                    match message {
                        OutgoingMessage::Request((peer, request)) => {
                            swarm.behaviour_mut().request_response().send_request(&peer, request);
                        }
                        OutgoingMessage::Response((channel, response)) => {
                            println!("sending response on channel");
                            if let Err(e) = swarm.behaviour_mut().request_response().send_response(channel, response) {
                                // log error
                                println!("failed to send response: {e:?}");
                            }
                        }
                    }
                }
                event = swarm.select_next_some() => {
                    match event {
                        SwarmEvent::NewListenAddr {
                            listener_id: _,
                            address,
                        } => {
                            println!("new listen address: {address}");
                        }
                        SwarmEvent::ExternalAddrConfirmed { address } => {
                            println!("external address confirmed: {address}");
                        }
                        SwarmEvent::ConnectionClosed {
                            peer_id,
                            cause,
                            endpoint: _,
                            connection_id: _,
                            num_established: _,
                        } => {
                            println!("connection closed with peer {peer_id}: {cause:?}");
                        }
                        SwarmEvent::Behaviour(event) => event.handle(incoming_message_tx.clone()).await,
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

    pub fn with_validator_authentication(mut self) -> Self {
        self.protocols = self.protocols.with_validator_authentication();
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

    pub fn try_build(
        self,
    ) -> Result<(
        Node,
        tokio::sync::mpsc::Receiver<IncomingMessage>,
        tokio::sync::mpsc::Sender<OutgoingMessage>,
    )> {
        let Self {
            port,
            mut listen_addrs,
            keypair,
            agent_version,
            protocols,
            bootnodes,
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

        Ok((
            Node {
                peer_id,
                swarm,
                listen_addrs,
                bootnodes,
                incoming_message_tx,
                outgoing_message_rx,
            },
            incoming_message_rx,
            outgoing_message_tx,
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
    use super::NodeBuilder;
    use crate::message;

    #[tokio::test]
    async fn two_nodes_can_connect_and_do_request_response() {
        let (node1, mut incoming_message_rx1, outgoing_message_tx1) =
            NodeBuilder::new().with_get_task_logs().try_build().unwrap();
        let node1_peer_id = node1.peer_id();

        let (node2, mut incoming_message_rx2, outgoing_message_tx2) = NodeBuilder::new()
            .with_get_task_logs()
            .with_bootnodes(node1.multiaddrs())
            .try_build()
            .unwrap();
        let node2_peer_id = node2.peer_id();

        tokio::spawn(async move { node1.run().await });
        tokio::spawn(async move { node2.run().await });

        // TODO: implement a way to get peer count
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // send request from node1->node2
        let request = message::Request::GetTaskLogs;
        outgoing_message_tx1
            .send(request.into_outgoing_message(node2_peer_id))
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

        println!("received request from node1");

        // send response from node2->node1
        let response = message::Response::GetTaskLogs(message::GetTaskLogsResponse {
            logs: Ok(vec!["log1".to_string(), "log2".to_string()]),
        });
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
        assert_eq!(
            response.logs,
            Ok(vec!["log1".to_string(), "log2".to_string()])
        );
    }
}
