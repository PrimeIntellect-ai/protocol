use redis::Client;

#[derive(Clone)]
pub struct RedisStore {
    // TODO: fix -Temp making this public
    pub client: Client,
}

impl RedisStore {
    pub fn new(redis_url: &str) -> Self {
        match Client::open(redis_url) {
            Ok(client) => {
                println!("Successfully connected to Redis at {}", redis_url);
                Self { client }
            }
            Err(e) => {
                println!("Failed to connect to Redis at {}: {}", redis_url, e);
                panic!("Redis connection error: {}", e); // Handle error appropriately
            }
        }
    }
}
