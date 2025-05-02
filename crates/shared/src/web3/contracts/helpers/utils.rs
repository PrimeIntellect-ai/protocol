use alloy::providers::Provider;
use alloy::{
    contract::CallBuilder,
    network::Ethereum,
    primitives::{keccak256, FixedBytes, Selector},
    transports::http::{Client, Http},
};
use log::{debug, info, warn};
use tokio::time::{timeout, Duration};

use crate::web3::wallet::WalletProvider;

pub fn get_selector(fn_image: &str) -> Selector {
    keccak256(fn_image.as_bytes())[..4].try_into().unwrap()
}

pub type PrimeCallBuilder<'a> =
    CallBuilder<Http<Client>, &'a WalletProvider, alloy::json_abi::Function, Ethereum>;

pub async fn retry_call(
    mut call: PrimeCallBuilder<'_>,
    max_tries: u32,
    initial_gas_price: Option<u128>,
    provider: &WalletProvider,
) -> Result<FixedBytes<32>, Box<dyn std::error::Error>> {
    const WATCH_TIMEOUT_SECS: u64 = 20;
    const RETRY_DELAY_SECS: u64 = 5; // Add delay between retries
    let mut tries = 0;
    let mut gas_price = initial_gas_price;

    while tries < max_tries {
        if let Some(price) = gas_price {
            info!("Setting gas price to: {:?}", price);
            call = call.gas_price(price);
        }

        let mut network_gas_price: Option<u128> = None;
        if tries > 0 {
            tokio::time::sleep(Duration::from_secs(RETRY_DELAY_SECS)).await;
            match provider.get_gas_price().await {
                Ok(gas_price) => {
                    network_gas_price = Some(gas_price);
                }
                Err(err) => {
                    warn!("Failed to get gas price from provider: {:?}", err);
                }
            }
        }

        match call.send().await {
            Ok(result) => {
                debug!("Transaction sent, waiting for confirmation");
                // If the submission to chain passes it might still timeout
                match timeout(Duration::from_secs(WATCH_TIMEOUT_SECS), result.watch()).await {
                    Ok(watch_result) => match watch_result {
                        Ok(hash) => return Ok(hash),
                        Err(err) => {
                            tries += 1;
                            if tries == max_tries {
                                return Err(format!(
                                    "Transaction failed after {} attempts: {:?}",
                                    tries, err
                                )
                                .into());
                            }
                        }
                    },
                    Err(_) => {
                        // Watch timed out, retry the transaction
                        warn!("Watch timed out, retrying transaction");
                        tries += 1;
                        if tries == max_tries {
                            return Err("Max retries reached after watch timeouts".into());
                        }
                        // Increase gas price to help transaction go through
                        if let Some(network_gas_price) = network_gas_price {
                            gas_price = Some(network_gas_price * (110 + tries as u128 * 10) / 100);
                        }
                    }
                }
            }
            Err(err) => {
                warn!("Transaction failed: {:?}", err);

                if err
                    .to_string()
                    .contains("replacement transaction underpriced")
                {
                    tries += 1;
                    if tries == max_tries {
                        return Err("Max retries reached for underpriced transaction".into());
                    }

                    if let Some(network_gas_price) = network_gas_price {
                        gas_price = Some(network_gas_price * (110 + tries as u128 * 10) / 100);
                    }
                } else {
                    return Err(format!("Transaction failed: {:?}", err).into());
                }
            }
        }
    }

    Err("Max retries reached".into())
}



#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Error, Result};
    use alloy::{providers::ProviderBuilder, sol};



   sol! {
        #[allow(missing_docs)]
        // solc v0.8.26; solc Counter.sol --via-ir --optimize --bin
        #[sol(rpc, bytecode="6080806040523460135760df908160198239f35b600080fdfe6080806040526004361015601257600080fd5b60003560e01c9081633fb5c1cb1460925781638381f58a146079575063d09de08a14603c57600080fd5b3460745760003660031901126074576000546000198114605e57600101600055005b634e487b7160e01b600052601160045260246000fd5b600080fd5b3460745760003660031901126074576020906000548152f35b34607457602036600319011260745760043560005500fea2646970667358221220e978270883b7baed10810c4079c941512e93a7ba1cd1108c781d4bc738d9090564736f6c634300081a0033")]
        contract Counter {
            uint256 public number;
     
            function setNumber(uint256 newNumber) public {
                number = newNumber;
            }
     
            function increment() public {
                number++;
            }
        }
    }

    async fn terminte_pid(pid: u32) -> Result<(), Error> {
        std::process::Command::new("kill")
            .arg("-9")
            .arg(pid.to_string())
            .spawn()
            .expect("Failed to terminate anvil");
        Ok(())
    }

    async fn setup_test() -> Result<u32, Error> {
        // Start anvil in a subprocess
        let mut cmd = std::process::Command::new("anvil");
        cmd.arg("--port")
           .arg("9999")
           .stdout(std::process::Stdio::null())
           .stderr(std::process::Stdio::null());
        
        let status = cmd.spawn()
            .expect("Failed to start anvil");

        println!("Anvil started with pid: {}", status.id());
        tokio::time::sleep(Duration::from_secs(10)).await;
        Ok(status.id())
    }

    #[tokio::test]
    async fn test_setup_test() -> Result<(), Error> {
        // let pid = setup_test().await?;
        // terminte_pid(pid).await?;

        // let provider = ProviderBuilder::new().on_anvil_with_wallet();
 
        // Deploy the `Counter` contract.
        // let contract = Counter::deploy(&provider).await?;
 
        // println!("Deployed contract at address: {}", contract.address());
        Ok(())
    }

    
}

