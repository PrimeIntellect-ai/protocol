use crate::docker::taskbridge::file_handler;
use crate::docker::taskbridge::json_helper;
use crate::metrics::store::MetricsStore;
use crate::state::system_state::SystemState;
use anyhow::Result;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use shared::models::node::Node;
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::wallet::Wallet;
use shared::web3::wallet::WalletProvider;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::{fs, path::Path};
use tokio::io::AsyncReadExt;
use tokio::{io::BufReader, net::UnixListener};

const DEFAULT_SOCKET_FILE: &str = "prime-worker/com.prime.worker/metrics.sock";

pub struct TaskBridge {
    pub socket_path: std::path::PathBuf,
    pub metrics_store: Arc<MetricsStore>,
    pub contracts: Option<Contracts<WalletProvider>>,
    pub node_config: Option<Node>,
    pub node_wallet: Option<Wallet>,
    pub docker_storage_path: String,
    pub state: Arc<SystemState>,
}

#[derive(Deserialize, Serialize, Debug)]
struct MetricInput {
    task_id: String,
    #[serde(flatten)]
    metrics: std::collections::HashMap<String, serde_json::Value>,
}

impl TaskBridge {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        socket_path: Option<&str>,
        metrics_store: Arc<MetricsStore>,
        contracts: Option<Contracts<WalletProvider>>,
        node_config: Option<Node>,
        node_wallet: Option<Wallet>,
        docker_storage_path: String,
        state: Arc<SystemState>,
    ) -> Result<Arc<Self>> {
        let path = match socket_path {
            Some(path) => std::path::PathBuf::from(path),
            None => {
                let path =
                    homedir::my_home()?.ok_or(anyhow::anyhow!("failed to get home directory"))?;
                path.join(DEFAULT_SOCKET_FILE)
            }
        };

        Ok(Arc::new(Self {
            socket_path: path,
            metrics_store,
            contracts,
            node_config,
            node_wallet,
            docker_storage_path,
            state,
        }))
    }

    async fn handle_metric(self: Arc<Self>, input: &MetricInput) -> Result<()> {
        debug!("Processing metric message");
        for (key, value) in input.metrics.iter() {
            debug!("Metric - Key: {key}, Value: {value}");
            let _ = self
                .metrics_store
                .update_metric(
                    input.task_id.clone(),
                    key.to_string(),
                    value.as_f64().unwrap_or(0.0),
                )
                .await;
        }
        Ok(())
    }
    fn handle_file_upload(self: Arc<Self>, json_str: &str) -> Result<()> {
        debug!("Handling file upload");
        if let Ok(file_info) = serde_json::from_str::<serde_json::Value>(json_str) {
            let task_id = file_info["task_id"].as_str().unwrap_or("unknown");

            // Handle file upload if save_path is present
            if let Some(file_name) = file_info["output/save_path"].as_str() {
                info!("Handling file upload for task_id: {task_id}, file: {file_name}");

                let storage_path_inner = self.docker_storage_path.clone();
                let task_id_inner = task_id.to_string();
                let file_name_inner = file_name.to_string();
                let wallet_inner = self.node_wallet.as_ref().unwrap().clone();
                let state_inner = self.state.clone();

                tokio::spawn(async move {
                    if let Err(e) = file_handler::handle_file_upload(
                        &storage_path_inner,
                        &task_id_inner,
                        &file_name_inner,
                        &wallet_inner,
                        &state_inner,
                    )
                    .await
                    {
                        error!("Failed to handle file upload: {e}");
                    } else {
                        info!("File upload handled successfully");
                    }
                });
            }

            // Handle file validation if sha256 is present
            if let Some(file_sha) = file_info["output/sha256"].as_str() {
                debug!("Processing file validation message");
                let output_flops: f64 = file_info["output/output_flops"].as_f64().unwrap_or(0.0);
                let input_flops: f64 = file_info["output/input_flops"].as_f64().unwrap_or(0.0);

                info!(
                    "Handling file validation for task_id: {task_id}, sha: {file_sha}, output_flops: {output_flops}, input_flops: {input_flops}"
                );

                if let (Some(contracts_ref), Some(node_ref)) =
                    (self.contracts.clone(), self.node_config.clone())
                {
                    let file_sha_inner = file_sha.to_string();
                    let contracts_inner = contracts_ref.clone();
                    let node_inner = node_ref.clone();
                    let provider = match self.node_wallet.as_ref() {
                        Some(wallet) => wallet.provider(),
                        None => {
                            error!("No wallet provider found");
                            return Err(anyhow::anyhow!("No wallet provider found"));
                        }
                    };

                    if output_flops <= 0.0 {
                        error!("Invalid work units calculation: output_flops ({output_flops}) must be greater than 0.0. Blocking file validation submission.");
                        return Err(anyhow::anyhow!(
                            "Invalid work units: output_flops must be greater than 0.0"
                        ));
                    }
                    let work_units = output_flops;

                    tokio::spawn(async move {
                        if let Err(e) = file_handler::handle_file_validation(
                            &file_sha_inner,
                            &contracts_inner,
                            &node_inner,
                            &provider,
                            work_units,
                        )
                        .await
                        {
                            error!("Failed to handle file validation: {e}");
                        }
                    });
                } else {
                    error!("Missing contracts or node configuration for file validation");
                }
            }
        } else {
            error!("Failed to parse JSON: {json_str}");
        }
        Ok(())
    }

    async fn handle_message(self: Arc<Self>, json_str: &str) -> Result<()> {
        debug!("Extracted JSON object: {json_str}");
        if json_str.contains("output/save_path") {
            if let Err(e) = self.handle_file_upload(json_str) {
                error!("Failed to handle file upload: {}", e);
            }
        } else {
            debug!("Processing metric message");
            match serde_json::from_str::<MetricInput>(json_str) {
                Ok(input) => {
                    if let Err(e) = self.handle_metric(&input).await {
                        error!("Failed to handle metric: {e}");
                    }
                }
                Err(e) => {
                    error!("Failed to parse metric input: {json_str} {e}");
                }
            }
        }

        Ok(())
    }

    pub async fn run(self: Arc<Self>) -> Result<()> {
        let socket_path = Path::new(&self.socket_path);
        debug!("Setting up TaskBridge socket at: {}", socket_path.display());

        if let Some(parent) = socket_path.parent() {
            match fs::create_dir_all(parent) {
                Ok(_) => debug!("Created parent directory: {}", parent.display()),
                Err(e) => {
                    error!(
                        "Failed to create parent directory {}: {}",
                        parent.display(),
                        e
                    );
                    return Err(e.into());
                }
            }
        }

        // Cleanup existing socket if present
        if socket_path.exists() {
            match fs::remove_file(socket_path) {
                Ok(_) => debug!("Removed existing socket file"),
                Err(e) => {
                    error!("Failed to remove existing socket file: {e}");
                    return Err(e.into());
                }
            }
        }

        let listener = match UnixListener::bind(socket_path) {
            Ok(l) => {
                info!("Successfully bound to Unix socket");
                l
            }
            Err(e) => {
                error!("Failed to bind Unix socket: {e}");
                return Err(e.into());
            }
        };

        // allow both owner and group to read/write
        match fs::set_permissions(socket_path, fs::Permissions::from_mode(0o666)) {
            Ok(_) => debug!("Set socket permissions to 0o666"),
            Err(e) => {
                error!("Failed to set socket permissions: {e}");
                return Err(e.into());
            }
        }
        info!("TaskBridge socket created at: {}", socket_path.display());
        loop {
            let bridge = self.clone();
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    tokio::spawn(async move {
                        debug!("Received connection from {_addr:?}");
                        let mut reader = BufReader::new(stream);
                        let mut buffer = vec![0; 1024];
                        let mut data = Vec::new();

                        loop {
                            let n = match reader.read(&mut buffer).await {
                                Ok(0) => {
                                    debug!("Connection closed by client");
                                    0
                                }
                                Ok(n) => {
                                    debug!("Read {n} bytes from socket");
                                    n
                                }
                                Err(e) => {
                                    error!("Error reading from stream: {e}");
                                    break;
                                }
                            };

                            data.extend_from_slice(&buffer[..n]);
                            debug!("Current data buffer size: {} bytes", data.len());

                            if let Ok(data_str) = std::str::from_utf8(&data) {
                                debug!("Raw data received: {data_str}");
                            } else {
                                debug!("Raw data received (non-UTF8): {} bytes", data.len());
                            }

                            let mut current_pos = 0;
                            while current_pos < data.len() {
                                // Try to find a complete JSON object
                                if let Some((json_str, byte_length)) =
                                    json_helper::extract_next_json(&data[current_pos..])
                                {
                                    let json_str = json_str.to_string();
                                    let bridge_clone = bridge.clone();
                                    if let Err(e) = bridge_clone.handle_message(&json_str).await {
                                        error!("Error handling message: {e}");
                                    }

                                    current_pos += byte_length;
                                    debug!(
                                        "Advanced position to {current_pos} after processing JSON"
                                    );
                                } else {
                                    debug!("No complete JSON object found, waiting for more data");
                                    break;
                                }
                            }

                            data = data.split_off(current_pos);
                            debug!(
                                "Remaining data buffer size after processing: {} bytes",
                                data.len()
                            );
                            if n == 0 {
                                if data.is_empty() {
                                    // No data left to process, we can break
                                    break;
                                } else {
                                    // We have data but couldn't parse it as complete JSON objects
                                    // and the connection is closed - log and discard
                                    if let Ok(unparsed) = std::str::from_utf8(&data) {
                                        warn!("Discarding unparseable data after connection close: {unparsed}");
                                    } else {
                                        warn!("Discarding unparseable binary data after connection close ({} bytes)", data.len());
                                    }
                                    // Break out of the loop
                                    break;
                                }
                            }
                        }
                    });
                }
                Err(e) => error!("Accept failed on Unix socket: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::store::MetricsStore;
    use serde_json::json;
    use shared::models::metric::MetricKey;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::io::AsyncWriteExt;
    use tokio::net::UnixStream;

    #[tokio::test]
    async fn test_socket_creation() -> Result<()> {
        let temp_dir = tempdir()?;
        let socket_path = temp_dir.path().join("test.sock");
        let metrics_store = Arc::new(MetricsStore::new());
        let state = Arc::new(SystemState::new(None, false, None));
        let bridge = TaskBridge::new(
            Some(socket_path.to_str().unwrap()),
            metrics_store.clone(),
            None,
            None,
            None,
            "test_storage_path".to_string(),
            state,
        )
        .unwrap();

        // Run the bridge in background
        let bridge_handle = tokio::spawn(async move { bridge.run().await });
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Verify socket exists with correct permissions
        assert!(socket_path.exists());
        let metadata = fs::metadata(&socket_path)?;
        let permissions = metadata.permissions();
        assert_eq!(permissions.mode() & 0o777, 0o666);

        // Cleanup
        bridge_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_client_connection() -> Result<()> {
        let temp_dir = tempdir()?;
        let socket_path = temp_dir.path().join("test.sock");
        let metrics_store = Arc::new(MetricsStore::new());
        let state = Arc::new(SystemState::new(None, false, None));
        let bridge = TaskBridge::new(
            Some(socket_path.to_str().unwrap()),
            metrics_store.clone(),
            None,
            None,
            None,
            "test_storage_path".to_string(),
            state,
        )
        .unwrap();

        // Run bridge in background
        let bridge_handle = tokio::spawn(async move { bridge.run().await });

        tokio::time::sleep(Duration::from_millis(100)).await;

        // Test client connection
        let stream = UnixStream::connect(&socket_path).await?;

        // Log stream output for debugging
        debug!("Connected to stream: {:?}", stream.peer_addr());

        assert!(stream.peer_addr().is_ok());

        bridge_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_message_sending() -> Result<()> {
        let temp_dir = tempdir()?;
        let socket_path = temp_dir.path().join("test.sock");
        let metrics_store = Arc::new(MetricsStore::new());
        let state = Arc::new(SystemState::new(None, false, None));
        let bridge = TaskBridge::new(
            Some(socket_path.to_str().unwrap()),
            metrics_store.clone(),
            None,
            None,
            None,
            "test_storage_path".to_string(),
            state,
        )
        .unwrap();

        let bridge_handle = tokio::spawn(async move { bridge.run().await });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut stream = UnixStream::connect(&socket_path).await?;
        let data = json!({
            "task_id": "1234",
            "test_label": 10.0,
            "test_label2": 20.0,
        });
        let sample_metric = serde_json::to_string(&data)?;
        debug!("Sending {:?}", sample_metric);
        let msg = format!("{}{}", sample_metric, "\n");
        stream.write_all(msg.as_bytes()).await?;
        stream.flush().await?;

        tokio::time::sleep(Duration::from_millis(100)).await;

        let all_metrics = metrics_store.get_all_metrics().await;

        let key = MetricKey {
            task_id: "1234".to_string(),
            label: "test_label".to_string(),
        };
        assert!(all_metrics.contains_key(&key));
        assert_eq!(all_metrics.get(&key).unwrap(), &10.0);

        bridge_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_file_submission() -> Result<()> {
        let temp_dir = tempdir()?;
        let socket_path = temp_dir.path().join("test.sock");
        let metrics_store = Arc::new(MetricsStore::new());
        let state = Arc::new(SystemState::new(None, false, None));
        let bridge = TaskBridge::new(
            Some(socket_path.to_str().unwrap()),
            metrics_store.clone(),
            None,
            None,
            None,
            "test_storage_path".to_string(),
            state,
        )
        .unwrap();

        let bridge_handle = tokio::spawn(async move { bridge.run().await });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut stream = UnixStream::connect(&socket_path).await?;
        let json = json!({
            "task_id": "1234",
            "output/save_path": "test.txt",
            "output/sha256": "1234567890",
            "output/output_flops": 1500.0,
            "output/input_flops": 2500.0,
        });
        let sample_metric = serde_json::to_string(&json)?;
        debug!("Sending {:?}", sample_metric);
        let msg = format!("{}{}", sample_metric, "\n");
        stream.write_all(msg.as_bytes()).await?;
        stream.flush().await?;

        tokio::time::sleep(Duration::from_millis(100)).await;

        let all_metrics = metrics_store.get_all_metrics().await;
        assert!(
            all_metrics.is_empty(),
            "Expected metrics to be empty but found: {:?}",
            all_metrics
        );

        bridge_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_multiple_clients() -> Result<()> {
        let temp_dir = tempdir()?;
        let socket_path = temp_dir.path().join("test.sock");
        let metrics_store = Arc::new(MetricsStore::new());
        let state = Arc::new(SystemState::new(None, false, None));
        let bridge = TaskBridge::new(
            Some(socket_path.to_str().unwrap()),
            metrics_store.clone(),
            None,
            None,
            None,
            "test_storage_path".to_string(),
            state,
        )
        .unwrap();

        let bridge_handle = tokio::spawn(async move { bridge.run().await });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut clients = vec![];
        for _ in 0..3 {
            let stream = UnixStream::connect(&socket_path).await?;
            clients.push(stream);
        }

        for client in &clients {
            assert!(client.peer_addr().is_ok());
        }

        bridge_handle.abort();
        Ok(())
    }
}
