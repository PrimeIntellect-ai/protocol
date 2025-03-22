use crate::{api::server::AppState, docker::docker_manager::ContainerDetails};
use actix_web::{
    web::{self, get, post, Data},
    HttpRequest, HttpResponse, Scope,
};
use bollard::errors::Error as DockerError;
use bollard::models::ContainerStateStatusEnum;
use bollard::models::PortBinding;
use log::info;
use serde::{Deserialize, Serialize};
use serde_json::{self, json};
use shared::models::api::ApiResponse;
use shared::models::gpu_challenge::*;
use shared::models::task::{Task, TaskRequest};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

const PROVER_CONTAINER_ID: &str = "prime-core-gpu-challenge";
const TASK_NAME: &str = "gpu-challenge";
const IMAGE_NAME: &str = "matrix-prover";

// Store raw responses from the prover as strings
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GpuChallengeResult {
    // Store raw JSON strings instead of parsed values
    pub session_id: String,
    pub commitment_root: String,
    pub cr_result_json: String,
    pub row_proofs_json: String,
}

// Challenge status with results
#[derive(Debug, Clone)]
struct GpuChallengeStateData {
    session_id: Option<String>,
    container_id: Option<String>,
    status: Option<String>,
    result: Option<GpuChallengeResult>,
    error: Option<String>,
}

// Simple challenge state storage
struct ChallengeStateStorage {
    state: GpuChallengeStateData,
}

impl ChallengeStateStorage {
    fn new() -> Self {
        Self {
            state: GpuChallengeStateData {
                session_id: None,
                container_id: None,
                status: None,
                result: None,
                error: None,
            },
        }
    }

    fn new_session(&mut self, state: GpuChallengeStateData) {
        self.state = state;
    }

    fn set_status(&mut self, status: &str) {
        self.state.status = Some(status.to_string());
    }

    fn set_container_id(&mut self, container_id: &str) {
        self.state.container_id = Some(container_id.to_string());
    }

    fn set_result(&mut self, result: GpuChallengeResult) {
        self.state.result = Some(result);
    }

    fn mut_result(&mut self) -> Option<&mut GpuChallengeResult> {
        self.state.result.as_mut()
    }

    fn set_error(&mut self, error: String) {
        self.state.error = Some(error);
    }

    fn get_session_id(&self) -> Option<String> {
        self.state.session_id.clone()
    }

    fn get_status(&self) -> Option<String> {
        self.state.status.clone()
    }

    fn get_container_id(&self) -> Option<String> {
        self.state.container_id.clone()
    }

    fn wipe(&mut self) {
        self.state = GpuChallengeStateData {
            session_id: None,
            container_id: None,
            status: None,
            result: None,
            error: None,
        };
    }
}

// Thread-safe state storage
lazy_static::lazy_static! {
    static ref CURRENT_CHALLENGE: Arc<Mutex<ChallengeStateStorage>> = Arc::new(Mutex::new(ChallengeStateStorage::new()));
}

