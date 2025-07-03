use anyhow::Error;
use regex::Regex;
use serde::{Deserialize, Serialize};
use shared::web3::contracts::implementations::work_validators::synthetic_data_validator::WorkInfo;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub(crate) enum ValidationResult {
    Accept,
    Reject,
    Crashed,
    Pending,
    Unknown,
    Invalidated,
    IncompleteGroup,
    FileNameResolutionFailed,
}

impl fmt::Display for ValidationResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationResult::Accept => write!(f, "accept"),
            ValidationResult::Reject => write!(f, "reject"),
            ValidationResult::Crashed => write!(f, "crashed"),
            ValidationResult::Pending => write!(f, "pending"),
            ValidationResult::Unknown => write!(f, "unknown"),
            ValidationResult::Invalidated => write!(f, "invalidated"),
            ValidationResult::IncompleteGroup => write!(f, "incomplete_group"),
            ValidationResult::FileNameResolutionFailed => write!(f, "filename_resolution_failed"),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct WorkValidationInfo {
    pub status: ValidationResult,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RejectionInfo {
    pub work_key: String,
    pub reason: Option<String>,
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum InvalidationType {
    Soft,
    Hard,
}

impl fmt::Display for InvalidationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InvalidationType::Soft => write!(f, "soft"),
            InvalidationType::Hard => write!(f, "hard"),
        }
    }
}

#[derive(Debug)]
pub(crate) enum ProcessWorkKeyError {
    FileNameResolutionError(String),
    ValidationPollingError(String),
    InvalidatingWorkError(String),
    MaxAttemptsReached(String),
    GenericError(anyhow::Error),
    NoMatchingToplocConfig(String),
}

impl From<anyhow::Error> for ProcessWorkKeyError {
    fn from(err: anyhow::Error) -> Self {
        ProcessWorkKeyError::GenericError(err)
    }
}

impl fmt::Display for ProcessWorkKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProcessWorkKeyError::FileNameResolutionError(msg) => {
                write!(f, "File name resolution error: {msg}")
            }
            ProcessWorkKeyError::ValidationPollingError(msg) => {
                write!(f, "Validation polling error: {msg}")
            }
            ProcessWorkKeyError::InvalidatingWorkError(msg) => {
                write!(f, "Invalidating work error: {msg}")
            }
            ProcessWorkKeyError::MaxAttemptsReached(msg) => {
                write!(f, "Max attempts reached: {msg}")
            }
            ProcessWorkKeyError::GenericError(err) => {
                write!(f, "Generic error: {err}")
            }
            ProcessWorkKeyError::NoMatchingToplocConfig(msg) => {
                write!(f, "No matching toploc config: {msg:?}")
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupInformation {
    pub group_file_name: String,
    pub prefix: String,
    pub group_id: String,
    pub group_size: u32,
    pub file_number: u32,
    pub idx: String,
}
impl FromStr for GroupInformation {
    type Err = Error;

    fn from_str(file_name: &str) -> Result<Self, Self::Err> {
        let re = Regex::new(r".*?-([0-9a-fA-F]+)-(\d+)-(\d+)-(\d+)(\.[^.]+)$")
            .map_err(|e| Error::msg(format!("Failed to compile regex: {e}")))?;

        let caps = re
            .captures(file_name)
            .ok_or_else(|| Error::msg("File name does not match expected format"))?;

        let groupid_start = caps
            .get(1)
            .ok_or_else(|| Error::msg("Failed to extract group ID"))?
            .start();
        let prefix = file_name[..groupid_start - 1].to_string();

        let groupid = caps
            .get(1)
            .ok_or_else(|| Error::msg("Failed to extract group ID"))?
            .as_str();
        let groupsize = caps
            .get(2)
            .ok_or_else(|| Error::msg("Failed to extract group size"))?
            .as_str()
            .parse::<u32>()
            .map_err(|e| Error::msg(format!("Failed to parse group size: {e}")))?;
        let filenumber = caps
            .get(3)
            .ok_or_else(|| Error::msg("Failed to extract file number"))?
            .as_str()
            .parse::<u32>()
            .map_err(|e| Error::msg(format!("Failed to parse file number: {e}")))?;
        let idx = caps
            .get(4)
            .ok_or_else(|| Error::msg("Failed to extract index"))?
            .as_str();
        let extension = caps
            .get(5)
            .ok_or_else(|| Error::msg("Failed to extract extension"))?
            .as_str();

        let group_file_name = file_name[..file_name
            .rfind('-')
            .ok_or_else(|| Error::msg("Failed to find last hyphen in filename"))?]
            .to_string()
            + extension;

        Ok(Self {
            group_file_name,
            prefix,
            group_id: groupid.to_string(),
            group_size: groupsize,
            file_number: filenumber,
            idx: idx.to_string(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct ToplocGroup {
    pub group_file_name: String,
    pub group_size: u32,
    pub file_number: u32,
    pub prefix: String,
    pub sorted_work_keys: Vec<String>,
    pub group_id: String,
}

impl From<GroupInformation> for ToplocGroup {
    fn from(info: GroupInformation) -> Self {
        Self {
            group_file_name: info.group_file_name,
            group_size: info.group_size,
            file_number: info.file_number,
            prefix: info.prefix,
            sorted_work_keys: Vec::new(),
            group_id: info.group_id,
        }
    }
}

impl GroupInformation {
    pub fn to_redis(&self) -> Result<String, anyhow::Error> {
        Ok(serde_json::to_string(self)?)
    }

    pub fn from_redis(s: &str) -> Result<Self, anyhow::Error> {
        Ok(serde_json::from_str(s)?)
    }
}

#[derive(Debug)]
pub struct ValidationPlan {
    pub single_trigger_tasks: Vec<(String, WorkInfo)>,
    pub group_trigger_tasks: Vec<ToplocGroup>,
    pub status_check_tasks: Vec<String>,
    pub group_status_check_tasks: Vec<ToplocGroup>,
}
