mod checks;
mod cli;
mod console;
mod docker;
mod metrics;
mod operations;
mod p2p;
mod state;
mod utils;

pub use cli::Cli;
pub use utils::logging::setup_logging;

pub type TaskHandles = std::sync::Arc<tokio::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>;
