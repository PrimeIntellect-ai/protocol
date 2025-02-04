use alloy::primitives::{hex, Address};

// TODO: Parse these from env
#[cfg(not(feature = "testnet"))]
pub mod contract_addresses {
    use super::*;
    pub const PRIME_NETWORK_ADDRESS: Address =
        Address::new(hex!("0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512"));
    pub const AI_TOKEN_ADDRESS: Address =
        Address::new(hex!("0x5FbDB2315678afecb367f032d93F642f64180aa3"));
    pub const COMPUTE_REGISTRY_ADDRESS: Address =
        Address::new(hex!("0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0"));
    pub const DOMAIN_REGISTRY_ADDRESS: Address =
        Address::new(hex!("0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9"));
    pub const STAKE_MANAGER_ADDRESS: Address =
        Address::new(hex!("0xDc64a140Aa3E981100a9becA4E685f962f0cF6C9"));
    pub const COMPUTE_POOL_ADDRESS: Address =
        Address::new(hex!("0x0165878A594ca255338adfa4d48449f69242Eb8F"));
}

#[cfg(feature = "testnet")]
pub mod contract_addresses {
    use super::*;
    pub const PRIME_NETWORK_ADDRESS: Address =
        Address::new(hex!("0x0DFd3646391c8CBde50b8B3541a2F6f12718c23F"));
    pub const AI_TOKEN_ADDRESS: Address =
        Address::new(hex!("0x8958D3b2aa57Fe0d8CA6710EF1bED1f104e1CdeD"));
    pub const COMPUTE_REGISTRY_ADDRESS: Address =
        Address::new(hex!("0x3B03Ad8e9F03cfA364d80cd52b98E6523E041376"));
    pub const DOMAIN_REGISTRY_ADDRESS: Address =
        Address::new(hex!("0xE9f8e23199FA9A8331314272AdaF5D931c12384C"));
    pub const STAKE_MANAGER_ADDRESS: Address =
        Address::new(hex!("0x8e77B1e622f27B2F6cF8ED6605B15515F693bE3F"));
    pub const COMPUTE_POOL_ADDRESS: Address =
        Address::new(hex!("0x40d0bdd887b8f1711Ad8eD257dBFDe7d22AE9b67"));
}

pub use contract_addresses::*;
