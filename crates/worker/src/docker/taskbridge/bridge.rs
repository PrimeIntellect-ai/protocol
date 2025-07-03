use crate::docker::taskbridge::file_handler;
use crate::docker::taskbridge::json_helper;
use crate::metrics::store::MetricsStore;
use crate::state::system_state::SystemState;
use anyhow::bail;
use anyhow::Result;
use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::FutureExt;
use futures::StreamExt as _;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use shared::models::node::Node;
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::wallet::Wallet;
use shared::web3::wallet::WalletProvider;
use std::collections::HashSet;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::{fs, path::Path};
use tokio::io::AsyncReadExt;
use tokio::{io::BufReader, net::UnixListener};

const DEFAULT_SOCKET_FILE: &str = "prime-worker/com.prime.worker/metrics.sock";

pub struct TaskBridge {
    socket_path: std::path::PathBuf,
    config: TaskBridgeConfig,
}

#[derive(Clone)]
struct TaskBridgeConfig {
    metrics_store: Arc<MetricsStore>,

    // TODO: the optional values are only used for testing; refactor
    // the tests such that these aren't optional
    contracts: Option<Contracts<WalletProvider>>,
    node_config: Option<Node>,
    node_wallet: Option<Wallet>,
    docker_storage_path: String,
    state: Arc<SystemState>,
}

#[derive(Deserialize, Serialize, Debug)]
struct MetricInput {
    task_id: String,
    #[serde(flatten)]
    metrics: std::collections::HashMap<String, serde_json::Value>,
}

impl TaskBridge {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        socket_path: Option<&str>,
        metrics_store: Arc<MetricsStore>,
        contracts: Option<Contracts<WalletProvider>>,
        node_config: Option<Node>,
        node_wallet: Option<Wallet>,
        docker_storage_path: String,
        state: Arc<SystemState>,
    ) -> Result<Self> {
        let path = match socket_path {
            Some(path) => std::path::PathBuf::from(path),
            None => {
                let path =
                    homedir::my_home()?.ok_or(anyhow::anyhow!("failed to get home directory"))?;
                path.join(DEFAULT_SOCKET_FILE)
            }
        };

        Ok(Self {
            socket_path: path,
            config: TaskBridgeConfig {
                metrics_store,
                contracts,
                node_config,
                node_wallet,
                docker_storage_path,
                state,
            },
        })
    }

    pub(crate) fn get_socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    pub(crate) async fn run(self) -> Result<()> {
        let Self {
            socket_path,
            config,
        } = self;

        let socket_path = Path::new(&socket_path);
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

        let mut handle_stream_futures = FuturesUnordered::new();
        let mut listener_stream = tokio_stream::wrappers::UnixListenerStream::new(listener);
        let (file_validation_futures_tx, mut file_validation_futures_rx) =
            tokio::sync::mpsc::channel::<(String, BoxFuture<anyhow::Result<()>>)>(100);
        let mut file_validation_futures_set = HashSet::new();
        let mut file_validation_futures = FuturesUnordered::new();
        let (file_upload_futures_tx, mut file_upload_futures_rx) =
            tokio::sync::mpsc::channel::<BoxFuture<anyhow::Result<()>>>(100);
        let mut file_upload_futures_set = FuturesUnordered::new();

        loop {
            tokio::select! {
                Some(res) = listener_stream.next() => {
                    match res {
                        Ok(stream) => {
                            let handle_future = handle_stream(config.clone(), stream, file_upload_futures_tx.clone(), file_validation_futures_tx.clone()).fuse();
                            handle_stream_futures.push(tokio::task::spawn(handle_future));
                        }
                        Err(e) => {
                            error!("Accept failed on Unix socket: {e}");
                        }
                    }
                }
                Some(res) = handle_stream_futures.next() => {
                    match res {
                        Ok(Ok(())) => {
                            debug!("Stream handler completed successfully");
                        }
                        Ok(Err(e)) => {
                            error!("Stream handler failed: {e}");
                        }
                        Err(e) => {
                            error!("Error joining stream handler task: {e}");
                        }
                    }
                }
                Some((hash, fut)) = file_validation_futures_rx.recv() => {
                    if file_validation_futures_set.contains(&hash) {
                        debug!("duplicate file validation task for hash: {hash}, skipping");
                        continue;
                    }
                    // we never remove hashes from this set, as we should never
                    // submit the same file for validation twice.
                    file_validation_futures_set.insert(hash.clone());
                    file_validation_futures.push(async move {(hash, tokio::task::spawn(fut).await)});
                }
                Some((hash, res)) = file_validation_futures.next() => {
                    match res {
                        Ok(Ok(())) => {
                            debug!("File validation task for hash {hash} completed successfully");
                        }
                        Ok(Err(e)) => {
                            error!("File validation task for hash {hash} failed: {e}");
                        }
                        Err(e) => {
                            error!("Error joining file validation task for hash {hash}: {e}");
                        }
                    }
                }
                Some(fut) = file_upload_futures_rx.recv() => {
                    file_upload_futures_set.push(tokio::task::spawn(fut));
                }
                Some(res) = file_upload_futures_set.next() => {
                    match res {
                        Ok(Ok(())) => {
                            debug!("File upload task completed successfully");
                        }
                        Ok(Err(e)) => {
                            error!("File upload task failed: {e}");
                        }
                        Err(e) => {
                            error!("Error joining file upload task: {e}");
                        }
                    }
                }
            }
        }
    }
}

