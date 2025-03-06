use base64::{Engine as _, engine::general_purpose};
use google_cloud_storage::sign::{SignedURLMethod, SignedURLOptions};
use anyhow::Result;
use std::time::Duration;
use google_cloud_storage::client::{Client, ClientConfig};
use google_cloud_storage::client::google_cloud_auth::credentials::CredentialsFile;

pub async fn generate_upload_signed_url(
    bucket: &str, 
    object_path: &str, 
    credentials_base64: &str,
    content_type: Option<String>,
    expiration: Duration
) -> Result<String> {
    // Decode base64 to JSON string
    let credentials_json = general_purpose::STANDARD.decode(credentials_base64)?;
    let credentials_str = String::from_utf8(credentials_json)?;
    println!("credentials_str: {}", credentials_str);
    
    // Create client config directly from the JSON string
    let credentials = CredentialsFile::new_from_str(&credentials_str).await.unwrap();
    let config = ClientConfig::default().with_credentials(credentials).await.unwrap();
    let client = Client::new(config);

  
    
    // Set options for the signed URL
    let options = SignedURLOptions {
        method: SignedURLMethod::PUT,
        expires: expiration,
        content_type,
        ..Default::default()
    };
    
    // Generate the signed URL
    let signed_url = client.signed_url(bucket, object_path, None, None, options).await?;
    println!("signed_url: {}", signed_url);
    Ok(signed_url)
}
