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
        Address::new(hex!("0x1B831318291C3C3eEd5D2f3377A3Cfe95Fb53c34"));
    pub const AI_TOKEN_ADDRESS: Address =
        Address::new(hex!("0xB2Ab233218232FF1880A473F17d8E70DCCDE0ec4"));
    pub const COMPUTE_REGISTRY_ADDRESS: Address =
        Address::new(hex!("0x147f99D458Fb0Fb283167d8dAf45E276Eed8EFdC"));
    pub const DOMAIN_REGISTRY_ADDRESS: Address =
        Address::new(hex!("0x05363E5fC277c2EF9bd41b544dc300c658C4Ab98"));
    pub const STAKE_MANAGER_ADDRESS: Address =
        Address::new(hex!("0xe546CFADD36ffB647830450994DB39Bb1992F028"));
    pub const COMPUTE_POOL_ADDRESS: Address =
        Address::new(hex!("0x552DBd5886D87D8566283547052CCfD795631f6F"));
}

pub use contract_addresses::*;
