use crate::common::NodeDetails;
use crate::p2p_handler::auth::AuthenticationManager;
use crate::p2p_handler::message_processor::{MessageProcessor, MessageProcessorConfig};
use crate::p2p_handler::{Message, MessageType, Service as P2PService};
use p2p::{Keypair, PeerId};
use pyo3::prelude::*;
use pythonize::pythonize;
use shared::web3::wallet::Wallet;
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use url::Url;

// Add new imports for discovery functionality
use shared::discovery::fetch_nodes_from_discovery_urls;

// Add imports for invite functionality
use alloy::primitives::Address;
use prime_core::invite::{
    admin::{generate_invite_expiration, generate_invite_nonce, generate_invite_signature},
    common::InviteBuilder,
};
use std::str::FromStr;

// Add imports for compute pool contract functionality
use shared::web3::contracts::core::builder::{ContractBuilder, Contracts};
use shared::web3::wallet::WalletProvider;

/// Prime Protocol Orchestrator Client - for managing and distributing tasks
#[pyclass]
pub struct OrchestratorClient {
    runtime: Option<tokio::runtime::Runtime>,
    wallet: Option<Wallet>,
    cancellation_token: CancellationToken,
    // P2P fields
    auth_manager: Option<Arc<AuthenticationManager>>,
    outbound_tx: Option<Arc<Mutex<Sender<Message>>>>,
    user_message_rx: Option<Arc<Mutex<Receiver<Message>>>>,
    message_processor_handle: Option<JoinHandle<()>>,
    peer_id: Option<PeerId>,
    // Discovery service URLs
    discovery_urls: Vec<String>,
    // Contracts
    contracts: Option<Arc<Contracts<WalletProvider>>>,
}

#[pymethods]
impl OrchestratorClient {
    #[new]
    #[pyo3(signature = (rpc_url, private_key=None, discovery_urls=vec!["http://localhost:8089".to_string()]))]
    pub fn new(
        rpc_url: String,
        private_key: Option<String>,
        discovery_urls: Vec<String>,
    ) -> PyResult<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        let cancellation_token = CancellationToken::new();

