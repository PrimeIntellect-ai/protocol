use crate::web3::wallet::{Wallet, WalletProvider};
use alloy::{
    contract::{ContractInstance, Interface},
    network::Ethereum,
    primitives::Address,
    transports::http::{Client, Http},
};

use std::include_bytes;

macro_rules! include_abi {
    ($path:expr) => {{
        const ABI_BYTES: &[u8] = include_bytes!($path);
        ABI_BYTES
    }};
}

#[derive(Clone)]
pub struct Contract {
    instance: ContractInstance<Http<Client>, WalletProvider, Ethereum>,
    provider: WalletProvider,
}

impl Contract {
    pub fn new(address: Address, wallet: &Wallet, abi_file_path: &str) -> Self {
        let instance = Self::parse_abi(abi_file_path, wallet, address);
        Self {
            instance,
            provider: wallet.provider.clone(),
        }
    }

    fn parse_abi(
        path: &str,
        wallet: &Wallet,
        address: Address,
    ) -> ContractInstance<Http<Client>, WalletProvider, Ethereum> {
        let artifact = match path {
            "compute_registry.json" => {
                include_abi!("../../../../artifacts/abi/compute_registry.json")
            }
            "ai_token.json" => include_abi!("../../../../artifacts/abi/ai_token.json"),
            "prime_network.json" => include_abi!("../../../../artifacts/abi/prime_network.json"),
            "compute_pool.json" => include_abi!("../../../../artifacts/abi/compute_pool.json"),
            "synthetic_data_work_validator.json" => {
                include_abi!("../../../../artifacts/abi/synthetic_data_work_validator.json")
            }
            _ => panic!("Unknown ABI file: {}", path),
        };

        let abi_json: serde_json::Value = serde_json::from_slice(artifact)
            .map_err(|err| {
                eprintln!("Failed to parse JSON: {}", err);
                std::process::exit(1);
            })
            .unwrap_or_else(|_| {
                eprintln!("Error parsing JSON, exiting.");
                std::process::exit(1);
            });
        let abi =
            serde_json::from_value(abi_json.clone()).expect("Failed to parse ABI from artifact");
        ContractInstance::new(address, wallet.provider.clone(), Interface::new(abi))
    }

    pub fn instance(&self) -> &ContractInstance<Http<Client>, WalletProvider, Ethereum> {
        &self.instance
    }

    pub fn provider(&self) -> &WalletProvider {
        &self.provider
    }
}
