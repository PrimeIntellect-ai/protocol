use pyo3::prelude::*;

mod client;
mod error;

use client::PrimeProtocolClientCore;

// todo: We need a manager + validator side to send messages

/// Prime Protocol Python client
#[pyclass]
pub struct PrimeProtocolClient {
    inner: PrimeProtocolClientCore,
}

#[pymethods]
impl PrimeProtocolClient {
    #[new]
    #[pyo3(signature = (compute_pool_id, rpc_url, private_key_provider=None, private_key_node=None))]
    pub fn new(
        compute_pool_id: u64,
        rpc_url: String,
        private_key_provider: Option<String>,
        private_key_node: Option<String>,
    ) -> PyResult<Self> {
        // todo: revisit default arguments here that are currently none
        let inner = PrimeProtocolClientCore::new(
            compute_pool_id,
            rpc_url,
            private_key_provider,
            private_key_node,
            None,
            None,
        )
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

        Ok(Self { inner })
    }

    pub fn start(&self) -> PyResult<()> {
        // Create a new runtime for this call
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        // Run the async function
        let result = rt.block_on(self.inner.start_async());

        // Clean shutdown
        rt.shutdown_background();

        result.map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }
}

#[pymodule]
fn primeprotocol(m: &Bound<'_, PyModule>) -> PyResult<()> {
    pyo3_log::init();
    m.add_class::<PrimeProtocolClient>()?;
    Ok(())
}
