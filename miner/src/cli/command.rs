use crate::api::start_server;
use crate::system::run_system_check;
use clap::{Parser, Subcommand};
use colored::*;
use std::io::Write;
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

        /// Dry run the command without starting the miner
        #[arg(long)]
        #[arg(default_value = "false")]
        dry_run: bool,
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
            dry_run: _,
        } => {
            println!("\n{}", "🚀 PRIME MINER INITIALIZATION".bright_cyan().bold());
            println!("{}", "═".repeat(50).bright_cyan());

            // Steps:
            // 1. Ensure we have enough eth in our wallet to register on training run
            println!("\n[ETH] {}", "Checking wallet balance...".bright_green());

            // 2. Run Hardware detection and check
            println!("\n[SYS] {}", "Hardware detection".bright_blue());

            // 3. Run Software check
            println!("\n[SYS] {}", "Software verification".bright_blue());

            // 4. Run Network check
            println!("\n[NET] {}", "Network connectivity check".bright_magenta());

            // 5. Run Miner registration
            if !*dry_run {
                println!(
                    "\n[REG] {}",
                    "Registering miner on network...".bright_yellow()
                );
            } else {
                println!("\n[REG] {}", "Skipping miner registration (dry run mode)".bright_yellow());
            }

            // 6. Start HTTP Server to receive challenges and invites to join cluster
            println!("\n[SRV] {}", "Starting endpoint service".bright_white());

            // 7. Share Node IP with discovery service
            println!(
                "\n[P2P] {}",
                "Broadcasting node to discovery service...".bright_cyan()
            );

            print!(
                "\n[VAL] {}",
                "Waiting for validation challenge... ".bright_green()
            );

            // 8. Wait for validation challenge and monitor status on chain
            std::io::stdout().flush().unwrap();
            let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let mut i = 0;
            loop {
                print!(
                    "\r[VAL] {}",
                    "Waiting for validation challenge... ".yellow().bold()
                );
                print!("{}", spinner[i].yellow().bold());
                std::io::stdout().flush().unwrap();
                std::thread::sleep(std::time::Duration::from_millis(100));
                i = (i + 1) % spinner.len();
            }

            /*
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
            */
        }
    }
}
