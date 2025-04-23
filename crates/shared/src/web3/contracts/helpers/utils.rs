use alloy::primitives::{keccak256, Selector};

pub fn get_selector(fn_image: &str) -> Selector {
    keccak256(fn_image.as_bytes())[..4].try_into().unwrap()
}
