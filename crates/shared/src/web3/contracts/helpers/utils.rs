use alloy::contract::CallDecoder;
use alloy::providers::Provider;
use alloy::{
    contract::CallBuilder,
    primitives::{keccak256, FixedBytes, Selector},
};
use alloy_provider::Network;
use anyhow::Result;
use log::{debug, info, warn};
use tokio::time::{timeout, Duration};

use crate::web3::wallet::WalletProvider;

pub fn get_selector(fn_image: &str) -> Selector {
    keccak256(fn_image.as_bytes())[..4].try_into().unwrap()
}

pub type PrimeCallBuilder<'a, D> = alloy::contract::CallBuilder<&'a WalletProvider, D>;
pub async fn retry_call<P, D, N>(
    mut call: CallBuilder<&P, D, N>,
    max_tries: u32,
    initial_gas_price: Option<u128>,
    provider: P,
    retry_delay: Option<u64>,
) -> Result<FixedBytes<32>>
where
    P: Provider<N>,
    N: Network,
    D: CallDecoder,
{
    let mut tries = 0;
    let gas_price = initial_gas_price;
    let mut gas_multiplier: Option<f64> = None;
    let retry_delay = retry_delay.unwrap_or(5);
    if let Some(price) = gas_price {
        info!("Setting gas price to: {:?}", price);
        call = call.gas_price(price);
    }

    while tries < max_tries {
        let mut network_gas_price: Option<u128> = None;
        if tries > 0 {
            tokio::time::sleep(Duration::from_secs(retry_delay)).await;
            match provider.get_gas_price().await {
                Ok(gas_price) => {
                    network_gas_price = Some(gas_price);
                }
                Err(err) => {
                    warn!("Failed to get gas price from provider: {:?}", err);
                }
            }
        }
        if let (Some(multiplier), Some(nw_gas_price)) = (gas_multiplier, network_gas_price) {
            let new_gas = (multiplier * nw_gas_price as f64).round() as u128;
            if new_gas < nw_gas_price {
                return Err(anyhow::anyhow!("Gas price is too low"));
            }
            call = call.max_fee_per_gas(new_gas);
            call = call.gas_price(new_gas);
        }
        match call.send().await {
            Ok(result) => {
                debug!("Transaction sent, waiting for confirmation");
                match timeout(Duration::from_secs(20), result.watch()).await {
                    Ok(watch_result) => match watch_result {
                        Ok(hash) => return Ok(hash),
                        Err(err) => {
                            tries += 1;
                            if tries == max_tries {
                                return Err(anyhow::anyhow!(
                                    "Transaction failed after {} attempts: {:?}",
                                    tries,
                                    err
                                ));
                            }
                        }
                    },
                    Err(_) => {
                        warn!("Watch timed out, retrying transaction");
                        tries += 1;
                        if tries == max_tries {
                            return Err(anyhow::anyhow!(
                                "Max retries reached after watch timeouts"
                            ));
                        }
                        gas_multiplier = Some((110.0 + (tries as f64 * 10.0)) / 100.0);
                    }
                }
            }
            Err(err) => {
                warn!("Transaction failed: {:?}", err);
                println!("err: {:?}", err);

                let err_str = err.to_string();
                let retryable_errors = [
                    "replacement transaction underpriced",
                    "nonce too low",
                    "transaction underpriced",
                    "insufficient funds for gas",
                    "already known",
                    "temporarily unavailable",
                    "network congested",
                    "gas price too low",
                    "transaction pool full",
                    "max fee per gas less than block base fee",
                ];

                if retryable_errors.iter().any(|e| err_str.contains(e)) {
                    tries += 1;
                    if tries == max_tries {
                        return Err(anyhow::anyhow!(
                            "Max retries reached after {} attempts. Last error: {:?}",
                            tries,
                            err
                        ));
                    }

                    if err_str.contains("underpriced")
                        || err_str.contains("gas price too low")
                        || err_str.contains("max fee per gas less than block base fee")
                    {
                        gas_multiplier = Some((110.0 + (tries as f64 * 200.0)) / 100.0);
                    }
                } else {
                    return Err(anyhow::anyhow!(
                        "Transaction failed with non-retryable error: {:?}",
                        err
                    ));
                }
            }
        }
    }

    Err(anyhow::anyhow!("Max retries reached"))
}

#[cfg(test)]
mod tests {

    use std::sync::Arc;

    use super::*;
    use alloy::{primitives::U256, providers::ProviderBuilder, sol};
    use alloy_provider::WalletProvider;

    use anyhow::{Error, Result};

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

    #[tokio::test]
    async fn test_concurrent_calls() -> Result<(), Error> {
        let provider = Arc::new(
            ProviderBuilder::new()
                .with_simple_nonce_management()
                .connect_anvil_with_wallet_and_config(|anvil| anvil.block_time(2))?,
        );

        let provider_clone = provider.clone();
        let contract = Counter::deploy(provider_clone).await?;

        let provider_clone_1 = provider.clone();
        let contract_clone_1 = contract.clone();
        let handle_1 = tokio::spawn(async move {
            let call = contract_clone_1.setNumber(U256::from(100));
            retry_call(call, 3, Some(1), provider_clone_1, None).await
        });

        let contract_clone_2 = contract.clone();
        let provider_clone_2 = provider.clone();
        let handle_2 = tokio::spawn(async move {
            let call = contract_clone_2.setNumber(U256::from(100));
            retry_call(call, 3, None, provider_clone_2, None).await
        });

        let tx_base = handle_1.await.unwrap();
        let tx_one = handle_2.await.unwrap();

        assert!(tx_base.is_ok());
        assert!(tx_one.is_ok());
        Ok(())
    }

    #[tokio::test]
    async fn test_transaction_replacement() -> Result<(), Error> {
        let provider = Arc::new(
            ProviderBuilder::new()
                .with_simple_nonce_management()
                .connect_anvil_with_wallet_and_config(|anvil| anvil.block_time(5))?,
        );
        let contract = Counter::deploy(provider.clone()).await?;

        let wallet = provider.wallet();

        let tx_count = provider
            .get_transaction_count(wallet.default_signer().address())
            .await?;

        let provider_clone = provider.clone();
        let _ = contract.increment().nonce(tx_count).send().await?;

        let call_two = contract.increment().nonce(tx_count);
        let tx = retry_call(call_two, 3, None, provider_clone, Some(1)).await;

        assert!(tx.is_ok());
        Ok(())
    }
}
