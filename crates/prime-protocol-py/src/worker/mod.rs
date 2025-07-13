use pyo3::prelude::*;

mod auth;
mod blockchain;
mod client;
mod constants;
mod message_processor;
mod p2p;

pub(crate) use client::WorkerClientCore;
use tokio_util::sync::CancellationToken;

use crate::worker::p2p::Message;
use constants::DEFAULT_P2P_PORT;

/// Prime Protocol Worker Client - for compute nodes that execute tasks
#[pyclass]
pub(crate) struct WorkerClient {
    inner: WorkerClientCore,
    runtime: Option<tokio::runtime::Runtime>,
    cancellation_token: CancellationToken,
}

#[pymethods]
impl WorkerClient {
    #[new]
    #[pyo3(signature = (compute_pool_id, rpc_url, private_key_provider=None, private_key_node=None, p2p_port=DEFAULT_P2P_PORT))]
    pub fn new(
        compute_pool_id: u64,
        rpc_url: String,
        private_key_provider: Option<String>,
        private_key_node: Option<String>,
        p2p_port: u16,
    ) -> PyResult<Self> {
        let cancellation_token = CancellationToken::new();
        let inner = WorkerClientCore::new(
            compute_pool_id,
            rpc_url,
            private_key_provider,
            private_key_node,
            None,
            None,
            cancellation_token.clone(),
            p2p_port,
        )
        .map_err(to_py_err)?;

        Ok(Self {
            inner,
            runtime: None,
            cancellation_token,
        })
    }

    /// Start the worker client
    pub fn start(&mut self, py: Python) -> PyResult<()> {
        if self.runtime.is_some() {
            return Err(to_py_runtime_err("Client already started"));
        }

        let rt = create_runtime()?;
        let result = py.allow_threads(|| rt.block_on(self.inner.start_async()));

        self.runtime = Some(rt);
        result.map_err(to_py_err)
    }

    /// Get the next message from the P2P network
    pub fn get_next_message(&self, py: Python) -> PyResult<Option<PyObject>> {
        let rt = self.ensure_runtime()?;

        Ok(py.allow_threads(|| {
            rt.block_on(self.inner.get_next_message())
                .map(message_to_pyobject)
        }))
    }

    /// Send a message to a peer
    pub fn send_message(
        &self,
        peer_id: String,
        data: Vec<u8>,
        multiaddrs: Vec<String>,
        py: Python,
    ) -> PyResult<()> {
        let rt = self.ensure_runtime()?;

        let message = Message {
            message_type: p2p::MessageType::General { data },
            peer_id,
            multiaddrs,
            sender_address: None, // Will be filled from our wallet automatically
            response_tx: None,
        };

        py.allow_threads(|| rt.block_on(self.inner.send_message(message)))
            .map_err(to_py_err)
    }

    /// Get this node's peer ID
    pub fn get_own_peer_id(&self) -> PyResult<Option<String>> {
        self.ensure_runtime()?;
        Ok(self.inner.get_peer_id().map(|id| id.to_string()))
    }

    /// Stop the worker client and clean up resources
    pub fn stop(&mut self, py: Python) -> PyResult<()> {
        // Cancel all background tasks
        self.cancellation_token.cancel();

        if let Some(rt) = self.runtime.as_ref() {
            let inner = &mut self.inner;
            py.allow_threads(|| rt.block_on(inner.stop_async()))
                .map_err(to_py_err)?;
        }

        // Clean up the runtime
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_background();
        }

        Ok(())
    }
}

// Helper methods
impl WorkerClient {
    fn ensure_runtime(&self) -> PyResult<&tokio::runtime::Runtime> {
        self.runtime
            .as_ref()
            .ok_or_else(|| to_py_runtime_err("Client not started. Call start() first."))
    }
}

// Utility functions
fn create_runtime() -> PyResult<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| to_py_runtime_err(&format!("Failed to create runtime: {}", e)))
}

fn to_py_err(err: impl std::fmt::Display) -> PyErr {
    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(err.to_string())
}

fn to_py_runtime_err(msg: &str) -> PyErr {
    PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(msg.to_string())
}

fn message_to_pyobject(message: Message) -> PyObject {
    let message_data = match message.message_type {
        p2p::MessageType::General { data } => {
            serde_json::json!({
                "type": "general",
                "data": data,
            })
        }
        p2p::MessageType::AuthenticationInitiation { challenge } => {
            serde_json::json!({
                "type": "auth_initiation",
                "challenge": challenge,
            })
        }
        p2p::MessageType::AuthenticationResponse {
            challenge,
            signature,
        } => {
            serde_json::json!({
                "type": "auth_response",
                "challenge": challenge,
                "signature": signature,
            })
        }
        p2p::MessageType::AuthenticationSolution { signature } => {
            serde_json::json!({
                "type": "auth_solution",
                "signature": signature,
            })
        }
    };

    let json_value = serde_json::json!({
        "message": message_data,
        "peer_id": message.peer_id,
        "multiaddrs": message.multiaddrs,
        "sender_address": message.sender_address,
    });

    Python::with_gil(|py| crate::utils::json_parser::json_to_pyobject(py, &json_value))
}
