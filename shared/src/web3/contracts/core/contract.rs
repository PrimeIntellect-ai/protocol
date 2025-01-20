use crate::web3::wallet::{Wallet, WalletProvider};
use alloy::{
    contract::{ContractInstance, Interface},
    network::Ethereum,
    primitives::Address,
    transports::http::{Client, Http},
};

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
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("artifacts")
            .join("abi")
            .join(path);

        let artifact = std::fs::read(&path)
            .unwrap_or_else(|_| panic!("Failed to read artifact at: {}", path.display()));
        let abi_json: serde_json::Value = serde_json::from_slice(&artifact)
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
