/// Protocol ID for Prime P2P communication
pub const PRIME_P2P_PROTOCOL: &[u8] = b"prime-p2p-v1";

/// Timeout for P2P requests in seconds
pub const P2P_REQUEST_TIMEOUT: u64 = 30;

/// Maximum message size in bytes (10MB)
pub const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024;
