use crate::{HardwareValidator, MetricsContext, SyntheticDataValidator};
use alloy::primitives::{utils::Unit, Address, U256};
use anyhow::{bail, Context as _, Result};
use futures::stream::FuturesUnordered;
use futures::StreamExt as _;
use log::{error, info, warn};
use shared::models::node::NodeWithMetadata;
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::wallet::WalletProvider;
use std::collections::HashSet;
use std::str::FromStr as _;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

pub struct ValidatorHealth {
    last_validation_timestamp: u64,
    last_loop_duration_ms: u64,
}

impl ValidatorHealth {
    fn new() -> Self {
        Self {
            last_validation_timestamp: 0,
            last_loop_duration_ms: 0,
        }
    }

    fn update(&mut self, timestamp: u64, duration_ms: u64) {
        self.last_validation_timestamp = timestamp;
        self.last_loop_duration_ms = duration_ms;
    }

    pub fn last_validation_timestamp(&self) -> u64 {
        self.last_validation_timestamp
    }

    pub fn last_loop_duration_ms(&self) -> u64 {
        self.last_loop_duration_ms
    }
}

pub struct Validator {
    synthetic_validator: Option<SyntheticDataValidator<WalletProvider>>, // TOOD: does this need to be optional?
    provider: WalletProvider,
    contracts: Contracts<WalletProvider>,
    hardware_validator: HardwareValidator,
    cancellation_token: tokio_util::sync::CancellationToken,
    kademlia_action_tx: tokio::sync::mpsc::Sender<p2p::KademliaActionWithChannel>,
    disable_hardware_validation: bool,
    metrics_ctx: MetricsContext,
    validator_health: Arc<Mutex<ValidatorHealth>>,
}

impl Validator {
    pub fn new(
        cancellation_token: tokio_util::sync::CancellationToken,
        provider: WalletProvider,
        contracts: Contracts<WalletProvider>,
        hardware_validator: HardwareValidator,
        synthetic_validator: Option<SyntheticDataValidator<WalletProvider>>,
        kademlia_action_tx: tokio::sync::mpsc::Sender<p2p::KademliaActionWithChannel>,
        disable_hardware_validation: bool,
        metrics_ctx: MetricsContext,
    ) -> Result<(Self, Arc<Mutex<ValidatorHealth>>)> {
        if contracts.stake_manager.is_none() {
            bail!("stake manager contract not initialized");
        };

        let validator_health = Arc::new(Mutex::new(ValidatorHealth::new()));

        Ok((
            Self {
                cancellation_token,
                provider,
                contracts,
                hardware_validator,
                synthetic_validator,
                kademlia_action_tx,
                disable_hardware_validation,
                metrics_ctx,
                validator_health: validator_health.clone(),
            },
            validator_health,
        ))
    }

    pub async fn run(self) {
        let Self {
            cancellation_token,
            provider,
            contracts,
            hardware_validator,
            synthetic_validator,
            kademlia_action_tx,
            disable_hardware_validation,
            metrics_ctx,
            validator_health,
        } = self;

        let stake_manager = contracts
            .stake_manager
            .as_ref()
            .expect("stake manager contract must be initialized");

        loop {
            let sleep = tokio::time::sleep(std::time::Duration::from_secs(5));
            tokio::select! {
                _ = cancellation_token.cancelled() => {
                    info!("Validator is stopping due to cancellation signal");
                    break;
                }
                _ = sleep => {
                    info!("Validator is starting validation loop");
                    if let Err(e) = perform_validation(
                        synthetic_validator.clone(),
                        provider.clone(),
                        contracts.clone(),
                        hardware_validator.clone(),
                        stake_manager.clone(),
                        kademlia_action_tx.clone(),
                        disable_hardware_validation,
                        metrics_ctx.clone(),
                        validator_health.clone(),
                    ).await {
                        error!("Validation loop failed: {e:#}");
                    } else {
                        info!("Validation loop completed successfully");
                    }
                }
            }
        }
    }
}

