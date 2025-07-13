use pyo3::prelude::*;
use pythonize::pythonize;
use shared::models::node::DiscoveryNode;
use shared::security::request_signer::sign_request_with_nonce;
use shared::web3::wallet::Wallet;
use std::time::Duration;
use url::Url;

/// Node details for validator operations
#[pyclass]
#[derive(Clone)]
pub(crate) struct NodeDetails {
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
    /// Get compute specs as a Python dictionary
    pub fn get_compute_specs(&self, py: Python) -> PyResult<PyObject> {
        // This would need access to the original DiscoveryNode's compute_specs
        // For now returning None
        Ok(py.None())
    }

    /// Get location as a Python dictionary
    pub fn get_location(&self, py: Python) -> PyResult<PyObject> {
        // This would need access to the original DiscoveryNode's location
        // For now returning None
        Ok(py.None())
    }
}

/// Prime Protocol Validator Client - for validating nodes and tasks
#[pyclass]
pub(crate) struct ValidatorClient {
    runtime: Option<tokio::runtime::Runtime>,
    wallet: Option<Wallet>,
    discovery_urls: Vec<String>,
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
        let rpc_url = Url::parse(&rpc_url).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Invalid RPC URL: {}", e))
        })?;
        let wallet = Wallet::new(&private_key, rpc_url)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        Ok(Self {
            runtime: Some(runtime),
            wallet: Some(wallet),
            discovery_urls,
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

    /// Initialize the validator client
    pub fn start(&mut self, _py: Python) -> PyResult<()> {
        self.get_or_create_runtime()?;
        Ok(())
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
