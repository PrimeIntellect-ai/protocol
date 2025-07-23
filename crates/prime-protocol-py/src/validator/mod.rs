use crate::common::NodeDetails;
use crate::p2p_handler::auth::AuthenticationManager;
use crate::p2p_handler::message_processor::{MessageProcessor, MessageProcessorConfig};
use crate::p2p_handler::{Message, MessageType, Service as P2PService};
use alloy::primitives::Address;
use p2p::{Keypair, PeerId};
use pyo3::prelude::*;
use pythonize::pythonize;
use shared::models::node::DiscoveryNode;
use shared::security::request_signer::sign_request_with_nonce;
use shared::web3::contracts::core::builder::{ContractBuilder, Contracts};
use shared::web3::wallet::{Wallet, WalletProvider};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use url::Url;

/// Prime Protocol Validator Client - for validating nodes and tasks
#[pyclass]
pub struct ValidatorClient {
    runtime: Option<tokio::runtime::Runtime>,
    wallet: Option<Wallet>,
    discovery_urls: Vec<String>,
    cancellation_token: CancellationToken,
    // P2P fields
    auth_manager: Option<Arc<AuthenticationManager>>,
    outbound_tx: Option<Arc<Mutex<Sender<Message>>>>,
    user_message_rx: Option<Arc<Mutex<Receiver<Message>>>>,
    message_processor_handle: Option<JoinHandle<()>>,
    peer_id: Option<PeerId>,
    contracts: Option<Arc<Contracts<WalletProvider>>>,
}

#[pymethods]
impl ValidatorClient {
    #[new]
    #[pyo3(signature = (rpc_url, private_key, discovery_urls=vec!["http://localhost:8089".to_string()]))]
    pub fn new(
        rpc_url: String,
        private_key: String,
        discovery_urls: Vec<String>,
    ) -> PyResult<Self> {
        let rpc_url_parsed = Url::parse(&rpc_url).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Invalid RPC URL: {}", e))
        })?;
        let wallet = Wallet::new(&private_key, rpc_url_parsed)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        let cancellation_token = CancellationToken::new();

        Ok(Self {
            runtime: Some(runtime),
            wallet: Some(wallet),
            discovery_urls,
            cancellation_token,
            auth_manager: None,
            outbound_tx: None,
            user_message_rx: None,
            message_processor_handle: None,
            peer_id: None,
            contracts: None,
        })
    }

    /// List all nodes that are not validated yet
    pub fn list_non_validated_nodes(&self, py: Python) -> PyResult<Vec<NodeDetails>> {
        let rt = self.get_or_create_runtime()?;
        let wallet = self.wallet.as_ref().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Wallet not initialized")
        })?;

        let discovery_urls = self.discovery_urls.clone();

        // Release the GIL while performing async operations
        py.allow_threads(|| {
            rt.block_on(async {
                self.fetch_non_validated_nodes(&discovery_urls, wallet)
                    .await
            })
        })
    }

    /// List all nodes with their details as Python dictionaries
    pub fn list_all_nodes_dict(&self, py: Python) -> PyResult<Vec<PyObject>> {
        let rt = self.get_or_create_runtime()?;
        let wallet = self.wallet.as_ref().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Wallet not initialized")
        })?;

        let discovery_urls = self.discovery_urls.clone();

        // Release the GIL while performing async operations
        let nodes = py.allow_threads(|| {
            rt.block_on(async { self.fetch_all_nodes(&discovery_urls, wallet).await })
        })?;

        // Convert to Python dictionaries
        let result: Result<Vec<_>, _> =
            nodes.into_iter().map(|node| pythonize(py, &node)).collect();

        let python_objects =
            result.map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

        // Convert Bound<PyAny> to Py<PyAny>
        let py_objects: Vec<PyObject> = python_objects
            .into_iter()
            .map(|bound| bound.into())
            .collect();

        Ok(py_objects)
    }

    /// Get the number of non-validated nodes
    pub fn get_non_validated_count(&self, py: Python) -> PyResult<usize> {
        let nodes = self.list_non_validated_nodes(py)?;
        Ok(nodes.len())
    }

    /// Initialize the validator client with optional P2P support
    #[pyo3(signature = (p2p_port=None))]
    pub fn start(&mut self, py: Python, p2p_port: Option<u16>) -> PyResult<()> {
        // Initialize contracts if not already done
        if self.contracts.is_none() {
            let wallet = self.wallet.as_ref().ok_or_else(|| {
                PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Wallet not initialized")
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

        if let Some(port) = p2p_port {
            // Initialize P2P if port is provided
            let wallet = self
                .wallet
                .as_ref()
                .ok_or_else(|| {
                    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Wallet not initialized")
                })?
                .clone();

            let cancellation_token = self.cancellation_token.clone();

            // Get runtime reference for P2P initialization
            let rt = self.runtime.as_ref().ok_or_else(|| {
                PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Runtime not initialized")
            })?;

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
            sender_address: None,
            is_sender_validator: true, // Validator is sending this message
            is_sender_pool_owner: false,
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

    /// Validate a node on the Prime Network contract
    ///
    /// Args:
    ///     node_address: The node address to validate
    ///     provider_address: The provider's address
    ///
    /// Returns:
    ///     Transaction hash as a string if successful
    pub fn validate_node(
        &self,
        py: Python,
        node_address: String,
        provider_address: String,
    ) -> PyResult<String> {
        let rt = self.get_or_create_runtime()?;

        let contracts = self.contracts.as_ref().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "Contracts not initialized. Call start() first.",
            )
        })?;

        let contracts_clone = contracts.clone();

        // Release the GIL while performing async operations
        py.allow_threads(|| {
            rt.block_on(async {
                // Parse addresses
                let provider_addr =
                    Address::parse_checksummed(&provider_address, None).map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                            "Invalid provider address: {}",
                            e
                        ))
                    })?;

                let node_addr = Address::parse_checksummed(&node_address, None).map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "Invalid node address: {}",
                        e
                    ))
                })?;

                let tx_hash = contracts_clone
                    .prime_network
                    .validate_node(provider_addr, node_addr)
                    .await
                    .map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                            "Failed to validate node: {}",
                            e
                        ))
                    })?;

                Ok(format!("0x{}", hex::encode(tx_hash)))
            })
        })
    }

    /// Validate a node on the Prime Network contract with explicit addresses
    ///
    /// Args:
    ///     provider_address: The provider's address
    ///     node_address: The node's address  
    ///
    /// Returns:
    ///     Transaction hash as a string if successful
    pub fn validate_node_with_addresses(
        &self,
        py: Python,
        provider_address: String,
        node_address: String,
    ) -> PyResult<String> {
        let rt = self.get_or_create_runtime()?;

        let contracts = self.contracts.as_ref().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "Contracts not initialized. Call start() first.",
            )
        })?;

        let contracts_clone = contracts.clone();

        // Release the GIL while performing async operations
        py.allow_threads(|| {
            rt.block_on(async {
                // Parse addresses
                let provider_addr =
                    Address::parse_checksummed(&provider_address, None).map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                            "Invalid provider address: {}",
                            e
                        ))
                    })?;

                let node_addr = Address::parse_checksummed(&node_address, None).map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "Invalid node address: {}",
                        e
                    ))
                })?;

                let tx_hash = contracts_clone
                    .prime_network
                    .validate_node(provider_addr, node_addr)
                    .await
                    .map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                            "Failed to validate node: {}",
                            e
                        ))
                    })?;

                Ok(format!("0x{}", hex::encode(tx_hash)))
            })
        })
    }

    /// Get the validator's peer ID
    pub fn get_peer_id(&self) -> PyResult<Option<String>> {
        Ok(self.peer_id.map(|id| id.to_string()))
    }
}

