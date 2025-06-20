use alloy::primitives::{hex, Address};

// TODO: Parse these from env
#[cfg(not(feature = "testnet"))]
pub mod contract_addresses {
    use super::*;
    pub const PRIME_NETWORK_ADDRESS: Address =
        Address::new(hex!("0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9"));
    pub const AI_TOKEN_ADDRESS: Address =
        Address::new(hex!("0x5FbDB2315678afecb367f032d93F642f64180aa3"));
    pub const COMPUTE_REGISTRY_ADDRESS: Address =
        Address::new(hex!("0x5FC8d32690cc91D4c39d9d3abcBD16989F875707"));
    pub const DOMAIN_REGISTRY_ADDRESS: Address =
        Address::new(hex!("0x0165878A594ca255338adfa4d48449f69242Eb8F"));
    pub const STAKE_MANAGER_ADDRESS: Address =
        Address::new(hex!("0xa513E6E4b8f2a923D98304ec87F64353C4D5C853"));
    pub const COMPUTE_POOL_ADDRESS: Address =
        Address::new(hex!("0x610178dA211FEF7D417bC0e6FeD39F05609AD788"));
}

#[cfg(feature = "testnet")]
pub mod contract_addresses {
    use super::*;
    pub const PRIME_NETWORK_ADDRESS: Address =
        Address::new(hex!("0x8eA4f11DbfDfE2D7f8fB64Dd2c9dd4Ed610bf03E"));
    pub const AI_TOKEN_ADDRESS: Address =
        Address::new(hex!("0xAF874da2758fd319656D07cAD856EE1220c949d6"));
    pub const COMPUTE_REGISTRY_ADDRESS: Address =
        Address::new(hex!("0x43308a48E7bf1349e0f1732e4B027af6770d8f64"));
    pub const DOMAIN_REGISTRY_ADDRESS: Address =
        Address::new(hex!("0xc3dC276d7D23eDb8E17D40B9d04dc016a94E3380"));
    pub const STAKE_MANAGER_ADDRESS: Address =
        Address::new(hex!("0x5bD0CFDCD2d5D548A0d161CB47234798701c91BE"));
    pub const COMPUTE_POOL_ADDRESS: Address =
        Address::new(hex!("0x8c924BE4413931384A917bE76F4f8Aa6A56a674c"));
}

pub use contract_addresses::*;
