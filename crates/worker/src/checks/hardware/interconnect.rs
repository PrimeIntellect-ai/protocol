use rand::RngCore;
use reqwest::Client;
use std::time::Instant;

pub(crate) struct InterconnectCheck;

impl InterconnectCheck {
    pub(crate) async fn check_speeds() -> Result<(f64, f64), Box<dyn std::error::Error>> {
        let client = Client::new();

        // Download test: Request a 10 MB file using the query parameter.
        // Cloudflare's speed test endpoint is not officially documented or guaranteed
        // Consider using a more reliable speed test service or implementing our own test server
        let download_bytes = 10 * 1024 * 1024; // 10 MB
        let download_url = format!("https://speed.cloudflare.com/__down?bytes={download_bytes}");
        let start = Instant::now();
        let response = client.get(&download_url).send().await?;

        // Verify we got a successful response
        if !response.status().is_success() {
            return Err(format!("Speed test failed with status: {}", response.status()).into());
        }

        let data = response.bytes().await?;

        // Verify we got the expected amount of data
        if data.len() != download_bytes {
            return Err(format!(
                "Received {} bytes but expected {} bytes",
                data.len(),
                download_bytes
            )
            .into());
        }

        let elapsed = start.elapsed().as_secs_f64();
        let download_speed_mbps = (data.len() as f64 * 8.0) / (elapsed * 1_000_000.0);
        // Upload test: Generate 10 MB of random data.
        let upload_url = "https://speed.cloudflare.com/__up";
        let upload_size = 5 * 1024 * 1024; // 5 MB
        let mut rng = rand::rng();
        let mut upload_data = vec![0u8; upload_size];
        rng.fill_bytes(&mut upload_data);

        let start = Instant::now();
        let upload_result = tokio::time::timeout(
            std::time::Duration::from_secs(30), // 30 second timeout
            client
                .post(upload_url)
                .header("Content-Type", "application/octet-stream")
                .body(upload_data)
                .send(),
        )
        .await;

        let upload_speed_mbps = match upload_result {
            Ok(response) => match response {
                Ok(_) => {
                    let elapsed = start.elapsed().as_secs_f64();
                    (upload_size as f64 * 8.0) / (elapsed * 1_000_000.0)
                }
                Err(_) => 0.0,
            },
            Err(_) => {
                println!("Upload speed test timed out after 30 seconds");
                0.0
            }
        };

        Ok((download_speed_mbps, upload_speed_mbps))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_check_speeds() {
        let result = InterconnectCheck::check_speeds().await;
        println!("Test Result: {:?}", result);

        // Verify the result is Ok and contains expected tuple structure
        assert!(result.is_ok());

        let (download_speed, upload_speed) = result.unwrap();

        // Verify speeds are positive numbers
        assert!(download_speed > 0.0, "Download speed should be positive");
        assert!(upload_speed > 0.0, "Upload speed should be positive");

        // Verify speeds are within reasonable bounds (0.1 Mbps to 10000 Mbps)
        assert!(download_speed >= 0.1, "Download speed too low");
        assert!(
            download_speed <= 10000.0,
            "Download speed unreasonably high"
        );
        assert!(upload_speed >= 0.1, "Upload speed too low");
        assert!(upload_speed <= 10000.0, "Upload speed unreasonably high");
    }
}
