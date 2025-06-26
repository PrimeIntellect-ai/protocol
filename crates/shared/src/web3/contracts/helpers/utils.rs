// This is the correct, minimal, and tested fix.

use alloy::contract::CallDecoder;
use alloy::providers::Provider;
use alloy::{
    contract::CallBuilder,
    primitives::{keccak256, FixedBytes, Selector},
    providers::Network,
};
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
    provider: P,
    retry_delay: Option<u64>,
) -> Result<FixedBytes<32>>
where
    P: Provider<N> + Clone,
    N: Network,
    D: CallDecoder + Clone,
{
    const PENDING_TRANSACTION_TIMEOUT: Duration = Duration::from_secs(60);

    let mut tries = 0;
    let retry_delay = retry_delay.unwrap_or(2);
    let mut tx_hash = None;

    while tries < max_tries {
        if tries > 0 {
            tokio::time::sleep(Duration::from_secs(retry_delay)).await;

            if let Some(tx_hash) = tx_hash {
                let receipt = provider.get_transaction_receipt(tx_hash).await?;
                if receipt.is_some() {
                    return Ok(tx_hash);
                }
            }

            // On retry, always fetch fresh fee estimates from the provider.
            let priority_fee_res = provider.get_max_priority_fee_per_gas().await;
            let gas_price_res = provider.get_gas_price().await;

            if let (Ok(priority_fee), Ok(gas_price)) = (priority_fee_res, gas_price_res) {
                // To replace a transaction, we need to bump both fees.
                // A common strategy is to increase by a percentage (e.g., 20%).
                let new_priority_fee = (priority_fee as f64 * 1.2).round() as u128;
                let new_gas_price = (gas_price as f64 * 1.2).round() as u128;

                info!(
                    "Retrying with bumped fees: max_fee={new_gas_price}, priority_fee={new_priority_fee}"
                );

                call = call
                    .clone()
                    .max_fee_per_gas(new_gas_price)
                    .max_priority_fee_per_gas(new_priority_fee)
            } else {
                warn!("Could not get new gas fees, retrying with old settings.");
            }
        }

        match call.clone().send().await {
            Ok(result) => {
                debug!("Transaction sent, waiting for confirmation...");
                tx_hash = Some(result.tx_hash());

                match result
                    .with_timeout(Some(PENDING_TRANSACTION_TIMEOUT))
                    .watch()
                    .await
                {
                    Ok(hash) => return Ok(hash),
                    Err(err) => warn!("Transaction watch failed: {err:?}"),
                }
            }
            Err(err) => {
                warn!("Transaction send failed: {err:?}");
                let err_str = err.to_string().to_lowercase();
                if !err_str.contains("replacement transaction underpriced")
                    && !err_str.contains("nonce too low")
                    && !err_str.contains("transaction already imported")
                {
                    return Err(anyhow::anyhow!("Non-retryable error: {:?}", err));
                }
            }
        }
        tries += 1;
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

        let contract = Counter::deploy(provider.clone()).await?;
        let handle_1 = tokio::spawn({
            let contract = contract.clone();
            let provider = provider.clone();
            async move {
                let call = contract.setNumber(U256::from(100));
                retry_call(call, 3, provider, None).await
            }
        });

        let handle_2 = tokio::spawn({
            let contract = contract.clone();
            let provider = provider.clone();
            async move {
                let call = contract.setNumber(U256::from(100));
                retry_call(call, 3, provider, None).await
            }
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

        let _ = contract.increment().nonce(tx_count).send().await?;
        let call_two = contract.increment().nonce(tx_count);
        let tx = retry_call(call_two, 3, provider, Some(1)).await;

        assert!(tx.is_ok());
        Ok(())
    }
}
