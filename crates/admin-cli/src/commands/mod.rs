use crate::config::Config;
use eyre::Result;

pub mod common;
pub mod domain;
pub mod node;
pub mod pool;
pub mod provider;
pub mod test;
pub mod token;
pub mod wallet;
pub mod work;

pub use domain::*;
pub use node::*;
pub use pool::*;
pub use provider::*;
pub use test::*;
pub use token::*;
pub use wallet::*;
pub use work::*;

pub async fn handle_token_command(command: TokenCommands, config: &Config) -> Result<()> {
    token::handle_command(command, config).await
}

pub async fn handle_domain_command(command: DomainCommands, config: &Config) -> Result<()> {
    domain::handle_command(command, config).await
}

pub async fn handle_pool_command(command: PoolCommands, config: &Config) -> Result<()> {
    pool::handle_command(command, config).await
}

pub async fn handle_provider_command(command: ProviderCommands, config: &Config) -> Result<()> {
    provider::handle_command(command, config).await
}

pub async fn handle_work_command(command: WorkCommands, config: &Config) -> Result<()> {
    work::handle_command(command, config).await
}

pub async fn handle_node_command(command: NodeCommands, config: &Config) -> Result<()> {
    node::handle_command(command, config).await
}

pub async fn handle_wallet_command(command: WalletCommands, config: &Config) -> Result<()> {
    wallet::handle_command(command, config).await
}

pub async fn handle_test_command(command: TestCommands, config: &Config) -> Result<()> {
    test::handle_command(command, config).await
}
