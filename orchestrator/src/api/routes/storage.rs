use crate::utils::google_cloud::generate_upload_signed_url;
use crate::{api::server::AppState, utils::google_cloud::generate_mapping_file};
use actix_web::{
    web::{self, post, Data},
    HttpResponse, Scope,
};
use std::time::Duration;

#[derive(serde::Deserialize)]
pub struct RequestUploadRequest {
    pub file_name: String,
    pub file_size: u64,
    pub file_type: String,
    pub sha256: String,
}

async fn request_upload(
    request_upload: web::Json<RequestUploadRequest>,
    app_state: Data<AppState>,
) -> HttpResponse {
    let file_name = &request_upload.file_name;
    let file_size = &request_upload.file_size;
    let file_type = &request_upload.file_type;
    let sha256 = &request_upload.sha256;
    println!("request_upload: {} {} {}", file_name, file_size, file_type);

    // Get credentials from app state
    let credentials = match &app_state.s3_credentials {
        Some(creds) => creds,
        None => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": "Storage credentials not configured"
            }))
        }
    };

    if let Err(e) = generate_mapping_file(
        "protocol-development-bucket", // TODO
        credentials,
        sha256,
        file_name,
    )
    .await
    {
        log::error!("Failed to generate mapping file: {}", e);
        return HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": format!("Failed to generate mapping file: {}", e)
        }));
    }

    // Generate signed upload URL
    match generate_upload_signed_url(
        "protocol-development-bucket", // TODO: Make configurable
        file_name,
        credentials,
        Some(file_type.to_string()),
        Duration::from_secs(3600), // 1 hour expiry
        Some(*file_size),
    )
    .await
    {
        Ok(signed_url) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "signed_url": signed_url
        })),
        Err(e) => {
            log::error!("Failed to generate upload URL: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": format!("Failed to generate upload URL: {}", e)
            }))
        }
    }
}

pub fn storage_routes() -> Scope {
    web::scope("/storage").route("/request-upload", post().to(request_upload))
}
