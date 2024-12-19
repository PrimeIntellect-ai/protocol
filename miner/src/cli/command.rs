use crate::api::start_server;
use crate::system::run_system_check;
use clap::{Parser, Subcommand};
use colored::*;
use std::path::PathBuf;
use tokio;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    Run {
        /// Subnet ID to run the miner on
        #[arg(long)]
        subnet_id: String,

        /// Wallet address
        #[arg(long)]
        wallet_address: String, 

        /// Path to wallet private key file (e.g. ./keys/eth-private-key.json)
        #[arg(long)]
        private_key: PathBuf,

        /// Port number for the miner to listen on
        #[arg(long, default_value = "8080")]
        port: u16,

        /// External IP address for the miner to advertise
        #[arg(long)]
        external_ip: String,
    },
}

pub fn execute_command(command: &Commands) {
    match command {
        Commands::Run {
            subnet_id: _,
            wallet_address: _,
            private_key: _,
            port,
            external_ip,
        } => {
            println!("{}", "Starting miner...".green().bold());
            println!("{}", "Detecting hardware...".yellow().bold());

            if let Err(err) = run_system_check() {
                eprintln!("{}", format!("System check failed: {}", err).red().bold());
                std::process::exit(1);
            }

            // Start HTTP server
            let runtime = tokio::runtime::Runtime::new().unwrap();
            if let Err(err) = runtime.block_on(start_server(external_ip, *port)) {
                eprintln!(
                    "{}",
                    format!("Failed to start server: {}", err).red().bold()
                );
                std::process::exit(1);
            }
        }
    }
}
