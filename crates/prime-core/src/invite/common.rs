use alloy::primitives::Address;
use anyhow::Result;
use p2p::{InviteRequest, InviteRequestUrl};
use std::time::{SystemTime, UNIX_EPOCH};

/// Builder for creating invite requests
pub struct InviteBuilder {
    pool_id: u32,
    url: InviteRequestUrl,
}

impl InviteBuilder {
    /// Creates a new InviteBuilder with a master URL
    pub fn with_url(pool_id: u32, url: String) -> Self {
        Self {
            pool_id,
            url: InviteRequestUrl::MasterUrl(url),
        }
    }

    /// Creates a new InviteBuilder with IP and port
    pub fn with_ip_port(pool_id: u32, ip: String, port: u16) -> Self {
        Self {
            pool_id,
            url: InviteRequestUrl::MasterIpPort(ip, port),
        }
    }

    /// Builds an InviteRequest with the given parameters
    pub fn build(
        self,
        invite_signature: [u8; 65],
        nonce: [u8; 32],
        expiration: [u8; 32],
    ) -> Result<InviteRequest> {
        Ok(InviteRequest {
            invite: hex::encode(invite_signature),
            pool_id: self.pool_id,
            url: self.url,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| anyhow::anyhow!("System time error: {}", e))?
                .as_secs(),
            expiration,
            nonce,
        })
    }
}

/// Metadata for an invite request that includes worker information
#[derive(Debug, Clone)]
pub struct InviteMetadata {
    pub worker_wallet_address: Address,
    pub worker_p2p_id: String,
    pub worker_addresses: Vec<String>,
}

/// Full invite request with metadata
#[derive(Debug)]
pub struct InviteWithMetadata {
    pub metadata: InviteMetadata,
    pub request: InviteRequest,
}

/// Helper to parse InviteRequestUrl into a usable endpoint
pub fn get_endpoint_from_url(url: &InviteRequestUrl, path: &str) -> String {
    match url {
        InviteRequestUrl::MasterIpPort(ip, port) => {
            format!("http://{ip}:{port}/{path}")
        }
        InviteRequestUrl::MasterUrl(url) => {
            let url = url.trim_end_matches('/');
            format!("{url}/{path}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invite_builder_with_url() {
        let builder = InviteBuilder::with_url(1, "https://example.com".to_string());
        let signature = [0u8; 65];
        let nonce = [1u8; 32];
        let expiration = [2u8; 32];

        let invite = builder.build(signature, nonce, expiration).unwrap();
        assert_eq!(invite.pool_id, 1);
        assert!(matches!(invite.url, InviteRequestUrl::MasterUrl(_)));
    }

    #[test]
    fn test_get_endpoint_from_url() {
        let url = InviteRequestUrl::MasterUrl("https://example.com".to_string());
        assert_eq!(
            get_endpoint_from_url(&url, "heartbeat"),
            "https://example.com/heartbeat"
        );

        let ip_port = InviteRequestUrl::MasterIpPort("192.168.1.1".to_string(), 8080);
        assert_eq!(
            get_endpoint_from_url(&ip_port, "heartbeat"),
            "http://192.168.1.1:8080/heartbeat"
        );
    }
}
