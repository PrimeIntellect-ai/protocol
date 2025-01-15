use alloy::{
    network::{EthereumWallet, NetworkWallet},
    signers::local::PrivateKeySigner,
    providers::{Provider, ProviderBuilder, RootProvider},
    transports::http::{Http, Client},
    primitives::U256,
};
use url::Url;

pub struct Wallet {
    pub wallet: EthereumWallet,
    pub provider: RootProvider<Http<Client>>
}

impl Wallet {
    pub async fn new(private_key: &str, provider_url: Url) -> Result<Self, Box<dyn std::error::Error>> {
        let signer: PrivateKeySigner = private_key.parse()?;
        let wallet = EthereumWallet::from(signer);
        let address = wallet.default_signer().address();
        println!("Wallet: {:?}", address);
        
        // Provider creation is async, so we need to build and connect
        let provider = ProviderBuilder::new()
            .on_http(provider_url);
            
        Ok(Self { 
            wallet,
            provider
        })
    }
}
