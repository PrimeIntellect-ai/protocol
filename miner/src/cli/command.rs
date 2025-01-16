use crate::api::server::start_server;
use crate::checks::hardware::run_hardware_check;
use crate::checks::software::run_software_check;
use crate::web3::wallet::{self, Wallet, WalletProvider};
use alloy::dyn_abi::DynSolValue;
// Import the Wallet struct
use alloy::{
    contract::{ContractInstance, Interface},
    network::{Ethereum, EthereumWallet, NetworkWallet, TransactionBuilder},
    primitives::utils::keccak256 as keccak,
    primitives::{address, Address, U256},
    providers::{Provider, ProviderBuilder, RootProvider},
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
    signers::Signer,
    transports::http::{Client, Http},
};

use clap::{Parser, Subcommand};
use colored::*;
use hex;
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json;
use url::Url;

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
        private_key_provider: String,

        /// Wallet private key (as a hex string)
        #[arg(long)]
        private_key_node: String,

        /// RPC URL
        #[arg(long, default_value = "http://localhost:8545")]
        rpc_url: String,

        /// Port number for the miner to listen on
        #[arg(long, default_value = "8080")]
        port: u16,

        /// External IP address for the miner to advertise
        #[arg(long)]
        external_ip: String,

        /// Compute pool ID
        #[arg(long)]
        compute_pool_id: u64,

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
            private_key_provider,
            private_key_node,
            port,
            external_ip,
            compute_pool_id,
            dry_run,
            rpc_url,
        } => {
            println!("\n{}", "🚀 PRIME MINER INITIALIZATION".bright_cyan().bold());
            let runtime = tokio::runtime::Runtime::new().unwrap();

            println!("{}", "═".repeat(50).bright_cyan());
            // Steps:
            // 1. Ensure we have enough eth in our wallet to register on training run

            // [RISK WARNING! Writing a private key in the code file is insecure behavior.]

            // Display the public address of the wallet
            let provider_wallet_instance =
                match Wallet::new(private_key_provider, Url::parse(rpc_url).unwrap()) {
                    Ok(wallet) => wallet,
                    Err(err) => {
                        eprintln!("Failed to create wallet: {}", err);
                        std::process::exit(1);
                    }
                };

            let node_wallet_instance =
                match Wallet::new(private_key_node, Url::parse(rpc_url).unwrap()) {
                    Ok(wallet) => wallet,
                    Err(err) => {
                        eprintln!("Failed to create wallet: {}", err);
                        std::process::exit(1);
                    }
                };

            let balance = runtime
                .block_on(provider_wallet_instance.get_balance())
                .unwrap();
            println!("Balance: {:?}", balance);
            let prime_network_address = "0xe7f1725e7734ce288f8367e1bb143e90bb3f0512";
            let ai_token_address = "0x5fbdb2315678afecb367f032d93f642f64180aa3";
            let compute_registry_address = "0x9fe46736679d2d9a65f0992f2272de9f3c7fa6e0";

            fn load_contract(
                address: &str,
                wallet: &Wallet,
                abi_file_path: &str,
            ) -> ContractInstance<Http<Client>, WalletProvider, Ethereum> {
                // Changed these types
                let address: Address = address.parse().expect("Invalid address format");
                let path = format!(
                    "{}/{}",
                    std::env::current_dir()
                        .expect("Failed to get current directory")
                        .to_string_lossy(),
                    abi_file_path
                );
                let artifact = std::fs::read(path).expect("Failed to read artifact");
                let abi_json: serde_json::Value = serde_json::from_slice(&artifact)
                    .map_err(|err| {
                        eprintln!("Failed to parse JSON: {}", err);
                        std::process::exit(1);
                    })
                    .unwrap_or_else(|_| {
                        eprintln!("Error parsing JSON, exiting.");
                        std::process::exit(1);
                    });

                let abi_value = abi_json
                    .get("abi")
                    .expect("Failed to get ABI from artifact");

                let abi = serde_json::from_str(&abi_value.to_string()).unwrap();
                ContractInstance::new(address.into(), wallet.provider.clone(), Interface::new(abi))
            }

            let ai_token_contract = load_contract(
                ai_token_address,
                &provider_wallet_instance,
                "artifacts/abi/AIToken.json",
            );
            let prime_network_contract = load_contract(
                prime_network_address,
                &provider_wallet_instance,
                "artifacts/abi/PrimeNetwork.json",
            );

            let compute_registry_contract = load_contract(
                compute_registry_address,
                &provider_wallet_instance,
                "artifacts/abi/ComputeRegistry.json",
            );

            // TODO: Rewrite abi loading as function

            // Super ugly experiment - will move code later

            // TODO: Register provider on network
            // TODO: Later we do not have to reregister on network
            pub async fn register_provider(
                wallet: &Wallet,
                prime_network_contract: &ContractInstance<Http<Client>, WalletProvider, Ethereum>,
                ai_token_contract: &ContractInstance<Http<Client>, WalletProvider, Ethereum>,
                compute_registry_contract: &ContractInstance<
                    Http<Client>,
                    WalletProvider,
                    Ethereum,
                >,
                prime_network_address: Address,
            ) -> Result<(), Box<dyn std::error::Error>> {
                let address = wallet.wallet.default_signer().address();
                println!("Address: {:?}", address);

                let balance: U256 = ai_token_contract
                    .function("balanceOf", &[address.into()])?
                    .call()
                    .await?
                    .first()
                    .and_then(|value| value.as_uint())
                    .unwrap_or_default()
                    .0;

                // Check if we are already provider
                let provider_response = compute_registry_contract
                    .function("getProvider", &[address.into()])?
                    .call()
                    .await?;

                let provider_address = provider_response
                    .first()
                    .unwrap()
                    .as_tuple()
                    .unwrap()
                    .get(0)
                    .unwrap()
                    .as_address()
                    .unwrap();

                let is_whitelisted = provider_response
                    .first()
                    .unwrap()
                    .as_tuple()
                    .unwrap()
                    .get(1)
                    .unwrap()
                    .as_bool()
                    .unwrap();
                println!("Provider address: {:?}", provider_address);
                println!("Is whitelisted: {:?}", is_whitelisted);
                let provider_exists = provider_address != Address::default();
                println!("Provider registered: {}", provider_exists);

                println!("Balance: {} tokens", balance);
                if !provider_exists {
                    let amount: U256 = U256::from(100);
                    let approve_tx = ai_token_contract
                        .function("approve", &[prime_network_address.into(), amount.into()])?
                        .send()
                        .await?
                        .watch()
                        .await?;
                    println!("Transaction approved: {:?}", approve_tx);

                    let register_tx = prime_network_contract
                        .function("registerProvider", &[amount.into()])?
                        .send()
                        .await?
                        .watch()
                        .await?;
                    println!("Provider registered: {:?}", register_tx);
                }

                // Get provider details again  - cleanup later
                let provider_response = compute_registry_contract
                    .function("getProvider", &[address.into()])?
                    .call()
                    .await?;

                let provider_address = provider_response
                    .first()
                    .unwrap()
                    .as_tuple()
                    .unwrap()
                    .get(0)
                    .unwrap()
                    .as_address()
                    .unwrap();

                let is_whitelisted = provider_response
                    .first()
                    .unwrap()
                    .as_tuple()
                    .unwrap()
                    .get(1)
                    .unwrap()
                    .as_bool()
                    .unwrap();
                println!("Provider address: {:?}", provider_address);
                println!("Is whitelisted: {:?}", is_whitelisted);

                let provider_exists = provider_address != Address::default();
                if !provider_exists {
                    eprintln!("Provider could not be registered.");
                    std::process::exit(1);
                }

                if !is_whitelisted {
                    eprintln!("Provider is not whitelisted. Cannot proceed.");
                    std::process::exit(1);
                }

                Ok(())
            }

            if let Err(e) = runtime.block_on(register_provider(
                &provider_wallet_instance,
                &prime_network_contract,
                &ai_token_contract,
                &compute_registry_contract,
                prime_network_address.parse().unwrap(),
            )) {
                eprintln!("Failed to register provider: {}", e);
                std::process::exit(1);
            }

            pub async fn add_compute_node(
                provider_wallet: &Wallet,
                node_wallet: &Wallet,
                prime_network_contract: &ContractInstance<Http<Client>, WalletProvider, Ethereum>,
                compute_registry_contract: &ContractInstance<
                    Http<Client>,
                    WalletProvider,
                    Ethereum,
                >,
            ) -> Result<(), Box<dyn std::error::Error>> {
                let node_response = compute_registry_contract
                    .function(
                        "getNode",
                        &[
                            provider_wallet.wallet.default_signer().address().into(),
                            node_wallet.wallet.default_signer().address().into(),
                        ],
                    )?
                    .call()
                    .await;

                // TODO: This should be cleaned up - either we add additional check if this is actually the no-exist error or work on the contract response
                match node_response {
                    Ok(response) => {
                        println!("Node response: {:?}", response);
                        if let Some(node_data) = response.first() {
                            // Process node data if it exists
                            let node_tuple = node_data.as_tuple().unwrap();
                            let is_active: bool = node_tuple.get(5).unwrap().as_bool().unwrap();
                            let is_validated: bool = node_tuple.get(6).unwrap().as_bool().unwrap();
                            println!("Is active: {:?}", is_active);
                            println!("Is validated: {:?}", is_validated);
                            return Ok(()); // Return Ok if the node is registered
                        } else {
                            println!("Node is not registered. Proceeding to add the node.");
                        }
                    }
                    Err(e) => {
                        println!("Expected failure when calling getNode: {}", e);
                    }
                }

                println!("Adding compute node");
                println!(
                    "Provider wallet: {:?}",
                    provider_wallet.wallet.default_signer().address()
                );
                println!(
                    "Node wallet: {:?}",
                    node_wallet.wallet.default_signer().address()
                );

                let provider_address = provider_wallet.wallet.default_signer().address();
                let node_address = node_wallet.wallet.default_signer().address();
                let digest =
                    keccak(&[provider_address.as_slice(), node_address.as_slice()].concat());

                let signature = node_wallet
                    .signer
                    .sign_message(&digest.as_slice())
                    .await?
                    .as_bytes();
                println!("Signature: {:?}", signature);

                // Create the signature bytes

                let compute_units: U256 = U256::from(10);
                let add_node_tx = prime_network_contract
                    .function(
                        "addComputeNode",
                        &[
                            node_address.into(),
                            "ipfs://nodekey/".to_string().into(),
                            compute_units.into(),
                            DynSolValue::Bytes(signature.to_vec()),
                        ],
                    )?
                    .send()
                    .await?
                    .watch()
                    .await?;

                /*let register_tx = prime_network_contract
                .function("registerProvider", &[amount.into()])?
                .send()
                .await?
                .watch()
                .await?;*/
                println!("Add node tx: {:?}", add_node_tx);
                Ok(())
            }

            if let Err(e) = runtime.block_on(add_compute_node(
                &provider_wallet_instance,
                &node_wallet_instance,
                &prime_network_contract,
                &compute_registry_contract,
            )) {
                eprintln!("Failed to add compute node: {}", e);
                std::process::exit(1);
            }

            // TODO: We need the system info and should share this with discovery service
            if run_system_checks(false, false).is_err() {
                std::process::exit(1);
            }

            println!("Uploading discovery info");
            pub async fn upload_discovery_info(
                wallet: &Wallet,
                compute_pool_id: &u64,
            ) -> Result<(), Box<dyn std::error::Error>> {
                println!(
                    "\n[P2P] {}",
                    "Broadcasting node to discovery service...".bright_cyan()
                );

                let base_url = "http://localhost:8089";
                let endpoint = format!("/api/nodes/{}", wallet.wallet.default_signer().address());
                let url = format!("{}{}", base_url, endpoint);

                println!("URL: {:?}", url);
                let node_data = serde_json::json!({
                    "capacity": 100,
                    "computePoolId": compute_pool_id,
                    "ipAddress": "127.0.0.1",
                    "port": 8080,
                });

                let node_data_string = serde_json::to_string(&node_data).unwrap().replace("\\", "");
                let message = format!(
                    "/api/nodes/{}{}",
                    wallet.wallet.default_signer().address(),
                    node_data_string
                );
                println!("Message: {:?}", message);
                let signature = wallet
                    .signer
                    .sign_message(&message.as_bytes())
                    .await?
                    .as_bytes();
                let signature_string = format!("0x{}", hex::encode(signature));

                println!("Signature: {:?}", signature_string);
                let response = reqwest::Client::new()
                    .put(&url)
                    .header(
                        "x-address",
                        wallet.wallet.default_signer().address().to_string(),
                    )
                    .header("x-signature", signature_string)
                    .json(&node_data)
                    .send()
                    .await?;

                if !response.status().is_success() {
                    eprintln!(
                        "Error: Received response with status code {}",
                        response.status()
                    );
                    std::process::exit(1);
                }

                Ok(())
            }

            if let Err(e) = runtime.block_on(upload_discovery_info(
                &node_wallet_instance,
                compute_pool_id,
            )) {
                eprintln!("Failed to upload discovery info: {}", e);
                std::process::exit(1);
            }
            println!("{}", "Discovery info uploaded".bright_green().bold());

            // 5. Run Miner registration
            /*if !dry_run {
                println!(
                    "\n[REG] {}",
                    "Registering miner on network...".bright_yellow()
                );
            } else {
                println!(
                    "\n[REG] {}",
                    "Skipping miner registration (dry run mode)".bright_yellow()
                );
            }*/

            // 6. Start HTTP Server to receive challenges and invites to join cluster
            println!("\n[SRV] {}", "Starting endpoint service".bright_white());

            // 7. Share Node IP with discovery service
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
