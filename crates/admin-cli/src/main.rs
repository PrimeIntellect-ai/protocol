use clap::{Parser, Subcommand};
use eyre::Result;

mod commands;
mod config;
mod secure_key;

use commands::*;
use config::Config;

#[derive(Parser)]
#[command(name = "admin-cli")]
#[command(about = "Prime Intellect Network Administrative CLI")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Configuration file path
    #[arg(long, global = true)]
    config: Option<String>,

    /// RPC URL (overrides config)
    #[arg(long, global = true)]
    rpc_url: Option<String>,

    /// Environment file path
    #[arg(long, global = true, default_value = ".env")]
    env_file: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Token management operations
    Token {
        #[command(subcommand)]
        command: TokenCommands,
    },
    /// Domain management operations
    Domain {
        #[command(subcommand)]
        command: DomainCommands,
    },
    /// Pool management operations
    Pool {
        #[command(subcommand)]
        command: PoolCommands,
    },
    /// Provider management operations
    Provider {
        #[command(subcommand)]
        command: ProviderCommands,
    },
    /// Work management operations
    Work {
        #[command(subcommand)]
        command: WorkCommands,
    },
    /// Node management operations
    Node {
        #[command(subcommand)]
        command: NodeCommands,
    },
    /// Wallet operations
    Wallet {
        #[command(subcommand)]
        command: WalletCommands,
    },
    /// Testing utilities
    Test {
        #[command(subcommand)]
        command: TestCommands,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let config = Config::load(&cli.config, &cli.env_file)?;
    let config = if let Some(rpc_url) = cli.rpc_url {
        config.with_rpc_url(rpc_url)
    } else {
        config
    };

    match cli.command {
        Commands::Token { command } => handle_token_command(command, &config).await,
        Commands::Domain { command } => handle_domain_command(command, &config).await,
        Commands::Pool { command } => handle_pool_command(command, &config).await,
        Commands::Provider { command } => handle_provider_command(command, &config).await,
        Commands::Work { command } => handle_work_command(command, &config).await,
        Commands::Node { command } => handle_node_command(command, &config).await,
        Commands::Wallet { command } => handle_wallet_command(command, &config).await,
        Commands::Test { command } => handle_test_command(command, &config).await,
    }
}