async fn handle_stream(
    config: TaskBridgeConfig,
    stream: tokio::net::UnixStream,
    file_upload_futures_tx: tokio::sync::mpsc::Sender<BoxFuture<'_, anyhow::Result<()>>>,
    file_validation_futures_tx: tokio::sync::mpsc::Sender<(
        String,
        BoxFuture<'_, anyhow::Result<()>>,
    )>,
) -> Result<()> {
    let addr = stream.peer_addr()?;
    debug!("Received connection from {addr:?}");
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
                bail!("Error reading from stream: {e}");
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
                if let Err(e) = handle_message(
                    config.clone(),
                    &json_str,
                    file_upload_futures_tx.clone(),
                    file_validation_futures_tx.clone(),
                )
                .await
                {
                    error!("Error handling message: {e}");
                }

                current_pos += byte_length;
                debug!("Advanced position to {current_pos} after processing JSON");
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
                    warn!(
                        "Discarding unparseable binary data after connection close ({} bytes)",
                        data.len()
                    );
                }
                // Break out of the loop
                break;
            }
        }
    }
    Ok(())
}

async fn handle_metric(config: TaskBridgeConfig, input: &MetricInput) -> Result<()> {
    debug!("Processing metric message");
    for (key, value) in input.metrics.iter() {
        debug!("Metric - Key: {key}, Value: {value}");
        let _ = config
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

async fn handle_file_upload(
    config: TaskBridgeConfig,
    json_str: &str,
    file_upload_futures_tx: tokio::sync::mpsc::Sender<BoxFuture<'_, anyhow::Result<()>>>,
    file_validation_futures_tx: tokio::sync::mpsc::Sender<(
        String,
        BoxFuture<'_, anyhow::Result<()>>,
    )>,
) -> Result<()> {
    debug!("Handling file upload");
    if let Ok(file_info) = serde_json::from_str::<serde_json::Value>(json_str) {
        let task_id = file_info["task_id"].as_str().unwrap_or("unknown");

        // Handle file upload if save_path is present
        if let Some(file_name) = file_info["output/save_path"].as_str() {
            info!("Handling file upload for task_id: {task_id}, file: {file_name}");

            let Some(wallet) = config.node_wallet.as_ref() else {
                bail!("no wallet found; must be set to upload files");
            };

            let _ = file_upload_futures_tx
                .send(Box::pin(file_handler::handle_file_upload(
                    config.docker_storage_path.clone(),
                    task_id.to_string(),
                    file_name.to_string(),
                    wallet.clone(),
                    config.state.clone(),
                )))
                .await;
        }

        // Handle file validation if sha256 is present
        if let Some(file_sha) = file_info["output/sha256"].as_str() {
            debug!("Processing file validation message");
            let output_flops: f64 = file_info["output/output_flops"].as_f64().unwrap_or(0.0);
            let input_flops: f64 = file_info["output/input_flops"].as_f64().unwrap_or(0.0);

            info!(
                "Handling file validation for task_id: {task_id}, sha: {file_sha}, output_flops: {output_flops}, input_flops: {input_flops}"
            );

            if let (Some(contracts), Some(node)) =
                (config.contracts.clone(), config.node_config.clone())
            {
                let provider = match config.node_wallet.as_ref() {
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

                let _ = file_validation_futures_tx
                    .send((
                        file_sha.to_string(),
                        Box::pin(file_handler::handle_file_validation(
                            file_sha.to_string(),
                            contracts.clone(),
                            node.clone(),
                            provider,
                            work_units,
                        )),
                    ))
                    .await;
            } else {
                error!("Missing contracts or node configuration for file validation");
            }
        }
    } else {
        error!("Failed to parse JSON: {json_str}");
    }
    Ok(())
}

async fn handle_message(
    config: TaskBridgeConfig,
    json_str: &str,
    file_upload_futures_tx: tokio::sync::mpsc::Sender<BoxFuture<'_, anyhow::Result<()>>>,
    file_validation_futures_tx: tokio::sync::mpsc::Sender<(
        String,
        BoxFuture<'_, anyhow::Result<()>>,
    )>,
) -> Result<()> {
    debug!("Extracted JSON object: {json_str}");
    if json_str.contains("output/save_path") {
        if let Err(e) = handle_file_upload(
            config,
            json_str,
            file_upload_futures_tx,
            file_validation_futures_tx,
        )
        .await
        {
            error!("Failed to handle file upload: {e}");
        }
    } else {
        debug!("Processing metric message");
        match serde_json::from_str::<MetricInput>(json_str) {
            Ok(input) => {
                if let Err(e) = handle_metric(config, &input).await {
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