        // Create wallet if private key is provided
        let wallet = if let Some(key) = private_key {
            let rpc_url_parsed = Url::parse(&rpc_url).map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Invalid RPC URL: {}", e))
            })?;
            let w = Wallet::new(&key, rpc_url_parsed)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

            let wallet_address = w.wallet.default_signer().address().to_string();
            log::info!("Orchestrator wallet address: {}", wallet_address);

            Some(w)
        } else {
            None
        };

        Ok(Self {
            runtime: Some(runtime),
            wallet,
            cancellation_token,
            auth_manager: None,
            outbound_tx: None,
            user_message_rx: None,
            message_processor_handle: None,
            peer_id: None,
            discovery_urls,
            contracts: None,
        })
    }

    /// Initialize the orchestrator client with optional P2P support
    #[pyo3(signature = (p2p_port=None))]
    pub fn start(&mut self, py: Python, p2p_port: Option<u16>) -> PyResult<()> {
        // Initialize contracts if not already done
        if self.contracts.is_none() {
            let wallet = self.wallet.as_ref().ok_or_else(|| {
                PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                    "Wallet not initialized. Provide private_key when creating client.",
                )
            })?;

            let wallet_provider = wallet.provider();

            // Get runtime reference and use it before mutating self
            let rt = self.runtime.as_ref().ok_or_else(|| {
                PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Runtime not initialized")
            })?;

            let contracts = py.allow_threads(|| {
                rt.block_on(async {
                    // Build all contracts (required due to known bug)
                    ContractBuilder::new(wallet_provider)
                        .with_compute_pool()
                        .with_compute_registry()
                        .with_ai_token()
                        .with_prime_network()
                        .with_stake_manager()
                        .build()
                        .map_err(|e| {
                            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                                "Failed to build contracts: {}",
                                e
                            ))
                        })
                })
            })?;

            self.contracts = Some(Arc::new(contracts));
        }

        let rt = self.get_or_create_runtime()?;

        if let Some(port) = p2p_port {
            // Initialize P2P if port is provided
            let wallet = self
                .wallet
                .as_ref()
                .ok_or_else(|| {
                    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                        "Wallet not initialized. Provide private_key when creating client.",
                    )
                })?
                .clone();

            let cancellation_token = self.cancellation_token.clone();

            // Create the P2P components
            let (auth_manager, peer_id, outbound_tx, user_message_rx, message_processor_handle) =
                py.allow_threads(|| {
                    rt.block_on(async {
                        Self::create_p2p_components(wallet, port, cancellation_token)
                            .await
                            .map_err(|e| {
                                PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string())
                            })
                    })
                })?;

            // Update self with the created components
            self.auth_manager = Some(auth_manager);
            self.peer_id = Some(peer_id);
            self.outbound_tx = Some(outbound_tx);
            self.user_message_rx = Some(user_message_rx);
            self.message_processor_handle = Some(message_processor_handle);
        }

        Ok(())
    }

    /// Send a message to a peer
    pub fn send_message(
        &self,
        py: Python,
        peer_id: String,
        multiaddrs: Vec<String>,
        data: Vec<u8>,
    ) -> PyResult<()> {
        let rt = self.get_or_create_runtime()?;

        let auth_manager = self.auth_manager.as_ref().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "P2P not initialized. Call start() with p2p_port parameter.",
            )
        })?;

        let outbound_tx = self.outbound_tx.as_ref().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "P2P not initialized. Call start() with p2p_port parameter.",
            )
        })?;

        let message = Message {
            message_type: MessageType::General { data },
            peer_id,
            multiaddrs,
            sender_address: self
                .wallet
                .as_ref()
                .map(|w| w.wallet.default_signer().address().to_string()),
            is_sender_validator: false,
            is_sender_pool_owner: false, // This will be determined by the receiver
            response_tx: None,
        };

        py.allow_threads(|| {
            rt.block_on(async {
                crate::p2p_handler::send_message_with_auth(message, auth_manager, outbound_tx)
                    .await
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
            })
        })
    }

    /// Get the next message from the P2P network
    pub fn get_next_message(&self, py: Python) -> PyResult<Option<PyObject>> {
        let rt = self.get_or_create_runtime()?;

        let user_message_rx = self.user_message_rx.as_ref().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "P2P not initialized. Call start() with p2p_port parameter.",
            )
        })?;

        let message = py.allow_threads(|| {
            rt.block_on(async {
                tokio::time::timeout(
                    crate::constants::MESSAGE_QUEUE_TIMEOUT,
                    user_message_rx.lock().await.recv(),
                )
                .await
                .ok()
                .flatten()
            })
        });

        match message {
            Some(msg) => {
                let py_msg = pythonize(py, &msg)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
                Ok(Some(py_msg.into()))
            }
            None => Ok(None),
        }
    }

    /// Get the orchestrator's peer ID
    pub fn get_peer_id(&self) -> PyResult<Option<String>> {
        Ok(self.peer_id.map(|id| id.to_string()))
    }

    /// Get the orchestrator's wallet address
    pub fn get_wallet_address(&self) -> PyResult<Option<String>> {
        Ok(self
            .wallet
            .as_ref()
            .map(|w| w.wallet.default_signer().address().to_string()))
    }

    /// Stop the orchestrator client
    pub fn stop(&mut self, _py: Python) -> PyResult<()> {
        self.cancellation_token.cancel();

        // Stop message processor
        if let Some(handle) = self.message_processor_handle.take() {
            handle.abort();
        }

        Ok(())
    }

    /// List nodes for a specific compute pool
    pub fn list_nodes_for_pool(&self, py: Python, pool_id: u32) -> PyResult<Vec<NodeDetails>> {
        let rt = self.get_or_create_runtime()?;

        let wallet = self.wallet.as_ref().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "Wallet not initialized. Provide private_key when creating client.",
            )
        })?;

        let discovery_urls = self.discovery_urls.clone();
        let route = format!("/api/pool/{}", pool_id);

        let nodes = py.allow_threads(|| {
            rt.block_on(async {
                fetch_nodes_from_discovery_urls(&discovery_urls, &route, wallet)
                    .await
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
            })
        })?;

        Ok(nodes.into_iter().map(NodeDetails::from).collect())
    }

    /// Invite a node to join a compute pool
    ///
    /// This method creates a signed invite and sends it to the specified worker node.
    /// The invite contains pool information and authentication details that the worker
    /// can validate before joining the pool.
    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (peer_id, worker_address, pool_id, multiaddrs, domain_id=1, orchestrator_url=None, expiration_seconds=1000))]
    pub fn invite_node(
        &self,
        py: Python,
        peer_id: String,
        worker_address: String,
        pool_id: u32,
        multiaddrs: Vec<String>,
        domain_id: u32,
        orchestrator_url: Option<String>,
        expiration_seconds: u64,
    ) -> PyResult<()> {
        let rt = self.get_or_create_runtime()?;

        let wallet = self.wallet.as_ref().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "Wallet not initialized. Provide private_key when creating client.",
            )
        })?;

        let outbound_tx = self.outbound_tx.as_ref().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "P2P not initialized. Call start() with p2p_port parameter.",
            )
        })?;

        let auth_manager = self.auth_manager.as_ref().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "P2P not initialized. Call start() with p2p_port parameter.",
            )
        })?;

        // Parse worker address
        let worker_addr = Address::from_str(&worker_address).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                "Invalid worker address: {}",
                e
            ))
        })?;

        let wallet = wallet.clone();
        let outbound_tx = outbound_tx.clone();
        let auth_manager = auth_manager.clone();

        py.allow_threads(|| {
            rt.block_on(async {
                // Generate invite parameters
                let nonce = generate_invite_nonce();
                let expiration =
                    generate_invite_expiration(Some(expiration_seconds)).map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string())
                    })?;

                // Generate the invite signature
                let invite_signature = generate_invite_signature(
                    &wallet,
                    domain_id,
                    pool_id,
                    worker_addr,
                    nonce,
                    expiration,
                )
                .await
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

                // Build the invite request
                let invite_builder = if let Some(url) = orchestrator_url {
                    InviteBuilder::with_url(pool_id, url)
                } else {
                    // Default to localhost if no URL provided
                    InviteBuilder::with_url(pool_id, "http://localhost:8080".to_string())
                };

                let invite_request = invite_builder
                    .build(invite_signature, nonce, expiration)
                    .map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string())
                    })?;

                // Serialize the invite request
                let invite_data = serde_json::to_vec(&invite_request).map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string())
                })?;

                // Create a general message with the invite data
                let message = Message {
                    message_type: MessageType::General { data: invite_data },
                    peer_id,
                    multiaddrs,
                    sender_address: Some(wallet.wallet.default_signer().address().to_string()),
                    is_sender_validator: false,
                    is_sender_pool_owner: false, // This will be determined by the receiver
                    response_tx: None,
                };

                // Send the invite
                crate::p2p_handler::send_message_with_auth(message, &auth_manager, &outbound_tx)
                    .await
                    .map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string())
                    })?;

                Ok(())
            })
        })
    }

    /// Eject a node from a compute pool
    ///
    /// This method removes a node from the specified compute pool. Only the pool's
    /// compute manager can eject nodes from their pool.
    ///
    /// Args:
    ///     pool_id: The ID of the compute pool
    ///     node_address: The address of the node to eject
    ///     
    /// Returns:
    ///     The transaction hash as a hex string
    pub fn eject_node(&self, py: Python, pool_id: u32, node_address: String) -> PyResult<String> {
        let rt = self.get_or_create_runtime()?;

        let contracts = self.contracts.as_ref().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "Contracts not initialized. Call start() first.",
            )
        })?;

        // Parse node address
        let node_addr = Address::from_str(&node_address).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Invalid node address: {}", e))
        })?;

        let contracts_clone = contracts.clone();

        py.allow_threads(|| {
            rt.block_on(async {
                // Call eject_node on the contract
                let tx_hash = contracts_clone
                    .compute_pool
                    .eject_node(pool_id, node_addr)
                    .await
                    .map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                            "Failed to eject node: {}",
                            e
                        ))
                    })?;

                // Convert transaction hash to hex string
                Ok(format!("0x{}", tx_hash))
            })
        })
    }

    /// Blacklist a node from a compute pool
    ///
    /// This method blacklists a node from the specified compute pool, preventing it from
    /// rejoining. Only the pool's compute manager can blacklist nodes in their pool.
    ///
    /// Args:
    ///     pool_id: The ID of the compute pool
    ///     node_address: The address of the node to blacklist
    ///     
    /// Returns:
    ///     The transaction hash as a hex string
    pub fn blacklist_node(
        &self,
        py: Python,
        pool_id: u32,
        node_address: String,
    ) -> PyResult<String> {
        let rt = self.get_or_create_runtime()?;

        let contracts = self.contracts.as_ref().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "Contracts not initialized. Call start() first.",
            )
        })?;

        // Parse node address
        let node_addr = Address::from_str(&node_address).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Invalid node address: {}", e))
        })?;

        let contracts_clone = contracts.clone();

        py.allow_threads(|| {
            rt.block_on(async {
                // Call blacklist_node on the contract
                let tx_hash = contracts_clone
                    .compute_pool
                    .blacklist_node(pool_id, node_addr)
                    .await
                    .map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                            "Failed to blacklist node: {}",
                            e
                        ))
                    })?;

                // Convert transaction hash to hex string
                Ok(format!("0x{}", tx_hash))
            })
        })
    }

    /// Check if a node is blacklisted from a compute pool
    ///
    /// Args:
    ///     pool_id: The ID of the compute pool
    ///     node_address: The address of the node to check
    ///     
    /// Returns:
    ///     True if the node is blacklisted, False otherwise
    pub fn is_node_blacklisted(
        &self,
        py: Python,
        pool_id: u32,
        node_address: String,
    ) -> PyResult<bool> {
        let rt = self.get_or_create_runtime()?;

        let contracts = self.contracts.as_ref().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "Contracts not initialized. Call start() first.",
            )
        })?;

        // Parse node address
        let node_addr = Address::from_str(&node_address).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Invalid node address: {}", e))
        })?;

        let contracts_clone = contracts.clone();

        py.allow_threads(|| {
            rt.block_on(async {
                // Check if node is blacklisted
                contracts_clone
                    .compute_pool
                    .is_node_blacklisted(pool_id, node_addr)
                    .await
                    .map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                            "Failed to check blacklist status: {}",
                            e
                        ))
                    })
            })
        })
    }

    /// Get all blacklisted nodes for a compute pool
    ///
    /// Args:
    ///     pool_id: The ID of the compute pool
    ///     
    /// Returns:
    ///     List of blacklisted node addresses
    pub fn get_blacklisted_nodes(&self, py: Python, pool_id: u32) -> PyResult<Vec<String>> {
        let rt = self.get_or_create_runtime()?;

        let contracts = self.contracts.as_ref().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "Contracts not initialized. Call start() first.",
            )
        })?;

        let contracts_clone = contracts.clone();

        py.allow_threads(|| {
            rt.block_on(async {
                // Get blacklisted nodes
                let nodes = contracts_clone
                    .compute_pool
                    .get_blacklisted_nodes(pool_id)
                    .await
                    .map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                            "Failed to get blacklisted nodes: {}",
                            e
                        ))
                    })?;

                // Convert addresses to strings
                Ok(nodes.iter().map(|addr| addr.to_string()).collect())
            })
        })
    }

    /// Check if a node is in a compute pool
    ///
    /// Args:
    ///     pool_id: The ID of the compute pool
    ///     node_address: The address of the node to check
    ///     
    /// Returns:
    ///     True if the node is in the pool, False otherwise
    pub fn is_node_in_pool(
        &self,
        py: Python,
        pool_id: u32,
        node_address: String,
    ) -> PyResult<bool> {
        let rt = self.get_or_create_runtime()?;

        let contracts = self.contracts.as_ref().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "Contracts not initialized. Call start() first.",
            )
        })?;

        // Parse node address
        let node_addr = Address::from_str(&node_address).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Invalid node address: {}", e))
        })?;

        let contracts_clone = contracts.clone();

        py.allow_threads(|| {
            rt.block_on(async {
                // Check if node is in pool
                contracts_clone
                    .compute_pool
                    .is_node_in_pool(pool_id, node_addr)
                    .await
                    .map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                            "Failed to check if node is in pool: {}",
                            e
                        ))
                    })
            })
        })
    }
}

