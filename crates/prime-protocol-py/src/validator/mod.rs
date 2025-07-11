use pyo3::prelude::*;

pub(crate) mod message_queue;
use self::message_queue::MessageQueue;

/// Node details for validator operations
#[pyclass]
#[derive(Clone)]
pub(crate) struct NodeDetails {
    #[pyo3(get)]
    pub address: String,
}

#[pymethods]
impl NodeDetails {
    #[new]
    pub fn new(address: String) -> Self {
        Self { address }
    }
}

/// Prime Protocol Validator Client - for validating task results
#[pyclass]
pub(crate) struct ValidatorClient {
    message_queue: MessageQueue,
    runtime: Option<tokio::runtime::Runtime>,
}

#[pymethods]
impl ValidatorClient {
    #[new]
    #[pyo3(signature = (rpc_url, private_key=None))]
    pub fn new(rpc_url: String, private_key: Option<String>) -> PyResult<Self> {
        // TODO: Implement validator initialization
        let _ = rpc_url;
        let _ = private_key;

        Ok(Self {
            message_queue: MessageQueue::new(),
            runtime: None,
        })
    }

    /// Initialize the validator client and start listening for messages
    pub fn start(&mut self, py: Python) -> PyResult<()> {
        // Create a new runtime for this validator
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        // Store the runtime for future use
        self.runtime = Some(rt);

        Ok(())
    }

    pub fn list_nodes(&self) -> PyResult<Vec<String>> {
        // TODO: Implement validator node listing from chain that are not yet validated
        Ok(vec![])
    }

    pub fn fetch_node_details(&self, node_id: String) -> PyResult<Option<NodeDetails>> {
        // TODO: Implement validator node details fetching
        Ok(None)
    }

    pub fn mark_node_as_validated(&self, node_id: String) -> PyResult<()> {
        // TODO: Implement validator node marking as validated
        Ok(())
    }

    pub fn send_request_to_node(&self, node_id: String, request: String) -> PyResult<()> {
        // TODO: Implement validator node request sending
        Ok(())
    }

    pub fn send_request_to_node_address(
        &self,
        node_address: String,
        request: String,
    ) -> PyResult<()> {
        // TODO: Implement validator node request sending to specific address
        let _ = node_address;
        let _ = request;
        Ok(())
    }

    /// Get the latest validation result from the internal message queue
    pub fn get_latest_message(&self, py: Python) -> PyResult<Option<PyObject>> {
        if let Some(rt) = self.runtime.as_ref() {
            Ok(py.allow_threads(|| rt.block_on(self.message_queue.get_validation_result())))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "Validator not started. Call start() first.".to_string(),
            ))
        }
    }

    /// Get the number of pending validation results
    pub fn get_queue_size(&self, py: Python) -> PyResult<usize> {
        if let Some(rt) = self.runtime.as_ref() {
            Ok(py.allow_threads(|| rt.block_on(self.message_queue.get_queue_size())))
        } else {
            Ok(0)
        }
    }
}
