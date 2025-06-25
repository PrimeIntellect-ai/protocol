#[cfg(test)]
use log::debug;
use log::info;
use redis::Client;
#[cfg(test)]
use redis_test::server::RedisServer;
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use std::thread;
#[cfg(test)]
use std::time::Duration;
#[derive(Clone)]
pub struct RedisStore {
    pub client: Client,
    #[allow(dead_code)]
    #[cfg(test)]
    server: Arc<RedisServer>,
}

impl RedisStore {
    pub fn new(redis_url: &str) -> Self {
        match Client::open(redis_url) {
            Ok(client) => {
                info!("Successfully connected to Redis at {redis_url}");
                Self {
                    client,
                    #[cfg(test)]
                    server: Arc::new(RedisServer::new()),
                }
            }
            Err(e) => {
                panic!("Redis connection error: {e}");
            }
        }
    }

    #[cfg(test)]
    pub fn new_test() -> Self {
        let server = RedisServer::new();

        // Get the server address
        let (host, port) = match server.client_addr() {
            redis::ConnectionAddr::Tcp(host, port) => (host.clone(), *port),
            _ => panic!("Expected TCP connection"),
        };

        let redis_url = format!("redis://{}:{}", host, port);
        debug!("Starting test Redis server at {}", redis_url);

        // Add a small delay to ensure server is ready
        thread::sleep(Duration::from_millis(100));

        // Try to connect with retry logic
        let client = loop {
            if let Ok(client) = Client::open(redis_url.clone()) {
                // Verify connection works
                if let Ok(mut conn) = client.get_connection() {
                    if redis::cmd("PING").query::<String>(&mut conn).is_ok() {
                        break client;
                    }
                }
            }
            thread::sleep(Duration::from_millis(100));
        };

        Self {
            client,
            server: Arc::new(server),
        }
    }
}
