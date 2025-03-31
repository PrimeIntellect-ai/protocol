use alloy::{
    network::TransactionBuilder, primitives::Address, primitives::U256, providers::Provider,
    rpc::types::TransactionRequest,
};
use clap::Parser;
use eyre::Result;
use shared::web3::wallet::Wallet;
use std::str::FromStr;
use url::Url;

#[derive(Parser)]
struct Args {
    /// Address to send ETH to
    #[arg(short = 'a', long)]
    address: String,

    /// Private key for transaction signing
    #[arg(short = 'k', long)]
    key: String,

    /// RPC URL
    #[arg(short = 'r', long)]
    rpc_url: String,

    /// Amount to send
    #[arg(short = 'm', long, default_value = "1000")]
    amount: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let wallet = Wallet::new(&args.key, Url::parse(&args.rpc_url)?).unwrap();

    let balance_before = wallet.provider.get_balance(wallet.signer.address()).await?;
    // Start of Selection
    let accounts = wallet.provider.get_accounts().await?;
    let from = accounts[0];
    let to = Address::from_str(&args.address).unwrap();
    let amount = U256::from(args.amount);
    let tx = TransactionRequest::default()
        .with_from(from)
        .with_to(to)
        .with_value(amount);

    // Send the transaction and listen for the transaction to be included.
    let tx_hash = wallet.provider.send_transaction(tx).await?.watch().await?;

    log::info!("Sent transaction: {tx_hash}");

    log::info!(
        "Sender's ETH balance before transaction: {} ETH",
        balance_before / U256::from(10u64.pow(18))
    );

    let balance_after = wallet.provider.get_balance(to).await?;
    log::info!(
        "Receiver's ETH balance after transaction: {} ETH",
        balance_after / U256::from(10u64.pow(18))
    );

    Ok(())
}
