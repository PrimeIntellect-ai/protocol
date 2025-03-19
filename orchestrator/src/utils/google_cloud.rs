use anyhow::Result;
use base64::{engine::general_purpose, Engine as _};
use google_cloud_storage::client::google_cloud_auth::credentials::CredentialsFile;
use google_cloud_storage::client::{Client, ClientConfig};
use google_cloud_storage::sign::{SignedURLMethod, SignedURLOptions};
use std::time::Duration;

pub async fn generate_upload_signed_url(
    bucket: &str,
    object_path: &str,
    credentials_base64: &str,
    content_type: Option<String>,
    expiration: Duration,
    max_bytes: Option<u64>, // Maximum file size in bytes
) -> Result<String> {
    // Decode base64 to JSON string
    let credentials_json = general_purpose::STANDARD.decode(credentials_base64)?;
    let credentials_str = String::from_utf8(credentials_json)?;

    // Create client config directly from the JSON string
    let credentials = CredentialsFile::new_from_str(&credentials_str)
        .await
        .unwrap();
    let config = ClientConfig::default()
        .with_credentials(credentials)
        .await
        .unwrap();
    let client = Client::new(config);

    // Set options for the signed URL
    let mut options = SignedURLOptions {
        method: SignedURLMethod::PUT,
        expires: expiration,
        content_type,
        ..Default::default()
    };

    // Set max bytes if specified
    if let Some(bytes) = max_bytes {
        options.headers = vec![format!("content-length:{}", bytes)];
    }

    // Generate the signed URL
    let signed_url = client
        .signed_url(bucket, object_path, None, None, options)
        .await?;
    Ok(signed_url)
}
