use alloy::primitives::utils::keccak256 as keccak;
use alloy::primitives::{Address, Signature, U256};
use anyhow::{bail, Result};
use p2p::InviteRequest;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

/// Verifies an invite signature
///
/// This function is used by workers to verify that an invite is valid
/// and was signed by the correct pool owner/admin.
pub fn verify_invite_signature(
    invite: &InviteRequest,
    domain_id: u32,
    worker_address: Address,
    signer_address: Address,
) -> Result<bool> {
    // Parse the signature from hex string
    let signature = Signature::from_str(&invite.invite)
        .map_err(|e| anyhow::anyhow!("Failed to parse invite signature: {}", e))?;

    // Recreate the message that was signed
    let domain_id_bytes: [u8; 32] = U256::from(domain_id).to_be_bytes();
    let pool_id_bytes: [u8; 32] = U256::from(invite.pool_id).to_be_bytes();

    let message = keccak(
        [
            &domain_id_bytes,
            &pool_id_bytes,
            worker_address.as_slice(),
            &invite.nonce,
            &invite.expiration,
        ]
        .concat(),
    );

    // Verify the signature
    let recovered = signature
        .recover_address_from_msg(message.as_slice())
        .map_err(|e| anyhow::anyhow!("Failed to recover address: {}", e))?;

    Ok(recovered == signer_address)
}

/// Checks if an invite has expired
pub fn is_invite_expired(invite: &InviteRequest) -> Result<bool> {
    let expiration_u256 = U256::from_be_bytes(invite.expiration);
    let expiration_secs = expiration_u256.to::<u64>();

    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| anyhow::anyhow!("System time error: {}", e))?
        .as_secs();

    Ok(current_time > expiration_secs)
}

/// Validates an invite for a worker
///
/// Performs full validation including signature verification and expiration check
pub fn validate_invite(
    invite: &InviteRequest,
    domain_id: u32,
    worker_address: Address,
    pool_owner_address: Address,
) -> Result<()> {
    // Check expiration
    if is_invite_expired(invite)? {
        bail!("Invite has expired");
    }

    // Verify signature
    if !verify_invite_signature(invite, domain_id, worker_address, pool_owner_address)? {
        bail!("Invalid invite signature");
    }

    Ok(())
}

/// Extract pool information from an invite
#[derive(Debug, Clone)]
pub struct PoolInfo {
    pub pool_id: u32,
    pub endpoint_url: String,
}

impl PoolInfo {
    pub fn from_invite(invite: &InviteRequest) -> Self {
        use crate::invite::common::get_endpoint_from_url;

        Self {
            pool_id: invite.pool_id,
            endpoint_url: get_endpoint_from_url(&invite.url, ""),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p2p::InviteRequestUrl;

    #[test]
    fn test_is_invite_expired() {
        // Create an expired invite
        let past_time = U256::from(1000u64);
        let invite = InviteRequest {
            invite: "test".to_string(),
            pool_id: 1,
            url: InviteRequestUrl::MasterUrl("https://example.com".to_string()),
            timestamp: 0,
            expiration: past_time.to_be_bytes(),
            nonce: [0u8; 32],
        };

        assert!(is_invite_expired(&invite).unwrap());

        // Create a future invite
        let future_time = U256::from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600,
        );
        let future_invite = InviteRequest {
            invite: "test".to_string(),
            pool_id: 1,
            url: InviteRequestUrl::MasterUrl("https://example.com".to_string()),
            timestamp: 0,
            expiration: future_time.to_be_bytes(),
            nonce: [0u8; 32],
        };

        assert!(!is_invite_expired(&future_invite).unwrap());
    }

    #[test]
    fn test_pool_info_from_invite() {
        let invite = InviteRequest {
            invite: "test".to_string(),
            pool_id: 42,
            url: InviteRequestUrl::MasterUrl("https://example.com/api".to_string()),
            timestamp: 0,
            expiration: [0u8; 32],
            nonce: [0u8; 32],
        };

        let pool_info = PoolInfo::from_invite(&invite);
        assert_eq!(pool_info.pool_id, 42);
        assert_eq!(pool_info.endpoint_url, "https://example.com/api/");
    }
}
