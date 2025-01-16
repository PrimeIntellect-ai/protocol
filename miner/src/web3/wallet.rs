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
        // Find better solution for cling
        let signer_clone = signer.clone(); // Clone the signer to avoid moving it

        let wallet = EthereumWallet::from(signer);
        let address = wallet.default_signer().address();
        println!("Wallet: {:?}", address);

        // TODO: Cleanup
        let wallet_clone = wallet.clone(); // Clone the wallet to avoid moving it
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

    pub async fn get_balance(&self) -> Result<U256, Box<dyn std::error::Error>> {
        let balance = self
            .provider
            .get_balance(self.wallet.default_signer().address())
            .await?;
        Ok(balance)
    }
}
