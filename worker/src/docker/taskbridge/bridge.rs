use crate::docker::taskbridge::file_handler;
use crate::metrics::store::MetricsStore;
use crate::state::system_state::SystemState;
use anyhow::Result;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use shared::models::node::Node;
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::wallet::Wallet;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::{fs, path::Path};
use tokio::io::AsyncReadExt;
use tokio::{io::BufReader, net::UnixListener};

pub const SOCKET_NAME: &str = "metrics.sock";
const DEFAULT_MACOS_SOCKET: &str = "/tmp/com.prime.worker/";
const DEFAULT_LINUX_SOCKET: &str = "/tmp/com.prime.worker/";

pub struct TaskBridge {
    pub socket_path: String,
    pub metrics_store: Arc<MetricsStore>,
    pub contracts: Option<Arc<Contracts>>,
    pub node_config: Option<Node>,
    pub node_wallet: Option<Arc<Wallet>>,
    pub docker_storage_path: Option<String>,
    pub state: Arc<SystemState>,
    pub silence_metrics: bool,
}

#[derive(Deserialize, Serialize, Debug)]
struct MetricInput {
    task_id: String,
    label: String,
    value: f64,
}

#[derive(Deserialize, Serialize, Debug)]
struct RequestUploadRequest {
    file_name: String,
    file_size: u64,
    file_type: String,
}