// allow unused
#[allow(dead_code)]
pub async fn start_task_via_service(app_state: Data<AppState>) {
    // Set environment variables for container
    let mut env_vars = HashMap::new();
    env_vars.insert("PORT".to_string(), "12121".to_string());

    // Launch Docker container with GPU support
    let task: Task = TaskRequest {
        image: IMAGE_NAME.to_string(),
        name: TASK_NAME.to_string(),
        env_vars: Some(env_vars),
        command: Some("/usr/bin/python3".to_string()),
        args: Some(vec!["prover.py".to_string()]),
        ports: Some(vec!["12121/tcp".to_string()]),
    }
    .into();
    let task_clone = task.clone();
    let docker_service = app_state.docker_service.clone();

    // sleep for 2 minutes
    tokio::time::sleep(tokio::time::Duration::from_secs(120)).await;

    while !docker_service.state.get_is_running().await {
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        info!("Waiting for Docker service to start");
    }

    // sleep
    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    loop {
        // don't run it while there's a pending task
        if docker_service.state.get_current_task().await.is_none() {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        info!("Waiting for previous task to finish");
    }
    docker_service
        .state
        .set_current_task(Some(task_clone))
        .await;
    // sleep for a minute
    tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;
    // Spawn a background task to wait for the container to start
    tokio::spawn(async move {
        let mut retries = 0;
        while retries < 30 {
            if let Some(current_task) = docker_service.state.get_current_task().await {
                if current_task.state == shared::models::task::TaskState::RUNNING
                    && current_task.name == TASK_NAME
                {
                    let mut state = CURRENT_CHALLENGE.lock().await;
                    state.set_status("ready");
                    break;
                }
                info!("Current task: {:?}", current_task);
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
            retries += 1;
        }
        if retries >= 30 {
            let mut state = CURRENT_CHALLENGE.lock().await;
            state.wipe();
            info!("Failed to start GPU challenge container");
            // shut down docker
            docker_service.state.set_current_task(None).await;
        }
    });
}

pub async fn start_task_via_manager(app_state: Data<AppState>) -> anyhow::Result<()> {
    let manager = app_state.docker_service.docker_manager.clone();

    // before trying to start a new container, check that there isn't a stale one already running
    match manager.get_container_details(PROVER_CONTAINER_ID).await {
        Ok(container_details) => {
            manager.remove_container(&container_details.id).await?;
            info!("Stopped stale GPU challenge container.");
        }
        Err(_) => {
            info!("No stale containers, we're safe to proceed.");
        }
    }

    let image = IMAGE_NAME.to_string();
    let container_task_id = PROVER_CONTAINER_ID.to_string();
    let has_gpu = true;
    let mut env_vars: HashMap<String, String> = HashMap::new();
    env_vars.insert("PORT".to_string(), "12121".to_string());
    let cmd = vec!["/usr/bin/python3".to_string(), "prover.py".to_string()];
    let volumes: Vec<(String, String, bool)> = Vec::new();

    let ports = Some(vec!["12121/tcp".to_string()]);

    let mut port_bindings = ::std::collections::HashMap::new();
    if let Some(ports) = ports {
        let mut next_bound_port = 20000;
        for port in ports {
            port_bindings.insert(
                port.clone(),
                Some(vec![PortBinding {
                    host_ip: Some(String::from("127.0.0.1")),
                    host_port: Some(next_bound_port.to_string()),
                }]),
            );
            next_bound_port += 1;
        }
    }

    match manager
        .start_container(
            &image,
            &container_task_id,
            Some(env_vars),
            Some(cmd),
            Some(port_bindings),
            has_gpu,
            Some(volumes),
            Some(67108864),
        )
        .await
    {
        Ok(container_id) => {
            info!("Started GPU challenge container.");
            let mut state: tokio::sync::MutexGuard<'_, ChallengeStateStorage> =
                CURRENT_CHALLENGE.lock().await;
            state.set_status("initializing");
            state.set_container_id(&container_id);
            Ok(())
        }
        Err(e) => {
            info!("Failed to start GPU challenge container: {:?}", e);
            let mut state = CURRENT_CHALLENGE.lock().await;
            state.wipe();
            Err(anyhow::anyhow!("Failed to start GPU challenge container"))
        }
    }
}

pub async fn stop_task_via_manager(app_state: Data<AppState>) -> anyhow::Result<()> {
    let manager = app_state.docker_service.docker_manager.clone();

    let state = CURRENT_CHALLENGE.lock().await;
    match state.get_container_id() {
        Some(container_is) => {
            info!("Stopping GPU challenge container.");
            match manager.remove_container(&container_is).await {
                Ok(_) => Ok(()),
                Err(e) => Err(anyhow::anyhow!(
                    "Failed to stop GPU challenge container: {:?}",
                    e
                )),
            }
        }
        None => Err(anyhow::anyhow!("No GPU challenge container to stop")),
    }
}

pub async fn get_container_status(
    app_state: Data<AppState>,
) -> Result<ContainerDetails, DockerError> {
    let manager = app_state.docker_service.docker_manager.clone();
    let state = CURRENT_CHALLENGE.lock().await;
    return manager
        .get_container_details(&state.get_container_id().unwrap())
        .await;
}


async fn prover_send<T: serde::de::DeserializeOwned>(
    endpoint: &str,
    payload: Option<serde_json::Value>,
) -> anyhow::Result<T> {
    let client = reqwest::Client::new();
    let mut builder = client.post(format!("http://localhost:20000{}", endpoint));

    if let Some(json) = payload {
        builder = builder.json(&json);
    }

    let response = builder.send().await?;

    if !response.status().is_success() {
        let error_text = response.text().await?;
        return Err(anyhow::anyhow!("Prover request failed: {}", error_text));
    }

    response.json::<T>().await.map_err(anyhow::Error::from)
}

// Start a GPU challenge
pub async fn init_container(
    challenge_req: web::Json<GpuChallengeWorkerStart>,
    app_state: Data<AppState>,
) -> HttpResponse {
    info!(
        "Received GPU challenge request: session_id={}",
        challenge_req.session_id
    );

    let session_id = challenge_req.session_id.clone();

    // if state is anything other than empty, skip
    {
        let mut state = CURRENT_CHALLENGE.lock().await;
        info!("Challenge state: {:?}", state.state);
        if state.get_status().is_some() {
            return HttpResponse::Ok().json(ApiResponse::new(
                true,
                GpuChallengeStatus {
                    session_id: state.get_session_id().unwrap(),
                    status: state.get_status().unwrap(),
                },
            ));
        } else {
            // immediately create new challenge session
            state.new_session(GpuChallengeStateData {
                session_id: Some(session_id.clone()),
                container_id: None,
                status: Some("initializing".to_string()),
                result: None,
                error: None,
            });
        }
    }

    match start_task_via_manager(app_state.clone()).await {
        Ok(_) => {
            // Spawn a background task to wait for the container to start
            tokio::spawn(async move {
                let mut retries = 0;
                while retries < 30 {
                    {
                        match get_container_status(app_state.clone()).await {
                            Ok(container_details) => {
                                if container_details.status.unwrap()
                                    == ContainerStateStatusEnum::RUNNING
                                {
                                    let mut state = CURRENT_CHALLENGE.lock().await;
                                    state.set_status("ready");
                                    break;
                                }
                            }
                            Err(e) => {
                                info!("Failed to get container status: {:?}", e);
                            }
                        }
                    }
                    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                    retries += 1;
                }
                if retries >= 30 {
                    let mut state = CURRENT_CHALLENGE.lock().await;
                    info!("Failed to start GPU challenge container");
                    // shut down docker
                    stop_task_via_manager(app_state.clone()).await.unwrap();
                    state.wipe();
                }
            });
        }
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(ApiResponse::new(false, e.to_string()));
        }
    }

    info!("Started GPU challenge container.");

    // Return success with the session ID
    HttpResponse::Ok().json(ApiResponse::new(
        true,
        GpuChallengeStatus {
            session_id,
            status: "initializing".to_string(),
        },
    ))
}

// Get challenge status
pub async fn get_status(req: HttpRequest) -> HttpResponse {
    info!("Received status request: {:?}", req);

    let state = CURRENT_CHALLENGE.lock().await;
    info!("Current session ID: {:?}", state.get_session_id());

    if let Some(status) = &state.get_status() {
        let response = GpuChallengeStatus {
            session_id: state.get_session_id().unwrap(),
            status: status.clone(),
        };
        HttpResponse::Ok().json(ApiResponse::new(true, response))
    } else {
        HttpResponse::NotFound().json(ApiResponse::new(false, "No challenge currently running."))
    }
}

pub async fn compute_commitment(
    challenge_req: web::Json<GpuChallengeWorkerComputeCommitment>,
    // app_state: Data<AppState>,
) -> HttpResponse {
    let session_id = &challenge_req.session_id;
    let n = challenge_req.n;
    let master_seed = challenge_req.master_seed.clone();

    {
        // check that session ID matches
        let state = CURRENT_CHALLENGE.lock().await;
        if session_id != state.get_session_id().as_deref().unwrap_or("") {
            return HttpResponse::NotFound().json(ApiResponse::new(
                false,
                format!("No challenge found with session ID: {}", session_id),
            ));
        }
    }

    // Call prover's setAB endpoint
    match prover_send::<GpuSetABResponse>(
        "/setAB",
        Some(json!({
            "n": n,
            "seed": master_seed
        })),
    )
    .await
    {
        Ok(_) => {
            // Get commitment from prover
            match prover_send::<GpuCommitmentResponse>("/getCommitment", None).await {
                Ok(commitment_data) => {
                    let commitment_root = commitment_data.commitment_root;

                    // Store result
                    let mut state = CURRENT_CHALLENGE.lock().await;
                    state.set_result(GpuChallengeResult {
                        session_id: session_id.clone(),
                        commitment_root: commitment_root.clone(),
                        cr_result_json: "".to_string(),
                        row_proofs_json: "".to_string(),
                    });

                    HttpResponse::Ok().json(ApiResponse::new(
                        true,
                        json!({
                            "commitment_root": commitment_root
                        }),
                    ))
                }
                Err(e) => {
                    let mut state = CURRENT_CHALLENGE.lock().await;
                    state.set_error(e.to_string());
                    HttpResponse::InternalServerError().json(ApiResponse::new(false, e.to_string()))
                }
            }
        }
        Err(e) => {
            let mut state = CURRENT_CHALLENGE.lock().await;
            state.set_error(e.to_string());
            HttpResponse::InternalServerError().json(ApiResponse::new(false, e.to_string()))
        }
    }
}

pub async fn compute_cr(
    challenge_req: web::Json<GpuChallengeWorkerComputeCR>,
    // app_state: Data<AppState>,
) -> HttpResponse {
    let session_id = &challenge_req.session_id;
    let challenge_vector = challenge_req.r.clone();

    {
        // check that session ID matches
        let state = CURRENT_CHALLENGE.lock().await;
        if session_id != state.get_session_id().as_deref().unwrap_or("") {
            return HttpResponse::NotFound().json(ApiResponse::new(
                false,
                format!("No challenge found with session ID: {}", session_id),
            ));
        }
    }

    match prover_send::<GpuComputeCRResponse>(
        "/computeCR",
        Some(json!({
            "r": challenge_vector
        })),
    )
    .await
    {
        Ok(cr_data) => {
            let cr = cr_data.Cr;

            // Store result
            let mut state = CURRENT_CHALLENGE.lock().await;
            if let Some(result) = state.mut_result() {
                result.cr_result_json = cr.clone();
            }

            HttpResponse::Ok().json(ApiResponse::new(
                true,
                json!({
                    "Cr": cr
                }),
            ))
        }
        Err(e) => {
            let mut state = CURRENT_CHALLENGE.lock().await;
            state.set_error(e.to_string());
            HttpResponse::InternalServerError().json(ApiResponse::new(false, e.to_string()))
        }
    }
}

pub async fn compute_row_proofs(
    challenge_req: web::Json<GpuChallengeWorkerComputeRowProofs>,
    // app_state: Data<AppState>,
) -> HttpResponse {
    let session_id = &challenge_req.session_id;
    let row_indices = challenge_req.row_idxs.clone();

    {
        // check that session ID matches
        let state = CURRENT_CHALLENGE.lock().await;
        if session_id != state.get_session_id().as_deref().unwrap_or("") {
            return HttpResponse::NotFound().json(ApiResponse::new(
                false,
                format!("No challenge found with session ID: {}", session_id),
            ));
        }
    }

    match prover_send::<GpuRowProofsResponse>(
        "/getRowProofs",
        Some(json!({
            "row_idxs": row_indices
        })),
    )
    .await
    {
        Ok(proof_data) => {
            let proofs_json = serde_json::to_string(&proof_data).unwrap_or_default();

            // Store result
            let mut state = CURRENT_CHALLENGE.lock().await;
            if let Some(result) = state.mut_result() {
                result.row_proofs_json = proofs_json.clone();
            }

            HttpResponse::Ok().json(ApiResponse::new(true, proof_data))
        }
        Err(e) => {
            let mut state = CURRENT_CHALLENGE.lock().await;
            state.set_error(e.to_string());
            HttpResponse::InternalServerError().json(ApiResponse::new(false, e.to_string()))
        }
    }
}

// Register the routes
pub fn gpu_challenge_routes() -> Scope {
    web::scope("/gpu-challenge")
        .route("/init-container", post().to(init_container))
        .route("/status", get().to(get_status))
        .route("/compute-commitment", post().to(compute_commitment))
        .route("/compute-cr", post().to(compute_cr))
        .route("/compute-row-proofs", post().to(compute_row_proofs))
}
