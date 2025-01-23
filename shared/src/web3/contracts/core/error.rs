use alloy::primitives::Address;
use std::fmt;

#[derive(Debug)]
pub enum ContractError {
    // Initialization errors
    AbiParseError(String),
    ArtifactReadError(String),

    // Contract interaction errors
    CallError(String),
    TransactionError(String),

    // Data parsing errors
    DecodingError(String),
    InvalidResponse(String),

    // Business logic errors
    ProviderNotFound(Address),
    NodeNotRegistered { provider: Address, node: Address },
    InvalidProviderState(String),

    // Generic errors
    Web3Error(String),
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl std::error::Error for ContractError {}

impl fmt::Display for ContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Initialization errors
            ContractError::AbiParseError(msg) => write!(f, "Failed to parse ABI: {}", msg),
            ContractError::ArtifactReadError(msg) => write!(f, "Failed to read artifact: {}", msg),

            // Contract interaction errors
            ContractError::CallError(msg) => write!(f, "Contract call failed: {}", msg),
            ContractError::TransactionError(msg) => write!(f, "Transaction failed: {}", msg),

            // Data parsing errors
            ContractError::DecodingError(msg) => write!(f, "Failed to decode data: {}", msg),
            ContractError::InvalidResponse(msg) => write!(f, "Invalid contract response: {}", msg),

            // Business logic errors
            ContractError::ProviderNotFound(address) => {
                write!(f, "Provider not found: {:?}", address)
            }
            ContractError::NodeNotRegistered { provider, node } => write!(
                f,
                "Node {:?} not registered for provider {:?}",
                node, provider
            ),
            ContractError::InvalidProviderState(msg) => {
                write!(f, "Invalid provider state: {}", msg)
            }

            // Generic errors
            ContractError::Web3Error(msg) => write!(f, "Web3 error: {}", msg),
            ContractError::Other(e) => write!(f, "Other error: {}", e),
        }
    }
}

// Convenient type alias for Result with ContractError
pub type ContractResult<T> = Result<T, ContractError>;

// Conversion implementations for common error types
impl From<std::io::Error> for ContractError {
    fn from(err: std::io::Error) -> Self {
        ContractError::ArtifactReadError(err.to_string())
    }
}

impl From<serde_json::Error> for ContractError {
    fn from(err: serde_json::Error) -> Self {
        ContractError::AbiParseError(err.to_string())
    }
}