impl TaskBridge {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        socket_path: Option<&str>,
        metrics_store: Arc<MetricsStore>,
        contracts: Option<Arc<Contracts>>,
        node_config: Option<Node>,
        node_wallet: Option<Arc<Wallet>>,
        docker_storage_path: Option<String>,
        state: Arc<SystemState>,
        silence_metrics: bool,
    ) -> Self {
        let path = match socket_path {
            Some(path) => path.to_string(),
            None => {
                if cfg!(target_os = "macos") {
                    format!("{}{}", DEFAULT_MACOS_SOCKET, SOCKET_NAME)
                } else {
                    format!("{}{}", DEFAULT_LINUX_SOCKET, SOCKET_NAME)
                }
            }
        };

        Self {
            socket_path: path,
            metrics_store,
            contracts,
            node_config,
            node_wallet,
            docker_storage_path,
            state,
            silence_metrics,
        }
    }

    pub async fn run(&self) -> Result<()> {
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
                    error!("Failed to remove existing socket file: {}", e);
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
                error!("Failed to bind Unix socket: {}", e);
                return Err(e.into());
            }
        };

        // allow both owner and group to read/write
        match fs::set_permissions(socket_path, fs::Permissions::from_mode(0o660)) {
            Ok(_) => debug!("Set socket permissions to 0o660"),
            Err(e) => {
                error!("Failed to set socket permissions: {}", e);
                return Err(e.into());
            }
        }
        info!("TaskBridge socket created at: {}", socket_path.display());
        loop {
            let store = self.metrics_store.clone();
            let node = self.node_config.clone();
            let contracts = self.contracts.clone();
            let wallet = self.node_wallet.clone();
            let storage_path_clone = self.docker_storage_path.clone();
            let state_clone = self.state.clone();
            let silence_metrics = self.silence_metrics;
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    tokio::spawn(async move {
                        debug!("Received connection from {:?}", _addr);
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
                                    debug!("Read {} bytes from socket", n);
                                    n
                                }
                                Err(e) => {
                                    error!("Error reading from stream: {}", e);
                                    break;
                                }
                            };

                            data.extend_from_slice(&buffer[..n]);
                            debug!("Current data buffer size: {} bytes", data.len());

                            // Log the raw data received for debugging
                            if let Ok(data_str) = std::str::from_utf8(&data) {
                                info!("Raw data received: {}", data_str);
                            } else {
                                info!("Raw data received (non-UTF8): {} bytes", data.len());
                            }

                            let mut current_pos = 0;
                            while current_pos < data.len() {
                                // Try to find a complete JSON object
                                if let Some((json_str, byte_length)) =
                                    extract_next_json(&data[current_pos..])
                                {
                                    info!("Extracted JSON object: {}", json_str);
                                    if json_str.contains("file_name") {
                                        info!("Processing file_name message");
                                        if let Some(storage_path) = storage_path_clone.clone() {
                                            if let Ok(file_info) =
                                                serde_json::from_str::<serde_json::Value>(json_str)
                                            {
                                                if let Some(file_name) = file_info["value"].as_str()
                                                {
                                                    let task_id = file_info["task_id"]
                                                        .as_str()
                                                        .unwrap_or("unknown");
                                                    info!("Handling file upload for task_id: {}, file: {}", task_id, file_name);

                                                    if let Err(e) =
                                                        file_handler::handle_file_upload(
                                                            &storage_path,
                                                            task_id,
                                                            file_name,
                                                            wallet.as_ref().unwrap(),
                                                            &state_clone,
                                                        )
                                                        .await
                                                    {
                                                        error!(
                                                            "Failed to handle file upload: {}",
                                                            e
                                                        );
                                                    } else {
                                                        info!("File upload handled successfully");
                                                    }
                                                } else {
                                                    error!("Missing file_name value in JSON");
                                                }
                                            } else {
                                                error!(
                                                    "Failed to parse file_name JSON: {}",
                                                    json_str
                                                );
                                            }
                                        } else {
                                            error!("No storage path set");
                                        }
                                    } else if json_str.contains("file_sha") {
                                        info!("Processing file_sha message: {}", json_str);
                                        if let Ok(file_info) =
                                            serde_json::from_str::<serde_json::Value>(json_str)
                                        {
                                            if let Some(file_sha) = file_info["value"].as_str() {
                                                let task_id = file_info["task_id"]
                                                    .as_str()
                                                    .unwrap_or("unknown");
                                                info!("Handling file validation for task_id: {}, sha: {}", task_id, file_sha);

                                                if let (Some(contracts_ref), Some(node_ref)) =
                                                    (contracts.clone(), node.clone())
                                                {
                                                    if let Err(e) =
                                                        file_handler::handle_file_validation(
                                                            file_sha,
                                                            &contracts_ref,
                                                            &node_ref,
                                                        )
                                                        .await
                                                    {
                                                        error!(
                                                            "Failed to handle file validation: {}",
                                                            e
                                                        );
                                                    } else {
                                                        info!(
                                                            "File validation handled successfully"
                                                        );
                                                    }
                                                } else {
                                                    error!("Missing contracts or node configuration for file validation");
                                                }
                                            } else {
                                                error!("Missing file_sha value in JSON");
                                            }
                                        } else {
                                            error!("Failed to parse file_sha JSON: {}", json_str);
                                        }
                                    } else {
                                        info!("Processing metric message: {}", json_str);
                                        match serde_json::from_str::<MetricInput>(json_str) {
                                            Ok(input) => {
                                                info!(
                                                    "Received metric - Task: {}, Label: {}, Value: {}",
                                                    input.task_id, input.label, input.value
                                                );
                                                if !silence_metrics {
                                                    debug!("Updating metric store");
                                                    let _ = store
                                                        .update_metric(
                                                            input.task_id,
                                                            input.label,
                                                            input.value,
                                                        )
                                                        .await;
                                                }
                                            }
                                            Err(e) => {
                                                error!(
                                                    "Failed to parse metric input: {} {}",
                                                    json_str, e
                                                );
                                            }
                                        }
                                    }
                                    current_pos += byte_length;
                                    debug!(
                                        "Advanced position to {} after processing JSON",
                                        current_pos
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
                                        warn!("Discarding unparseable data after connection close: {}", unparsed);
                                    } else {
                                        warn!("Discarding unparseable binary data after connection close ({} bytes)", data.len());
                                    }
                                    // Break out of the loop
                                    break;
                                }
                            }
                        }

                        // Helper function to extract the next complete JSON object from a string
                        fn extract_next_json(input: &[u8]) -> Option<(&str, usize)> {
                            // Skip any leading whitespace (including newlines)
                            let mut start_pos = 0;
                            while start_pos < input.len() && (input[start_pos] <= 32) {
                                // ASCII space and below includes all whitespace
                                start_pos += 1;
                            }

                            if start_pos >= input.len() {
                                return None; // No content left
                            }

                            // If we find an opening brace, look for the matching closing brace
                            if input[start_pos] == b'{' {
                                let mut brace_count = 1;
                                let mut pos = start_pos + 1;

                                while pos < input.len() && brace_count > 0 {
                                    match input[pos] {
                                        b'{' => brace_count += 1,
                                        b'}' => brace_count -= 1,
                                        _ => {}
                                    }
                                    pos += 1;
                                }

                                if brace_count == 0 {
                                    // Found a complete JSON object
                                    if let Ok(json_str) =
                                        std::str::from_utf8(&input[start_pos..pos])
                                    {
                                        return Some((json_str, pos));
                                    }
                                }
                            }

                            // Alternatively, look for a newline-terminated JSON object
                            if let Some(newline_pos) =
                                input[start_pos..].iter().position(|&c| c == b'\n')
                            {
                                let end_pos = start_pos + newline_pos;
                                // Check if we have a complete JSON object in this line
                                if let Ok(line) = std::str::from_utf8(&input[start_pos..end_pos]) {
                                    let trimmed = line.trim();
                                    if trimmed.starts_with('{') && trimmed.ends_with('}') {
                                        return Some((trimmed, end_pos + 1)); // +1 to consume the newline
                                    }
                                }

                                // If not a complete JSON object, skip this line and try the next
                                return extract_next_json(&input[end_pos + 1..])
                                    .map(|(json, len)| (json, len + end_pos + 1));
                            }

                            None
                        }
                    });
                }
                Err(e) => error!("Accept failed on Unix socket: {}", e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::store::MetricsStore;
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
        let state = Arc::new(SystemState::new(None, false));
        let bridge = TaskBridge::new(
            Some(socket_path.to_str().unwrap()),
            metrics_store.clone(),
            None,
            None,
            None,
            None,
            state,
            false,
        );

        // Run the bridge in background
        let bridge_handle = tokio::spawn(async move { bridge.run().await });
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Verify socket exists with correct permissions
        assert!(socket_path.exists());
        let metadata = fs::metadata(&socket_path)?;
        let permissions = metadata.permissions();
        assert_eq!(permissions.mode() & 0o777, 0o660);

        // Cleanup
        bridge_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_client_connection() -> Result<()> {
        let temp_dir = tempdir()?;
        let socket_path = temp_dir.path().join("test.sock");
        let metrics_store = Arc::new(MetricsStore::new());
        let state = Arc::new(SystemState::new(None, false));
        let bridge = TaskBridge::new(
            Some(socket_path.to_str().unwrap()),
            metrics_store.clone(),
            None,
            None,
            None,
            None,
            state,
            false,
        );

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
        let state = Arc::new(SystemState::new(None, false));
        let bridge = TaskBridge::new(
            Some(socket_path.to_str().unwrap()),
            metrics_store.clone(),
            None,
            None,
            None,
            None,
            state,
            false,
        );

        let bridge_handle = tokio::spawn(async move { bridge.run().await });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut stream = UnixStream::connect(&socket_path).await?;
        let sample_metric = MetricInput {
            task_id: "1234".to_string(),
            label: "test_label".to_string(),
            value: 10.0,
        };
        let sample_metric = serde_json::to_string(&sample_metric)?;
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
    async fn test_multiple_clients() -> Result<()> {
        let temp_dir = tempdir()?;
        let socket_path = temp_dir.path().join("test.sock");
        let metrics_store = Arc::new(MetricsStore::new());
        let state = Arc::new(SystemState::new(None, false));
        let bridge = TaskBridge::new(
            Some(socket_path.to_str().unwrap()),
            metrics_store.clone(),
            None,
            None,
            None,
            None,
            state,
            false,
        );

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
