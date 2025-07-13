use std::time::Duration;

/// Default P2P port for worker nodes
pub const DEFAULT_P2P_PORT: u16 = 8000;

/// Default number of retries for funding operations
pub const DEFAULT_FUNDING_RETRY_COUNT: u32 = 10;

/// Default compute units for node registration
pub const DEFAULT_COMPUTE_UNITS: u64 = 1;

/// Timeout for blockchain operations
pub const BLOCKCHAIN_OPERATION_TIMEOUT: Duration = Duration::from_secs(300);

/// Timeout for message queue operations
pub const MESSAGE_QUEUE_TIMEOUT: Duration = Duration::from_millis(100);

/// Pool status check interval
pub const POOL_STATUS_CHECK_INTERVAL: Duration = Duration::from_secs(15);

/// P2P shutdown timeout
pub const P2P_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Channel sizes
pub const P2P_CHANNEL_SIZE: usize = 100;
pub const MESSAGE_QUEUE_CHANNEL_SIZE: usize = 300;
