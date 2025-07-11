use thiserror::Error;

/// Result type alias for Prime Protocol operations
pub type Result<T> = std::result::Result<T, PrimeProtocolError>;

/// Errors that can occur in the Prime Protocol client
#[derive(Debug, Error)]
pub enum PrimeProtocolError {
    /// Invalid configuration provided
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    /// Blockchain interaction error
    #[error("Blockchain error: {0}")]
    BlockchainError(String),

    /// General runtime error
    #[error("Runtime error: {0}")]
    #[allow(dead_code)]
    RuntimeError(String),
}
