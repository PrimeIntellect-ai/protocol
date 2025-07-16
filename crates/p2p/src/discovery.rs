use anyhow::{Context as _, Result};
use libp2p::kad::QueryResult;
use libp2p::kad::{self, store::RecordStore, QueryId, Quorum};
use libp2p::PeerId;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub const WORKER_DHT_KEY: &str = "prime-worker/1.0.0";

pub fn worker_dht_key_with_peer_id(peer_id: &PeerId) -> String {
    format!("{WORKER_DHT_KEY}/{peer_id}")
}

pub struct KademliaActionWithChannel {
    kad_action: KademliaAction,
    result_tx: tokio::sync::mpsc::Sender<Result<QueryResult>>,
}

impl KademliaActionWithChannel {
    pub fn new(
        kad_action: KademliaAction,
        result_tx: tokio::sync::mpsc::Sender<Result<QueryResult>>,
    ) -> Self {
        Self {
            kad_action,
            result_tx,
        }
    }

    pub(crate) fn result_tx(&self) -> tokio::sync::mpsc::Sender<Result<QueryResult>> {
        self.result_tx.clone()
    }
}

pub enum KademliaAction {
    PutRecord { key: Vec<u8>, value: Vec<u8> },
    GetRecord(Vec<u8>),
    StartProviding(Vec<u8>),
    StopProviding(Vec<u8>),
    GetProviders(Vec<u8>),
}

impl KademliaAction {
    pub fn into_kademlia_action_with_channel(
        self,
    ) -> (
        KademliaActionWithChannel,
        tokio::sync::mpsc::Receiver<Result<QueryResult>>,
    ) {
        let (result_tx, result_rx) = tokio::sync::mpsc::channel(1);
        (
            KademliaActionWithChannel {
                kad_action: self,
                result_tx,
            },
            result_rx,
        )
    }
}

pub(crate) struct OngoingKademliaQuery {
    pub(crate) result_tx: tokio::sync::mpsc::Sender<Result<QueryResult>>,
}

pub(crate) async fn handle_kademlia_action<S: RecordStore + Send + 'static>(
    kademlia: &mut kad::Behaviour<S>,
    action: KademliaActionWithChannel,
    ongoing_kademlia_queries: Arc<Mutex<HashMap<QueryId, OngoingKademliaQuery>>>,
) -> Result<()> {
    match action.kad_action {
        KademliaAction::PutRecord { key, value } => {
            let query_id = kademlia
                .put_record(
                    kad::Record {
                        key: key.into(),
                        value,
                        publisher: None,
                        expires: None,
                    },
                    Quorum::One,
                )
                .context("failed to put record in dht")?;
            let mut ongoing_queries = ongoing_kademlia_queries.lock().await;
            ongoing_queries.insert(
                query_id,
                OngoingKademliaQuery {
                    result_tx: action.result_tx,
                },
            );
        }
        KademliaAction::GetRecord(key) => {
            let query_id = kademlia.get_record(key.into());
            let mut ongoing_queries = ongoing_kademlia_queries.lock().await;
            ongoing_queries.insert(
                query_id,
                OngoingKademliaQuery {
                    result_tx: action.result_tx,
                },
            );
        }
        KademliaAction::StartProviding(key) => {
            let query_id = kademlia
                .start_providing(key.into())
                .context("failed to start providing key in dht")?;
            let mut ongoing_queries = ongoing_kademlia_queries.lock().await;
            ongoing_queries.insert(
                query_id,
                OngoingKademliaQuery {
                    result_tx: action.result_tx,
                },
            );
        }
        KademliaAction::StopProviding(key) => kademlia.stop_providing(&key.into()),
        KademliaAction::GetProviders(key) => {
            let query_id = kademlia.get_providers(key.into());
            let mut ongoing_queries = ongoing_kademlia_queries.lock().await;
            ongoing_queries.insert(
                query_id,
                OngoingKademliaQuery {
                    result_tx: action.result_tx,
                },
            );
        }
    }
    Ok(())
}
