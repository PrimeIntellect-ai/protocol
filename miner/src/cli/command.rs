use crate::api::server::start_server;
use crate::checks::hardware::run_hardware_check;
use crate::checks::software::run_software_check;
use clap::{Parser, Subcommand};
use colored::*;
use std::str::FromStr;
use alloy::{
    network::{EthereumWallet, NetworkWallet, TransactionBuilder},
    primitives::{address, U256},
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
};
use url::Url;
use crate::web3::wallet::Wallet; // Import the Wallet struct

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

        /// Wallet private key (as a hex string)
        #[arg(long)]
        private_key: String, 

        /// Port number for the miner to listen on
        #[arg(long, default_value = "8080")]
        port: u16,

        /// External IP address for the miner to advertise
        #[arg(long)]
        external_ip: String,

        /// Dry run the command without starting the miner
        #[arg(long, default_value = "false")]
        dry_run: bool,
    },
    /// Run system checks to verify hardware and software compatibility
    Check {
        /// Run only hardware checks
        #[arg(long, default_value = "false")]
        hardware_only: bool,

        /// Run only software checks  
        #[arg(long, default_value = "false")]
        software_only: bool,
    },
}

fn run_system_checks(hardware_only: bool, software_only: bool) -> Result<(), String> {
    if !software_only {
        println!("\n[SYS] {}", "Running hardware detection...".bright_blue());
        if let Err(err) = run_hardware_check() {
            eprintln!("{}", format!("Hardware check failed: {}", err).red().bold());
            return Err(err.to_string());
        }
    }

    if !hardware_only {
        println!("\n[SYS] {}", "Running software detection...".bright_blue());
        if let Err(err) = run_software_check() {
            eprintln!("{}", format!("Software check failed: {}", err).red().bold());
            return Err(err.to_string());
        }
    }

    Ok(())
}

pub fn execute_command(command: &Commands) {
    match command {
        Commands::Check {
            hardware_only,
            software_only,
        } => {
            println!("\n{}", "🔍 PRIME MINER SYSTEM CHECK".bright_cyan().bold());
            println!("{}", "═".repeat(50).bright_cyan());

            if run_system_checks(*hardware_only, *software_only).is_err() {
                std::process::exit(1);
            }

            println!(
                "\n[SYS] {}",
                "✅ System check passed!".bright_green().bold()
            );
        }
        Commands::Run {
            subnet_id: _,
            private_key,
            port,
            external_ip,
            dry_run,
        } => {
            println!("\n{}", "🚀 PRIME MINER INITIALIZATION".bright_cyan().bold());
            println!("{}", "═".repeat(50).bright_cyan());
            // Steps:
            // 1. Ensure we have enough eth in our wallet to register on training run

            // [RISK WARNING! Writing a private key in the code file is insecure behavior.]
            let signer: PrivateKeySigner = private_key.parse().expect("Invalid private key format");
            let wallet: EthereumWallet = EthereumWallet::from(signer);
            let address = wallet.default_signer().address();
            println!("{}", address);

            // Display the public address of the wallet
            let wallet_instance = Wallet::new(private_key, Url::parse("https://your.provider.url").unwrap()); 

            if run_system_checks(false, false).is_err() {
                std::process::exit(1);
            }

            // 4. Run Network check
            println!("\n[NET] {}", "Network connectivity check".bright_magenta());

            // 5. Run Miner registration
            if !dry_run {
                println!(
                    "\n[REG] {}",
                    "Registering miner on network...".bright_yellow()
                );
            } else {
                println!(
                    "\n[REG] {}",
                    "Skipping miner registration (dry run mode)".bright_yellow()
                );
            }

            // 6. Start HTTP Server to receive challenges and invites to join cluster
            println!("\n[SRV] {}", "Starting endpoint service".bright_white());

            // 7. Share Node IP with discovery service
            println!(
                "\n[P2P] {}",
                "Broadcasting node to discovery service...".bright_cyan()
            );

            /* print!(
                "\n[VAL] {}",
                "Waiting for validation challenge... ".bright_green()
            );*/

            // 8. Wait for validation challenge and monitor status on chain
            /* std::io::stdout().flush().unwrap();
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
            }*/

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
