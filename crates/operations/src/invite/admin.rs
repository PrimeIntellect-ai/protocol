use alloy::primitives::utils::keccak256 as keccak;
use alloy::primitives::{Address, U256};
use alloy::signers::Signer;
use anyhow::Result;
use rand_v8::prelude::*;
use shared::web3::wallet::Wallet;
use std::time::{SystemTime, UNIX_EPOCH};

/// Generates an invite signature for a node
///
/// This function is used by pool owners/admins to create signed invites
/// that authorize nodes to join their pool.
pub async fn generate_invite_signature(
    wallet: &Wallet,
    domain_id: u32,
    pool_id: u32,
    node_address: Address,
    nonce: [u8; 32],
    expiration: [u8; 32],
) -> Result<[u8; 65]> {
    let domain_id_bytes: [u8; 32] = U256::from(domain_id).to_be_bytes();
    let pool_id_bytes: [u8; 32] = U256::from(pool_id).to_be_bytes();

    let digest = keccak(
        [
            &domain_id_bytes,
            &pool_id_bytes,
            node_address.as_slice(),
            &nonce,
            &expiration,
        ]
        .concat(),
    );

    let signature = wallet
        .signer
        .sign_message(digest.as_slice())
        .await?
        .as_bytes()
        .to_owned();

    Ok(signature)
}

/// Generates an invite expiration timestamp
///
/// Creates an expiration timestamp for an invite, defaulting to 1000 seconds from now
pub fn generate_invite_expiration(seconds_from_now: Option<u64>) -> Result<[u8; 32]> {
    let duration = seconds_from_now.unwrap_or(1000);
    let expiration = U256::from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| anyhow::anyhow!("System time error: {}", e))?
            .as_secs()
            + duration,
    );
    Ok(expiration.to_be_bytes())
}

/// Generates a random nonce for invite
pub fn generate_invite_nonce() -> [u8; 32] {
    rand_v8::rngs::OsRng.gen()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_invite_nonce() {
        let nonce1 = generate_invite_nonce();
        let nonce2 = generate_invite_nonce();
        assert_ne!(nonce1, nonce2, "Nonces should be unique");
    }

    #[test]
    fn test_generate_invite_expiration() {
        let expiration = generate_invite_expiration(Some(3600)).unwrap();
        assert_eq!(expiration.len(), 32);
    }
}
