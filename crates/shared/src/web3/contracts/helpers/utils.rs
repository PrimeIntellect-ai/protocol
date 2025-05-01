use alloy::providers::Provider;
use alloy::{
    contract::CallBuilder,
    network::Ethereum,
    primitives::{keccak256, FixedBytes, Selector},
    transports::http::{Client, Http},
};
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
            println!("Setting gas price to: {:?}", price);
            call = call.gas_price(price);
        }

        if tries > 0 {
            tokio::time::sleep(Duration::from_secs(RETRY_DELAY_SECS)).await;
        }

        match call.send().await {
            Ok(result) => {
                println!("Transaction sent, waiting for confirmation");
                // If the submission to chain passes it might still timeout
                match timeout(Duration::from_secs(WATCH_TIMEOUT_SECS), result.watch()).await {
                    Ok(watch_result) => {
                        match watch_result {
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
                                // Increase gas price to help transaction go through
                                let network_gas_price = provider.get_gas_price().await.unwrap();
                                gas_price =
                                    Some(network_gas_price * (110 + tries as u128 * 10) / 100);
                            }
                        }
                    }
                    Err(_) => {
                        // Watch timed out, retry the transaction
                        println!("Watch timed out, retrying transaction");
                        tries += 1;
                        if tries == max_tries {
                            return Err("Max retries reached after watch timeouts".into());
                        }
                    }
                }
            }
            Err(err) => {
                if initial_gas_price.is_some() {
                    println!("Transaction failed that had initial gas: {:?}", err);
                } else {
                    println!("Transaction failed: {:?}", err);
                }

                if err
                    .to_string()
                    .contains("replacement transaction underpriced")
                {
                    tries += 1;
                    if tries == max_tries {
                        return Err("Max retries reached for underpriced transaction".into());
                    }

                    // Increase gas price by 10% plus an additional 10% for each retry
                    if let Some(current_price) = gas_price {
                        gas_price = Some(current_price * (110 + tries as u128 * 10) / 100);
                    }
                } else {
                    return Err(format!("Transaction failed: {:?}", err).into());
                }
            }
        }
    }

    Err("Max retries reached".into())
}