async fn perform_validation(
    synthetic_validator: Option<SyntheticDataValidator<WalletProvider>>,
    provider: WalletProvider,
    contracts: Contracts<WalletProvider>,
    hardware_validator: HardwareValidator,
    stake_manager: shared::web3::contracts::implementations::stake_manager::StakeManagerContract<
        WalletProvider,
    >,
    kademlia_action_tx: tokio::sync::mpsc::Sender<p2p::KademliaActionWithChannel>,
    disable_hardware_validation: bool,
    metrics_ctx: MetricsContext,
    validator_health: Arc<Mutex<ValidatorHealth>>,
) -> Result<()> {
    // Start timing the loop
    let loop_start = Instant::now();

    // Update the last validation timestamp
    let last_validation_timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("current time must be after unix epoch")
        .as_secs();

    if let Some(validator) = synthetic_validator.clone() {
        if let Err(e) = validator.validate_work().await {
            error!("Failed to validate work: {e}");
        }
    }

    if !disable_hardware_validation {
        let nodes = get_worker_nodes_from_dht(kademlia_action_tx.clone())
            .await
            .context("failed to fetch nodes from DHT")?;

        if nodes.is_empty() {
            info!("No worker nodes found in DHT, skipping hardware validation");
            return Ok(());
        }

        let futures = FuturesUnordered::new();
        for node in nodes {
            futures.push(NodeWithMetadata::new_from_contracts(
                node, &provider, &contracts,
            ));
        }
        let nodes = futures
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<NodeWithMetadata>>();
        if nodes.is_empty() {
            info!("No valid nodes found for hardware validation");
            return Ok(());
        }

        // Ensure nodes have enough stake
        let mut nodes_with_enough_stake = Vec::new();
        let mut provider_stake_cache: std::collections::HashMap<Address, (U256, U256)> =
            std::collections::HashMap::new();

        for node in nodes {
            let provider_address = Address::from_str(&node.node().provider_address).expect(
                "provider address must be valid, as it was checked in `NodeWithMetadata::new`",
            );

            let (stake, required_stake) =
                if let Some(&cached_info) = provider_stake_cache.get(&provider_address) {
                    cached_info
                } else {
                    let stake = stake_manager
                        .get_stake(provider_address)
                        .await
                        .unwrap_or_default();
                    let total_compute = contracts
                        .compute_registry
                        .get_provider_total_compute(provider_address)
                        .await
                        .unwrap_or_default();
                    let required_stake = stake_manager
                        .calculate_stake(U256::from(0), total_compute)
                        .await
                        .unwrap_or_default();

                    provider_stake_cache.insert(provider_address, (stake, required_stake));
                    (stake, required_stake)
                };

            if stake >= required_stake {
                nodes_with_enough_stake.push(node);
            } else {
                info!(
                    "Node {} has insufficient stake: {} (required: {})",
                    node.node().id,
                    stake / Unit::ETHER.wei(),
                    required_stake / Unit::ETHER.wei()
                );
            }
        }

        if let Err(e) = hardware_validator
            .validate_nodes(nodes_with_enough_stake)
            .await
        {
            error!("Error validating nodes: {e:#}");
        }
    }

    // Calculate and store loop duration
    let last_loop_duration_ms = loop_start.elapsed().as_millis();
    metrics_ctx.record_validation_loop_duration(loop_start.elapsed().as_secs_f64());
    info!("Validation loop completed in {last_loop_duration_ms}ms");

    let mut validator_health = validator_health.lock().await;
    validator_health.update(last_validation_timestamp, last_loop_duration_ms as u64);
    Ok(())
}

async fn get_worker_nodes_from_dht(
    kademlia_action_tx: tokio::sync::mpsc::Sender<p2p::KademliaActionWithChannel>,
) -> Result<Vec<shared::models::node::Node>, anyhow::Error> {
    let (kad_action, mut result_rx) =
        p2p::KademliaAction::GetProviders(p2p::WORKER_DHT_KEY.as_bytes().to_vec())
            .into_kademlia_action_with_channel();
    if let Err(e) = kademlia_action_tx.send(kad_action).await {
        bail!("failed to send Kademlia action: {e}");
    }

    info!("🔄 Fetching worker nodes from DHT...");
    let mut workers = HashSet::new();
    while let Some(result) = result_rx.recv().await {
        match result {
            Ok(res) => {
                match res {
                    p2p::KademliaQueryResult::GetProviders(res) => match res {
                        Ok(res) => match res {
                            p2p::KademliaGetProvidersOk::FoundProviders { key: _, providers } => {
                                workers.extend(providers.into_iter());
                            }
                            _ => {}
                        },
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

    let mut nodes = Vec::new();
    for peer_id in workers {
        let record_key = format!("{}:{}", p2p::WORKER_DHT_KEY, peer_id);
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
                            Ok(res) => match res {
                                p2p::KademliaGetRecordOk::FoundRecord(record) => {
                                    match serde_json::from_slice::<shared::models::node::Node>(
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
                                _ => {}
                            },
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
                    warn!("Kademlia action failed: {e}");
                }
            }
        }
    }

    Ok(nodes)
}
