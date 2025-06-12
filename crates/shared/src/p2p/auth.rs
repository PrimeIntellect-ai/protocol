use alloy::primitives::Address;
use alloy::signers::Signature;
use anyhow::Result;
use log::{debug, warn};
use rand_v8::Rng;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

/// Generate a random authentication challenge
pub fn generate_auth_challenge() -> String {
    let mut rng = rand_v8::thread_rng();
    let nonce: u64 = rng.gen();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    format!("prime-auth-challenge:{}:{}", timestamp, nonce)
}

/// Verify an authentication challenge response
pub fn verify_auth_response(challenge: &str, address: &str, signature: &str) -> Result<Address> {
    // Parse the expected address
    let expected_address =
        Address::from_str(address).map_err(|_| anyhow::anyhow!("Invalid address format"))?;

    // Parse the signature
    let signature = signature.trim_start_matches("0x");
    let parsed_signature =
        Signature::from_str(signature).map_err(|_| anyhow::anyhow!("Invalid signature format"))?;

    // Recover the address from the challenge message
    let recovered_address = parsed_signature
        .recover_address_from_msg(challenge)
        .map_err(|_| anyhow::anyhow!("Failed to recover address from challenge"))?;

    // Verify the address matches
    if recovered_address != expected_address {
        debug!("Recovered address: {:?}", recovered_address);
        debug!("Expected address: {:?}", expected_address);
        return Err(anyhow::anyhow!("Address mismatch"));
    }

    // Check challenge timestamp (10 second window)
    if let Some(timestamp_str) = challenge.split(':').nth(1) {
        if let Ok(challenge_timestamp) = timestamp_str.parse::<u64>() {
            let current_time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();

            if current_time - challenge_timestamp > 10 {
                return Err(anyhow::anyhow!("Challenge expired"));
            }
        }
    }

    Ok(recovered_address)
}

/// Validator for checking if addresses are allowed
pub trait AddressValidator: Send + Sync {
    fn is_allowed(&self, address: &Address) -> bool;
}

/// Simple whitelist validator
pub struct WhitelistValidator {
    allowed_addresses: Vec<Address>,
}

impl WhitelistValidator {
    pub fn new(addresses: Vec<Address>) -> Self {
        Self {
            allowed_addresses: addresses,
        }
    }
}

impl AddressValidator for WhitelistValidator {
    fn is_allowed(&self, address: &Address) -> bool {
        self.allowed_addresses.contains(address)
    }
}

/// Always-allow validator (for testing/development)
pub struct AllowAllValidator;

impl AddressValidator for AllowAllValidator {
    fn is_allowed(&self, _address: &Address) -> bool {
        true
    }
}

/// Validate authentication with address whitelist
pub fn validate_with_whitelist(
    challenge: &str,
    address: &str,
    signature: &str,
    validator: &dyn AddressValidator,
) -> Result<Address> {
    let recovered_address = verify_auth_response(challenge, address, signature)?;

    if !validator.is_allowed(&recovered_address) {
        warn!(
            "Authentication attempt from unauthorized address: {}",
            recovered_address
        );
        return Err(anyhow::anyhow!("Address not authorized"));
    }

    Ok(recovered_address)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::request_signer::sign_message;
    use crate::web3::wallet::Wallet;
    use std::str::FromStr;
    use url::Url;

    #[tokio::test]
    async fn test_auth_challenge_flow() {
        let private_key = "0000000000000000000000000000000000000000000000000000000000000001";
        let wallet =
            Wallet::new(private_key, Url::parse("http://localhost:8080").unwrap()).unwrap();

        let address = wallet.wallet.default_signer().address();

        // Generate challenge
        let challenge = generate_auth_challenge();

        // Sign challenge
        let signature = sign_message(&challenge, &wallet).await.unwrap();

        // Verify response
        let result = verify_auth_response(&challenge, &address.to_string(), &signature);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), address);
    }

    #[tokio::test]
    async fn test_whitelist_validation() {
        let private_key = "0000000000000000000000000000000000000000000000000000000000000001";
        let wallet =
            Wallet::new(private_key, Url::parse("http://localhost:8080").unwrap()).unwrap();

        let address = wallet.wallet.default_signer().address();
        let validator = WhitelistValidator::new(vec![address]);

        let challenge = generate_auth_challenge();
        let signature = sign_message(&challenge, &wallet).await.unwrap();

        let result =
            validate_with_whitelist(&challenge, &address.to_string(), &signature, &validator);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_unauthorized_address() {
        let private_key = "0000000000000000000000000000000000000000000000000000000000000001";
        let wallet =
            Wallet::new(private_key, Url::parse("http://localhost:8080").unwrap()).unwrap();

        let address = wallet.wallet.default_signer().address();
        let other_address =
            Address::from_str("0x742d35Cc6634C0532925a3b844Bc454e4438f44e").unwrap();
        let validator = WhitelistValidator::new(vec![other_address]); // Different address

        let challenge = generate_auth_challenge();
        let signature = sign_message(&challenge, &wallet).await.unwrap();

        let result =
            validate_with_whitelist(&challenge, &address.to_string(), &signature, &validator);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not authorized"));
    }
}
