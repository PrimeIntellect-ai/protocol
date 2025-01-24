use alloy_node_bindings::Anvil;
use alloy_node_bindings::AnvilInstance;
use std::process::Command;
use std::path::PathBuf;
use std::net::TcpListener;
use std::sync::Once;

static INIT: Once = Once::new();
const ANVIL_PORT: u16 = 8545;

pub struct TestChain {
    pub anvil: AnvilInstance,
    pub endpoint: String,
}

impl TestChain {
    pub fn setup() -> Result<Self, Box<dyn std::error::Error>> {
        if is_port_in_use(ANVIL_PORT) {
            return Err(format!("Port {} is already in use", ANVIL_PORT).into());
        }

        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR not set");
        let smart_contracts_dir = PathBuf::from(manifest_dir)
            .parent()
            .expect("Failed to get parent directory")
            .join("smart-contracts");

        std::env::set_current_dir(&smart_contracts_dir)?;

        let anvil = Anvil::new().port(ANVIL_PORT).spawn();
        let endpoint = anvil.endpoint();
        println!("RPC endpoint: {}", endpoint);

        // Deploy contracts only once for all tests
        INIT.call_once(|| {
            println!("Deploying contracts...");
            let output = Command::new("sh")
                .arg("deploy.sh")
                .output()
                .expect("Failed to execute deploy script");

            if !output.status.success() {
                panic!("Deploy script failed: {}", String::from_utf8_lossy(&output.stderr));
            }
            println!("Contracts deployed successfully");
        });

        Ok(TestChain {
            anvil,
            endpoint,
        })
    }

    // Generate a full prime setup including compute pool 
    pub fn generate_prime_setup() -> Result<(), Box<dyn std::error::Error>> {
        // TODO: Deploy compute pool, provider etc.
        Ok(())
    }
}

fn is_port_in_use(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_err()
}

// Automatically clean up Anvil when TestChain is dropped
impl Drop for TestChain {
    fn drop(&mut self) {
        println!("Cleaning up TestChain...");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_setup_chain() -> Result<(), Box<dyn std::error::Error>> {
        let chain = TestChain::setup()?;
        assert!(chain.endpoint.contains("8545"));
        Ok(())
    }
}