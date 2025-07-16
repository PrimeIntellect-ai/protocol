use anyhow::{bail, Result};
use log::{info, warn};

/// Given a kademlia action channel backed by a `p2p::Service`,
/// perform a DHT lookup for all nodes which claim to be a worker using `GetProviders` for `p2p::WORKER_DHT_KEY`.
/// then, for each of these nodes, query the DHT for their record (which stores their node information) using `GetRecord`.
pub async fn get_worker_nodes_from_dht(
    kademlia_action_tx: tokio::sync::mpsc::Sender<p2p::KademliaActionWithChannel>,
) -> Result<Vec<crate::models::Node>> {
    let (kad_action, mut result_rx) =
        p2p::KademliaAction::GetProviders(p2p::WORKER_DHT_KEY.as_bytes().to_vec())
            .into_kademlia_action_with_channel();
    if let Err(e) = kademlia_action_tx.send(kad_action).await {
        bail!("failed to send Kademlia action: {e}");
    }

    info!("🔄 Fetching worker nodes from DHT...");
    let mut workers = std::collections::HashSet::new();
    while let Some(result) = result_rx.recv().await {
        match result {
            Ok(res) => {
                match res {
                    p2p::KademliaQueryResult::GetProviders(res) => match res {
                        Ok(res) => {
                            if let p2p::KademliaGetProvidersOk::FoundProviders {
                                key: _,
                                providers,
                            } = res
                            {
                                workers.extend(providers.into_iter());
                            }
                        }
                        Err(e) => {
                            bail!("failed to get providers from DHT: {e}");
                        }
                    },
                    _ => {
                        // this case should never happen
                        bail!("unexpected Kademlia query result: {res:?}");
                    }
                }
            }
            Err(e) => {
                bail!("kademlia action failed: {e}");
            }
        }
    }

    log::debug!("got {} worker nodes from DHT", workers.len());

    let mut nodes = Vec::new();
    for peer_id in workers {
        let record_key = p2p::worker_dht_key_with_peer_id(&peer_id);
        let (kad_action, mut result_rx) =
            p2p::KademliaAction::GetRecord(record_key.as_bytes().to_vec())
                .into_kademlia_action_with_channel();
        if let Err(e) = kademlia_action_tx.send(kad_action).await {
            bail!("failed to send Kademlia action: {e}");
        }

        while let Some(result) = result_rx.recv().await {
            match result {
                Ok(res) => {
                    match res {
                        p2p::KademliaQueryResult::GetRecord(res) => match res {
                            Ok(res) => {
                                if let p2p::KademliaGetRecordOk::FoundRecord(record) = res {
                                    match serde_json::from_slice::<crate::models::Node>(
                                        &record.record.value,
                                    ) {
                                        Ok(node) => {
                                            nodes.push(node);
                                        }
                                        Err(e) => {
                                            warn!("failed to deserialize node record: {e}");
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("failed to get record from DHT: {e}");
                            }
                        },
                        _ => {
                            // this case should never happen
                            bail!("unexpected Kademlia query result: {res:?}");
                        }
                    }
                }
                Err(e) => {
                    warn!("kademlia action failed: {e}");
                }
            }
        }
    }

    Ok(nodes)
}
