use pyo3::prelude::*;
use shared::models::node::DiscoveryNode;

/// Node details structure shared between validator and orchestrator
#[pyclass]
#[derive(Clone)]
pub struct NodeDetails {
    #[pyo3(get)]
    pub id: String,
    #[pyo3(get)]
    pub provider_address: String,
    #[pyo3(get)]
    pub ip_address: String,
    #[pyo3(get)]
    pub port: u16,
    #[pyo3(get)]
    pub compute_pool_id: u32,
    #[pyo3(get)]
    pub is_validated: bool,
    #[pyo3(get)]
    pub is_active: bool,
    #[pyo3(get)]
    pub is_provider_whitelisted: bool,
    #[pyo3(get)]
    pub is_blacklisted: bool,
    #[pyo3(get)]
    pub worker_p2p_id: Option<String>,
    #[pyo3(get)]
    pub last_updated: Option<String>,
    #[pyo3(get)]
    pub created_at: Option<String>,
}

impl From<DiscoveryNode> for NodeDetails {
    fn from(node: DiscoveryNode) -> Self {
        Self {
            id: node.node.id,
            provider_address: node.node.provider_address,
            ip_address: node.node.ip_address,
            port: node.node.port,
            compute_pool_id: node.node.compute_pool_id,
            is_validated: node.is_validated,
            is_active: node.is_active,
            is_provider_whitelisted: node.is_provider_whitelisted,
            is_blacklisted: node.is_blacklisted,
            worker_p2p_id: node.node.worker_p2p_id,
            last_updated: node.last_updated.map(|dt| dt.to_rfc3339()),
            created_at: node.created_at.map(|dt| dt.to_rfc3339()),
        }
    }
}

#[pymethods]
impl NodeDetails {
    /// Get compute specifications as a Python dictionary
    pub fn get_compute_specs(&self, py: Python) -> PyResult<PyObject> {
        // This method would need access to the original node data
        // For now, return None since we don't store compute specs in NodeDetails
        Ok(py.None())
    }

    /// Get location information as a Python dictionary
    pub fn get_location(&self, py: Python) -> PyResult<PyObject> {
        // This method would need access to the original node data
        // For now, return None since we don't store location in NodeDetails
        Ok(py.None())
    }
}