// Private implementation methods
impl OrchestratorClient {
    fn get_or_create_runtime(&self) -> PyResult<&tokio::runtime::Runtime> {
        if let Some(ref rt) = self.runtime {
            Ok(rt)
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "Runtime not initialized. Call start() first.",
            ))
        }
    }

    async fn create_p2p_components(
        wallet: Wallet,
        port: u16,
        cancellation_token: CancellationToken,
    ) -> Result<
        (
            Arc<AuthenticationManager>,
            PeerId,
            Arc<Mutex<Sender<Message>>>,
            Arc<Mutex<Receiver<Message>>>,
            JoinHandle<()>,
        ),
        anyhow::Error,
    > {
        // Initialize authentication manager
        let auth_manager = Arc::new(AuthenticationManager::new(Arc::new(wallet.clone())));

        // Create P2P service
        let keypair = Keypair::generate_ed25519();
        let wallet_address = Some(wallet.wallet.default_signer().address().to_string());

        let (user_message_tx, user_message_rx) = tokio::sync::mpsc::channel::<Message>(1000);

        let (p2p_service, outbound_tx, message_queue_rx, authenticated_peers) = P2PService::new(
            keypair,
            port,
            cancellation_token.clone(),
            wallet_address.clone(),
        )?;

        let peer_id = p2p_service.node.peer_id();
        let outbound_tx = Arc::new(Mutex::new(outbound_tx));
        let user_message_rx = Arc::new(Mutex::new(user_message_rx));

        // Start P2P service
        tokio::task::spawn(p2p_service.run());

        // Start message processor
        let config = MessageProcessorConfig {
            auth_manager: auth_manager.clone(),
            message_queue_rx: Arc::new(Mutex::new(message_queue_rx)),
            user_message_tx,
            outbound_tx: outbound_tx.clone(),
            authenticated_peers,
            cancellation_token: cancellation_token.clone(),
            validator_addresses: Arc::new(std::collections::HashSet::new()), // Orchestrator doesn't need validator info
            pool_owner_address: wallet_address
                .as_ref()
                .and_then(|addr| alloy::primitives::Address::from_str(addr).ok()), // Parse orchestrator's address
            compute_manager_address: None, // Orchestrator doesn't need this
        };

        let message_processor = MessageProcessor::from_config(config);
        let message_processor_handle = message_processor.spawn();

        log::info!(
            "P2P service started on port {} with peer ID: {:?}",
            port,
            peer_id
        );

        Ok((
            auth_manager,
            peer_id,
            outbound_tx,
            user_message_rx,
            message_processor_handle,
        ))
    }
}
