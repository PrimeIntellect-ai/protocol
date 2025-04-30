use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time::timeout;

use stun::agent::*;
use stun::client::*;
use stun::message::*;
use stun::xoraddr::*;
use stun::Error;

use tracing::{debug, error, info};

pub struct StunCheck {
    pub timeout: Duration,
    pub port: u16,
}

impl StunCheck {
    pub fn new(timeout: Duration, port: u16) -> Self {
        Self { timeout, port }
    }

    async fn get_ip_from_stun_server_example_pattern(
        &self,
        server: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let bind_addr = format!("0.0.0.0:{}", self.port);
        let conn = UdpSocket::bind(bind_addr).await?;
        let local_addr = conn.local_addr()?;
        debug!("STUN local address for {}: {}", server, local_addr);

        let server_addr = tokio::net::lookup_host(server)
            .await?
            .next()
            .ok_or_else(|| format!("DNS resolution failed for {}", server))?;
        debug!("STUN server {} resolved to {}", server, server_addr);

        conn.connect(server_addr).await?;
        debug!("STUN UDP socket connected to {}", server_addr);

        let (handler_tx, mut handler_rx) = mpsc::unbounded_channel();
        let mut client = ClientBuilder::new().with_conn(Arc::new(conn)).build()?;

        let mut msg = Message::new();
        msg.build(&[Box::<TransactionId>::default(), Box::new(BINDING_REQUEST)])?;

        client.send(&msg, Some(Arc::new(handler_tx))).await?;
        debug!("STUN request sent to {}", server);

        let response_event = match timeout(self.timeout, handler_rx.recv()).await {
            Ok(Some(event)) => {
                debug!("STUN event received from {}", server);
                event
            }
            Ok(None) => {
                client.close().await?;
                return Err(
                    format!("STUN handler channel closed unexpectedly for {}", server).into(),
                );
            }
            Err(_) => {
                client.close().await?;
                return Err(format!("Timeout waiting for STUN response from {}", server).into());
            }
        };

        let response_msg = match response_event.event_body {
            Ok(msg) => msg,
            Err(e) => {
                client.close().await?;
                return Err(format!("Error in STUN event body from {}: {}", server, e).into());
            }
        };

        if response_msg.typ != MessageType::new(METHOD_BINDING, CLASS_SUCCESS_RESPONSE) {
            if response_msg.typ == MessageType::new(METHOD_BINDING, CLASS_ERROR_RESPONSE) {
                error!(
                    "STUN error response from {}: {:?}",
                    server, response_msg.typ
                );
            }
            client.close().await?;
            return Err(format!(
                "Received unexpected STUN message type from {}: {:?}",
                server, response_msg.typ
            )
            .into());
        }

        let mut xor_addr = XorMappedAddress::default();
        xor_addr.get_from(&response_msg)?;
        let public_ip = xor_addr.ip;

        info!(
            "STUN public IP {} ({}) obtained from {}",
            public_ip, xor_addr, server
        );

        client.close().await?;
        Ok(public_ip.to_string())
    }

    pub async fn get_public_ip(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let stun_servers = [
            "stun.l.google.com:19302",
            "stun.stunprotocol.org:3478",
            "stun.cloudflare.com:3478",
            "stun.ekiga.net:3478",
            "stun.ideasip.com:3478",
        ];

        let mut last_error: Option<Box<dyn std::error::Error + Send + Sync>> = None;

        for server in stun_servers {
            match self.get_ip_from_stun_server_example_pattern(server).await {
                Ok(ip) => return Ok(ip),
                Err(e) => {
                    error!("STUN failed attempt with {}: {}", server, e);
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            "Failed to get public IP from any STUN server (no specific error)".into()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::test;

    #[tokio::test]
    async fn test_get_public_ip() {
        let stun_check = StunCheck::new(Duration::from_secs(5), 0);
        let public_ip = stun_check.get_public_ip().await.unwrap();
        println!("Public IP: {}", public_ip);
        assert!(!public_ip.is_empty());
    }
}
