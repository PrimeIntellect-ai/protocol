use super::types::Metric;
use anyhow::Result;
use log::error;
use log::info;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{fs, path::Path};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::UnixListener,
};

const DEFAULT_MACOS_SOCKET: &str = "/tmp/com.prime.miner/metrics.sock";
const DEFAULT_LINUX_SOCKET: &str = "/var/run/com.prime.miner/metrics.sock";

pub struct TaskBridge {
    pub socket_path: String,
}

impl TaskBridge {
    pub fn new(socket_path: Option<&str>) -> Self {
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

        Self { socket_path: path }
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

        info!("TaskBridge listening on {}", socket_path.display());

        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    tokio::spawn(async move {
                        let mut reader = BufReader::new(stream);
                        let mut line = String::new();

                        while let Ok(n) = reader.read_line(&mut line).await {
                            if n == 0 {
                                break; // Connection closed
                            }
                            println!("msg {}", line);
                            match serde_json::from_str::<Metric>(&line) {
                                
                                Ok(metric) => {
                                    if let Err(e) = metric.validate() {
                                        println!("Invalid metric: {}", e);
                                        return;
                                    }
                                    println!("Received metric {} = {}", metric.label, metric.value);
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
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::io::AsyncWriteExt;
    use tokio::net::UnixStream;

    #[tokio::test]
    async fn test_socket_creation() -> Result<()> {
        let temp_dir = tempdir()?;
        let socket_path = temp_dir.path().join("test.sock");
        let bridge = TaskBridge::new(Some(socket_path.to_str().unwrap()));

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
        let bridge = TaskBridge::new(Some(socket_path.to_str().unwrap()));

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
        let bridge = TaskBridge::new(Some(socket_path.to_str().unwrap()));

        let bridge_handle = tokio::spawn(async move { bridge.run().await });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut stream = UnixStream::connect(&socket_path).await?;
        stream
            .write_all(b"{\"label\":\"tasks_processed\", \"value\":10,\"taskid\":\"task123\", }\n")
            .await?;
        stream.flush().await?;

        tokio::time::sleep(Duration::from_millis(100)).await;

        bridge_handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_multiple_clients() -> Result<()> {
        let temp_dir = tempdir()?;
        let socket_path = temp_dir.path().join("test.sock");
        let bridge = TaskBridge::new(Some(socket_path.to_str().unwrap()));

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
