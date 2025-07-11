use pyo3::prelude::*;

mod error;
mod message_queue;
mod utils;
mod worker;

use worker::WorkerClientCore;

/// Prime Protocol Worker Client - for compute nodes that execute tasks
#[pyclass]
pub struct WorkerClient {
    inner: WorkerClientCore,
    runtime: Option<tokio::runtime::Runtime>,
}

#[pymethods]
impl WorkerClient {
    #[new]
    #[pyo3(signature = (compute_pool_id, rpc_url, private_key_provider=None, private_key_node=None))]
    pub fn new(
        compute_pool_id: u64,
        rpc_url: String,
        private_key_provider: Option<String>,
        private_key_node: Option<String>,
    ) -> PyResult<Self> {
        let inner = WorkerClientCore::new(
            compute_pool_id,
            rpc_url,
            private_key_provider,
            private_key_node,
            None,
            None,
        )
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

        Ok(Self {
            inner,
            runtime: None,
        })
    }

    pub fn start(&mut self) -> PyResult<()> {
        // Create a new runtime for this call
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        // Run the async function
        let result = rt.block_on(self.inner.start_async());
        println!("system start completed");

        // Store the runtime for future use
        self.runtime = Some(rt);

        result.map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    pub fn get_pool_owner_message(&self) -> PyResult<Option<PyObject>> {
        if let Some(rt) = self.runtime.as_ref() {
            Ok(rt.block_on(self.inner.get_message_queue().get_pool_owner_message()))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "Client not started. Call start() first.".to_string(),
            ))
        }
    }

    pub fn get_validator_message(&self) -> PyResult<Option<PyObject>> {
        if let Some(rt) = self.runtime.as_ref() {
            Ok(rt.block_on(self.inner.get_message_queue().get_validator_message()))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "Client not started. Call start() first.".to_string(),
            ))
        }
    }

    pub fn stop(&mut self) -> PyResult<()> {
        if let Some(rt) = self.runtime.as_ref() {
            rt.block_on(self.inner.stop_async())
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
        }

        // Clean up the runtime
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_background();
        }

        Ok(())
    }
}

/// Prime Protocol Orchestrator Client - for managing and distributing tasks
#[pyclass]
pub struct OrchestratorClient {
    // TODO: Implement orchestrator-specific functionality
}

#[pymethods]
impl OrchestratorClient {
    #[new]
    #[pyo3(signature = (rpc_url, private_key=None))]
    pub fn new(rpc_url: String, private_key: Option<String>) -> PyResult<Self> {
        // TODO: Implement orchestrator initialization
        let _ = rpc_url;
        let _ = private_key;
        Ok(Self {})
    }
}

/// Prime Protocol Validator Client - for validating task results
#[pyclass]
pub struct ValidatorClient {
    // TODO: Implement validator-specific functionality
}

#[pymethods]
impl ValidatorClient {
    #[new]
    #[pyo3(signature = (rpc_url, private_key=None))]
    pub fn new(rpc_url: String, private_key: Option<String>) -> PyResult<Self> {
        // TODO: Implement validator initialization
        let _ = rpc_url;
        let _ = private_key;
        Ok(Self {})
    }
}

#[pymodule]
fn primeprotocol(m: &Bound<'_, PyModule>) -> PyResult<()> {
    pyo3_log::init();
    m.add_class::<WorkerClient>()?;
    m.add_class::<OrchestratorClient>()?;
    m.add_class::<ValidatorClient>()?;
    Ok(())
}
