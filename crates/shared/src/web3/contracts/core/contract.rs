use alloy::{
    contract::{ContractInstance, Interface},
    primitives::Address,
};

use std::include_bytes;

macro_rules! include_abi {
    ($path:expr) => {{
        const ABI_BYTES: &[u8] = include_bytes!($path);
        ABI_BYTES
    }};
}

#[derive(Clone)]
pub struct Contract<P: alloy_provider::Provider> {
    instance: ContractInstance<P>,
}

impl<P: alloy_provider::Provider> Contract<P> {
    pub fn new(address: Address, provider: P, abi_file_path: &str) -> Self {
        let instance = Self::parse_abi(abi_file_path, provider, address);
        Self { instance }
    }

    fn parse_abi(path: &str, provider: P, address: Address) -> ContractInstance<P> {
        let artifact = match path {
            "compute_registry.json" => {
                include_abi!("../../../../artifacts/abi/compute_registry.json")
            }
            "ai_token.json" => include_abi!("../../../../artifacts/abi/ai_token.json"),
            "prime_network.json" => include_abi!("../../../../artifacts/abi/prime_network.json"),
            "compute_pool.json" => include_abi!("../../../../artifacts/abi/compute_pool.json"),
            "rewards_distributor.json" => {
                include_abi!("../../../../artifacts/abi/rewards_distributor.json")
            }
            "synthetic_data_work_validator.json" => {
                include_abi!("../../../../artifacts/abi/synthetic_data_work_validator.json")
            }
            "stake_manager.json" => include_abi!("../../../../artifacts/abi/stake_manager.json"),
            "domain_registry.json" => {
                include_abi!("../../../../artifacts/abi/domain_registry.json")
            }
            _ => panic!("Unknown ABI file: {path}"),
        };

        let abi_json: serde_json::Value = serde_json::from_slice(artifact)
            .map_err(|err| {
                eprintln!("Failed to parse JSON: {err}");
                std::process::exit(1);
            })
            .unwrap_or_else(|_| {
                eprintln!("Error parsing JSON, exiting.");
                std::process::exit(1);
            });
        let abi = serde_json::from_value(abi_json).expect("Failed to parse ABI from artifact");

        ContractInstance::new(address, provider, Interface::new(abi))
    }

    pub fn instance(&self) -> &ContractInstance<P> {
        &self.instance
    }

    pub fn provider(&self) -> &P {
        self.instance.provider()
    }
}
