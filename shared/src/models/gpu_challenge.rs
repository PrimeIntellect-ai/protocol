use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct GpuChallengeInitResponse {
    pub seed: String,
    pub n: u64,
    pub session_id: String,
}

// Response from setting A,B matrices
#[derive(Debug, Serialize, Deserialize)]
pub struct GpuSetABResponse {
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GpuSetABRequest {
    pub seed: String,
    pub n: u64,
}

// Response containing the commitment root
#[derive(Debug, Serialize, Deserialize)]
pub struct GpuCommitmentResponse {
    pub commitment_root: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GpuChallengeVectorResponse {
    pub challenge_vector: String, // base64 encoded float array
}

// disable snake case warning
#[allow(non_snake_case)]
// Response containing C*r result
#[derive(Debug, Serialize, Deserialize)]
pub struct GpuComputeCRResponse {
    pub Cr: String, // base64 encoded float array
}

// Single row proof structure
#[derive(Debug, Serialize, Deserialize)]
pub struct GpuRowProof {
    pub row_idx: u64,
    pub row_data: String,         // base64 encoded float array
    pub merkle_path: Vec<String>, // array of hex strings
}

// Response containing multiple row proofs
#[derive(Debug, Serialize, Deserialize)]
pub struct GpuRowProofsResponse {
    pub rows: Vec<GpuRowProof>,
}

// Individual row check result
#[derive(Debug, Serialize, Deserialize)]
pub struct GpuRowCheckResult {
    pub row_idx: u64,
    pub pass: bool,
}

// Final row check response
#[derive(Debug, Serialize, Deserialize)]
pub struct GpuMultiRowCheckResponse {
    pub all_passed: bool,
    pub results: Vec<GpuRowCheckResult>,
}

// Request to start a GPU challenge - keep everything as strings
#[derive(Debug, Serialize, Deserialize)]
pub struct GpuChallengeInitRequest {
    pub n: u64,
}

// Request to submit commitment
#[derive(Debug, Serialize, Deserialize)]
pub struct GpuCommitmentRequest {
    pub session_id: String,
    pub commitment_root: String,
}

// Request to compute C*r
#[derive(Debug, Serialize, Deserialize)]
pub struct GpuComputeCRRequest {
    pub r: String, // base64 encoded challenge vector
}

// disable snake case warning
#[allow(non_snake_case)]
// Request to get row challenge
#[derive(Debug, Serialize, Deserialize)]
pub struct GpuRowChallengeRequest {
    pub session_id: String,
    pub Cr: String, // base64 encoded result
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GpuRowChallengeResponse {
    pub freivalds_ok: bool,  // base64 encoded float array
    pub spot_rows: Vec<u64>, // array of ints
}

// Request to get row proofs
#[derive(Debug, Serialize, Deserialize)]
pub struct GpuRowProofsRequest {
    pub row_idxs: Vec<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GpuClearRequest {
    pub session_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GpuClearResponse {
    pub status: String,
}

// Request to check multiple rows
#[derive(Debug, Serialize, Deserialize)]
pub struct GpuMultiRowCheckRequest {
    pub session_id: String,
    pub rows: Vec<GpuRowProof>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GpuChallengeResponse {
    pub session_id: String,
    pub master_seed: String,
    pub n: u64,
}

// Response for status check
#[derive(Debug, Serialize, Deserialize)]
pub struct GpuChallengeStatus {
    pub session_id: String,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GpuChallengeWorkerStart {
    pub session_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GpuChallengeWorkerGetStatus {
    pub session_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GpuChallengeWorkerComputeCommitment {
    pub session_id: String,
    pub master_seed: String,
    pub n: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GpuChallengeWorkerComputeCR {
    pub session_id: String,
    pub r: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GpuChallengeWorkerComputeRowProofs {
    pub session_id: String,
    pub row_idxs: Vec<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GpuChallengeWorkerComputeRowProofsResponse {
    pub rows: Vec<GpuRowProof>,
}