// Private implementation methods
impl ValidatorClient {
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

        let (p2p_service, outbound_tx, message_queue_rx, authenticated_peers) =
            P2PService::new(keypair, port, cancellation_token.clone(), wallet_address)?;

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
            cancellation_token,
            validator_addresses: Arc::new(std::collections::HashSet::new()), // Validator doesn't need to check other validators
            pool_owner_address: None, // Validator doesn't need pool owner info
            compute_manager_address: None, // Validator doesn't need this
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

    async fn fetch_non_validated_nodes(
        &self,
        discovery_urls: &[String],
        wallet: &Wallet,
    ) -> PyResult<Vec<NodeDetails>> {
        let nodes = self.fetch_all_nodes(discovery_urls, wallet).await?;
        Ok(nodes
            .into_iter()
            .filter(|node| !node.is_validated)
            .map(NodeDetails::from)
            .collect())
    }

    async fn fetch_all_nodes(
        &self,
        discovery_urls: &[String],
        wallet: &Wallet,
    ) -> PyResult<Vec<DiscoveryNode>> {
        let mut all_nodes = Vec::new();
        let mut any_success = false;

        for discovery_url in discovery_urls {
            match self
                .fetch_nodes_from_discovery_url(discovery_url, wallet)
                .await
            {
                Ok(nodes) => {
                    all_nodes.extend(nodes);
                    any_success = true;
                }
                Err(e) => {
                    // Log error but continue with other discovery services
                    log::error!("Failed to fetch nodes from {}: {}", discovery_url, e);
                }
            }
        }

        if !any_success {
            return Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "Failed to fetch nodes from all discovery services",
            ));
        }

        // Remove duplicates based on node ID
        let mut unique_nodes = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();
        for node in all_nodes {
            if seen_ids.insert(node.node.id.clone()) {
                unique_nodes.push(node);
            }
        }

        Ok(unique_nodes)
    }

    async fn fetch_nodes_from_discovery_url(
        &self,
        discovery_url: &str,
        wallet: &Wallet,
    ) -> Result<Vec<DiscoveryNode>, anyhow::Error> {
        let address = wallet.wallet.default_signer().address().to_string();

        let discovery_route = "/api/validator";
        let signature = sign_request_with_nonce(discovery_route, wallet, None)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "x-address",
            reqwest::header::HeaderValue::from_str(&address)?,
        );
        headers.insert(
            "x-signature",
            reqwest::header::HeaderValue::from_str(&signature.signature)?,
        );

        let response = reqwest::Client::new()
            .get(format!("{}{}", discovery_url, discovery_route))
            .query(&[("nonce", signature.nonce)])
            .headers(headers)
            .timeout(Duration::from_secs(10))
            .send()
            .await?;

        let response_text = response.text().await?;
        let parsed_response: shared::models::api::ApiResponse<Vec<DiscoveryNode>> =
            serde_json::from_str(&response_text)?;

        if !parsed_response.success {
            return Err(anyhow::anyhow!(
                "Failed to fetch nodes from {}: {:?}",
                discovery_url,
                parsed_response
            ));
        }

        Ok(parsed_response.data)
    }
}
