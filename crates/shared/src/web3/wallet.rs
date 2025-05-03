use alloy::primitives::Address;
use alloy::{
    network::EthereumWallet,
    primitives::U256,
    providers::fillers::{
        BlobGasFiller, ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller, WalletFiller,
    },
    providers::{Provider, ProviderBuilder, RootProvider},
    signers::local::PrivateKeySigner,
};
use alloy_provider::fillers::SimpleNonceManager;
use url::Url;

pub type WalletProvider = FillProvider<
    JoinFill<
        JoinFill<
            JoinFill<
                alloy_provider::Identity,
                JoinFill<GasFiller, JoinFill<BlobGasFiller, JoinFill<NonceFiller, ChainIdFiller>>>,
            >,
            NonceFiller<SimpleNonceManager>,
        >,
        WalletFiller<EthereumWallet>,
    >,
    RootProvider,
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
            .with_simple_nonce_management()
            .wallet(wallet_clone)
            .connect_http(provider_url);

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
}
