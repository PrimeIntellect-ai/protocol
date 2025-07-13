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
}

#[pymethods]
impl OrchestratorClient {
    #[new]
    #[pyo3(signature = (rpc_url, private_key=None))]
    pub fn new(rpc_url: String, private_key: Option<String>) -> PyResult<Self> {
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
            Some(
                Wallet::new(&key, rpc_url_parsed)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?,
            )
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
        })
    }

    /// Initialize the orchestrator client with optional P2P support
    #[pyo3(signature = (p2p_port=None))]
    pub fn start(&mut self, py: Python, p2p_port: Option<u16>) -> PyResult<()> {
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
            sender_address: None,
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

    pub fn list_validated_nodes(&self) -> PyResult<Vec<String>> {
        // TODO: Implement orchestrator node listing
        Ok(vec![])
    }

    pub fn list_nodes_from_chain(&self) -> PyResult<Vec<String>> {
        // TODO: Implement orchestrator node listing from chain
        Ok(vec![])
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
