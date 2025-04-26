use std::time::Duration;

use alloy::eips::{BlockId, BlockNumberOrTag};
use alloy::network::TransactionBuilder;
use alloy::primitives::Address;
use alloy::rpc::types::TransactionRequest;
use alloy::{
    network::{Ethereum, EthereumWallet},
    primitives::U256,
    providers::fillers::{
        BlobGasFiller, ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller, WalletFiller,
    },
    providers::{Identity, Provider, ProviderBuilder, RootProvider},
    signers::local::PrivateKeySigner,
    transports::http::{Client, Http},
};
use url::Url;
pub type WalletProvider = FillProvider<
    JoinFill<
        JoinFill<
            Identity,
            JoinFill<GasFiller, JoinFill<BlobGasFiller, JoinFill<NonceFiller, ChainIdFiller>>>,
        >,
        WalletFiller<EthereumWallet>,
    >,
    RootProvider<Http<Client>>,
    Http<Client>,
    Ethereum,
>;

pub struct Wallet {
    pub wallet: EthereumWallet,
    pub signer: PrivateKeySigner,
    pub provider: WalletProvider,
}

impl Wallet {
    pub fn new(private_key: &str, provider_url: Url) -> Result<Self, Box<dyn std::error::Error>> {
        let signer: PrivateKeySigner = private_key.parse()?;
        let signer_clone = signer.clone();
        let wallet = EthereumWallet::from(signer);

        let wallet_clone = wallet.clone();
        let provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .wallet(wallet_clone)
            .on_http(provider_url);

        Ok(Self {
            wallet,
            signer: signer_clone,
            provider,
        })
    }

    pub fn address(&self) -> Address {
        self.wallet.default_signer().address()
    }

    pub async fn get_balance(&self) -> Result<U256, Box<dyn std::error::Error>> {
        let address = self.wallet.default_signer().address();
        let balance = self.provider.get_balance(address).await?;

        Ok(balance)
    }
    async fn cancel_tx(
        &self,
        nonce_to_cancel: u64,
        wait_for_confirmation: bool,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        const CANCEL_PRIORITY_FEE_BUFFER_WEI: u64 = 1_000_000;
        const CANCEL_MAX_FEE_BUFFER_WEI: u64 = 10_000_000;
        const CANCEL_WATCH_TIMEOUT: Duration = Duration::from_secs(180);
        let provider = &self.provider;
        let address = self.address();

        log::info!(
            "Preparing cancellation transaction for nonce: {}",
            nonce_to_cancel
        );

        let current_fees = provider.estimate_eip1559_fees(None).await.map_err(|e| {
            let msg = format!(
                "Cancel Tx (Nonce {}): Failed EIP-1559 fee estimation: {}",
                nonce_to_cancel, e
            );
            log::error!("{}", msg);
            msg
        })?;

        let final_priority_fee = U256::from(current_fees.max_priority_fee_per_gas)
            + U256::from(CANCEL_PRIORITY_FEE_BUFFER_WEI);

        let current_base_fee_proxy: u128 = provider.get_gas_price().await.map_err(|e| {
            format!(
                "Failed to get current gas price for max fee calculation: {}",
                e
            )
        })?;

        let final_max_fee = U256::from(current_base_fee_proxy)
            + final_priority_fee
            + U256::from(CANCEL_MAX_FEE_BUFFER_WEI);

        let tx_request = TransactionRequest::default()
            .with_to(address)
            .with_value(U256::ZERO)
            .with_nonce(nonce_to_cancel)
            .with_gas_limit(21000)
            .with_max_fee_per_gas(final_max_fee.to::<u128>())
            .with_max_priority_fee_per_gas(final_priority_fee.to::<u128>());

        let pending_tx = provider.send_transaction(tx_request).await?;
        let tx_hash = *pending_tx.tx_hash();

        log::info!(
            "Cancellation transaction for nonce {} sent: {}",
            nonce_to_cancel,
            tx_hash
        );

        if wait_for_confirmation {
            match tokio::time::timeout(CANCEL_WATCH_TIMEOUT, pending_tx.watch()).await {
                Ok(Ok(receipt_hash)) => {
                    log::info!(
                        "Cancellation tx {} (nonce {}) confirmed. Receipt Hash: {}",
                        tx_hash,
                        nonce_to_cancel,
                        receipt_hash
                    );
                    Ok(true)
                }
                Ok(Err(e)) => {
                    log::error!(
                        "Error watching cancellation tx {} (nonce {}): {}",
                        tx_hash,
                        nonce_to_cancel,
                        e
                    );
                    Err(Box::new(e))
                }
                Err(_) => {
                    let msg = format!(
                        "Timeout waiting for cancellation tx {} (nonce {}) confirmation after {:?}",
                        tx_hash, nonce_to_cancel, CANCEL_WATCH_TIMEOUT
                    );
                    log::info!("{}", msg);
                    Err(msg.into())
                }
            }
        } else {
            Ok(true)
        }
    }

    pub async fn clear_pending_transactions(&self) -> Result<bool, Box<dyn std::error::Error>> {
        let address = self.address();

        let latest_nonce = self.provider.get_transaction_count(address).await?;
        let pending_nonce = self
            .provider
            .get_transaction_count(address)
            .block_id(BlockId::Number(BlockNumberOrTag::Pending))
            .await?;

        if pending_nonce > latest_nonce {
            log::info!(
                "Pending transactions detected - Pending nonce {:?} vs next accepted nonce {:?}",
                pending_nonce,
                latest_nonce
            );
            let total_pending = pending_nonce - latest_nonce;
            let mut nonce_to_cancel = latest_nonce;
            for i in 0..total_pending {
                if i > 0 {
                    nonce_to_cancel += 1;
                }
                println!("Nonce to cancel {}", nonce_to_cancel);
                match self.cancel_tx(nonce_to_cancel, false).await {
                    Ok(_) => {
                        log::info!(
                            "Successfully sent cancellation for nonce {}",
                            nonce_to_cancel
                        );
                    }
                    Err(e) => {
                        let error_str = e.to_string();

                        // Check if the error is due to nonce being too low
                        println!("Error {:?}", error_str);
                        if error_str.contains("nonce too low") {
                            // Try to extract the next nonce from the error message
                            if let Some(next_nonce_str) = error_str.split("next nonce ").nth(1) {
                                if let Some(next_nonce_end) = next_nonce_str.find(',') {
                                    let next_nonce_str = &next_nonce_str[..next_nonce_end];
                                    if let Ok(next_nonce) = next_nonce_str.parse::<u64>() {
                                        log::info!(
                                            "Nonce too low error. Adjusting from {} to {}",
                                            nonce_to_cancel,
                                            next_nonce
                                        );
                                        continue; // Try again with the updated nonce
                                    }
                                }
                            }

                            // If we couldn't parse the next nonce from the error, increment and try again
                            log::info!(
                                "Nonce too low error. Incrementing nonce from {} to {}",
                                nonce_to_cancel,
                                nonce_to_cancel + 1
                            );
                            continue;
                        }

                        // For other errors, log and break
                        log::error!(
                            "Failed to cancel transaction with nonce {}: {:?}",
                            nonce_to_cancel,
                            e
                        );
                        break;
                    }
                }
            }
        }

        Ok(pending_nonce > latest_nonce)
    }
}
