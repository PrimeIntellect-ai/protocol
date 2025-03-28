use crate::docker::taskbridge::file_handler;
use crate::metrics::store::MetricsStore;
use crate::state::system_state::SystemState;
use anyhow::Result;
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use shared::models::node::Node;
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::wallet::Wallet;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::{fs, path::Path};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::UnixListener,
};

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
                debug!("Successfully bound to Unix socket");
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

        loop {
            let store = self.metrics_store.clone();
            let node = self.node_config.clone();
            let contracts = self.contracts.clone();
            let wallet = self.node_wallet.clone();
            let storage_path_clone = self.docker_storage_path.clone();
            let state_clone = self.state.clone();

            match listener.accept().await {
                Ok((stream, _addr)) => {
                    tokio::spawn(async move {
                        let mut reader = BufReader::new(stream);
                        let mut line = String::new();
                        while let Ok(n) = reader.read_line(&mut line).await {
                            if n == 0 {
                                break; // Connection closed
                            }

                            let trimmed = line.trim();
                            let mut current_pos = 0;

                            // Keep processing JSON objects until we reach the end of the line
                            while current_pos < trimmed.len() {
                                // Try to find a complete JSON object
                                if let Some(json_str) = extract_next_json(&trimmed[current_pos..]) {
                                    debug!("Received metric: {:?}", json_str);
                                    if json_str.contains("file_name") {
                                        let storage_path = match &storage_path_clone {
                                            Some(path) => path,
                                            None => {
                                                println!(
                                                    "Storage path is not set - cannot upload file."
                                                );
                                                continue;
                                            }
                                        };

                                        if let Ok(file_info) =
                                            serde_json::from_str::<serde_json::Value>(json_str)
                                        {
                                            if let Some(file_name) = file_info["value"].as_str() {
                                                let task_id =
                                                    file_info["task_id"].as_str().unwrap();

                                                if let Err(e) = file_handler::handle_file_upload(
                                                    storage_path,
                                                    task_id,
                                                    file_name,
                                                    wallet.as_ref().unwrap(),
                                                    &state_clone,
                                                )
                                                .await
                                                {
                                                    error!("Failed to handle file upload: {}", e);
                                                }
                                            }
                                        }
                                    } else if json_str.contains("file_sha") {
                                        if let Ok(file_info) =
                                            serde_json::from_str::<serde_json::Value>(json_str)
                                        {
                                            if let Some(file_sha) = file_info["value"].as_str() {
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
                                                    }
                                                }
                                            }
                                        }
                                    } else {
                                        match serde_json::from_str::<MetricInput>(json_str) {
                                            Ok(input) => {
                                                if !store.silence_metrics {
                                                    info!(
                                                        "📊 Received metric - Task: {}, Label: {}, Value: {}",
                                                        input.task_id, input.label, input.value
                                                    );
                                                }
                                                let _ = store
                                                    .update_metric(
                                                        input.task_id,
                                                        input.label,
                                                        input.value,
                                                    )
                                                    .await;
                                            }
                                            Err(e) => {
                                                error!(
                                                    "Failed to parse metric input: {} {}",
                                                    json_str, e
                                                );
                                            }
                                        }
                                    }
                                    current_pos += json_str.len();
                                } else {
                                    break;
                                }
                            }

                            line.clear();
                        }

                        // Helper function to extract the next complete JSON object from a string
                        fn extract_next_json(input: &str) -> Option<&str> {
                            let mut brace_count = 0;
                            let mut start_found = false;
                            let mut start_idx = 0;

                            for (i, c) in input.chars().enumerate() {
                                match c {
                                    '{' => {
                                        if !start_found {
                                            start_found = true;
                                            start_idx = i;
                                        }
                                        brace_count += 1;
                                    }
                                    '}' => {
                                        brace_count -= 1;
                                        if brace_count == 0 && start_found {
                                            return Some(&input[start_idx..=i]);
                                        }
                                    }
                                    _ => continue,
                                }
                            }
                            None
                        }
                    });
                }
                Err(e) => error!("Accept failed: {}", e),
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
