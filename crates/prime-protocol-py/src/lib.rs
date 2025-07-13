use crate::orchestrator::OrchestratorClient;
use crate::validator::ValidatorClient;
use crate::worker::WorkerClient;
use pyo3::prelude::*;

mod constants;
mod error;
mod orchestrator;
mod p2p_handler;
mod utils;
mod validator;
mod worker;

#[pymodule]
fn primeprotocol(m: &Bound<'_, PyModule>) -> PyResult<()> {
    pyo3_log::init();
    m.add_class::<WorkerClient>()?;
    m.add_class::<OrchestratorClient>()?;
    m.add_class::<ValidatorClient>()?;
    Ok(())
}
