use crate::metrics::store::MetricsStore;
use anyhow::Result;
use log::error;
use shared::models::metric::Metric;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::{fs, path::Path};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::UnixListener,
};

const DEFAULT_MACOS_SOCKET: &str = "/tmp/com.prime.miner/metrics.sock";
const DEFAULT_LINUX_SOCKET: &str = "/var/run/com.prime.miner/metrics.sock";

pub struct TaskBridge {
    pub socket_path: String,
    pub metrics_store: Arc<MetricsStore>,
}

impl TaskBridge {
    pub fn new(socket_path: Option<&str>, metrics_store: Arc<MetricsStore>) -> Self {
        let path = match socket_path {
            Some(path) => path.to_string(),
            None => {
                if cfg!(target_os = "macos") {
                    DEFAULT_MACOS_SOCKET.to_string()
                } else {
                    DEFAULT_LINUX_SOCKET.to_string()
                }
            }
        };

        Self {
            socket_path: path,
            metrics_store,
        }
    }

    pub async fn run(&self) -> Result<()> {
        let socket_path = Path::new(&self.socket_path);

        if let Some(parent) = socket_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Cleanup existing socket if present
        if socket_path.exists() {
            fs::remove_file(socket_path)?;
        }

        let listener = UnixListener::bind(socket_path)?;

        // allow both owner and group to read/write
        fs::set_permissions(socket_path, fs::Permissions::from_mode(0o660))?;

        loop {
            let store = self.metrics_store.clone();
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    tokio::spawn(async move {
                        let mut reader = BufReader::new(stream);
                        let mut line = String::new();
                        while let Ok(n) = reader.read_line(&mut line).await {
                            if n == 0 {
                                break; // Connection closed
                            }
                            match serde_json::from_str::<Metric>(&line.trim()) {
                                // Trim the line before parsing
                                Ok(metric) => {
                                    if let Err(e) = metric.validate() {
                                        println!("Invalid metric: {}", e);
                                        return;
                                    }
                                    store.update_metric(metric).await;
                                }
                                Err(e) => {
                                    println!("Failed to parse metric: {}", e);
                                }
                            }

                            line.clear();
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
        let bridge = TaskBridge::new(Some(socket_path.to_str().unwrap()), metrics_store.clone());

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
        let bridge = TaskBridge::new(Some(socket_path.to_str().unwrap()), metrics_store.clone());

        // Run bridge in background
        let bridge_handle = tokio::spawn(async move { bridge.run().await });

        tokio::time::sleep(Duration::from_millis(100)).await;

        // Test client connection
        let stream = UnixStream::connect(&socket_path).await?;

        // Print stream output to debug
        println!("Connected to stream: {:?}", stream.peer_addr());

        assert!(stream.peer_addr().is_ok());

        bridge_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_message_sending() -> Result<()> {
        let temp_dir = tempdir()?;
        let socket_path = temp_dir.path().join("test.sock");
        let metrics_store = Arc::new(MetricsStore::new());
        let bridge = TaskBridge::new(Some(socket_path.to_str().unwrap()), metrics_store.clone());

        let bridge_handle = tokio::spawn(async move { bridge.run().await });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut stream = UnixStream::connect(&socket_path).await?;
        let sample_metric =
            Metric::new("test_label".to_string(), 10.0, "1234".to_string()).unwrap();
        let sample_metric = serde_json::to_string(&sample_metric)?;
        println!("Sending {:?}", sample_metric);
        let msg = format!("{}{}", sample_metric, "\n");
        stream.write_all(msg.as_bytes()).await?;
        stream.flush().await?;

        tokio::time::sleep(Duration::from_millis(100)).await;

        let all_metrics = metrics_store.get_all_metrics().await;
        let expected_metric = Metric {
            label: "test_label".to_string(),
            value: 10.0,
            taskid: "1234".to_string(),
        };
        let task_metrics = all_metrics.get("1234").unwrap();
        assert_eq!(task_metrics.get("test_label"), Some(&expected_metric));

        bridge_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_multiple_clients() -> Result<()> {
        let temp_dir = tempdir()?;
        let socket_path = temp_dir.path().join("test.sock");
        let metrics_store = Arc::new(MetricsStore::new());
        let bridge = TaskBridge::new(Some(socket_path.to_str().unwrap()), metrics_store.clone());

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
