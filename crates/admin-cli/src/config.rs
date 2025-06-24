use eyre::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub rpc_url: Option<String>,
    pub default_keys: HashMap<String, String>,
    pub contract_addresses: HashMap<String, String>,
}

impl Config {
    pub fn load(config_path: &Option<String>, env_file: &str) -> Result<Self> {
        dotenv::from_filename(env_file).ok();

        let mut config = if let Some(path) = config_path {
            Self::load_from_file(path)?
        } else {
            Self::default()
        };

        config.load_from_env()?;
        Ok(config)
    }

    pub fn load_from_file(path: &str) -> Result<Self> {
        if !Path::new(path).exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path))?;

        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path))?;

        Ok(config)
    }

    pub fn load_from_env(&mut self) -> Result<()> {
        if let Ok(rpc_url) = std::env::var("RPC_URL") {
            self.rpc_url = Some(rpc_url);
        }

        let env_vars = [
            ("PRIVATE_KEY_FEDERATOR", "federator"),
            ("PRIVATE_KEY_VALIDATOR", "validator"),
            ("PRIVATE_KEY_PROVIDER", "provider"),
            ("POOL_OWNER_PRIVATE_KEY", "pool_owner"),
            ("PRIVATE_KEY_NODE", "node"),
            ("PRIVATE_KEY_NODE_2", "node_2"),
        ];

        for (env_var, key_name) in env_vars {
            if let Ok(key) = std::env::var(env_var) {
                self.default_keys.insert(key_name.to_string(), key);
            }
        }

        let contract_vars = [
            ("WORK_VALIDATION_CONTRACT", "work_validation"),
            ("FEDERATOR_ADDRESS", "federator_address"),
            ("VALIDATOR_ADDRESS", "validator_address"),
            ("PROVIDER_ADDRESS", "provider_address"),
            ("POOL_OWNER_ADDRESS", "pool_owner_address"),
            ("NODE_ADDRESS", "node_address"),
        ];

        for (env_var, contract_name) in contract_vars {
            if let Ok(address) = std::env::var(env_var) {
                self.contract_addresses
                    .insert(contract_name.to_string(), address);
            }
        }

        Ok(())
    }

    pub fn with_rpc_url(mut self, rpc_url: String) -> Self {
        self.rpc_url = Some(rpc_url);
        self
    }

    pub fn get_rpc_url(&self) -> Result<String> {
        self.rpc_url.clone().ok_or_else(|| {
            eyre::eyre!("RPC URL not configured. Set RPC_URL environment variable or use --rpc-url")
        })
    }

    pub fn get_default_key(&self, role: &str) -> Option<&String> {
        self.default_keys.get(role)
    }

    pub fn get_contract_address(&self, name: &str) -> Option<&String> {
        self.contract_addresses.get(name)
    }
}
