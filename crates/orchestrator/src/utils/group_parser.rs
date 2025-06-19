use anyhow::{Error, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupInformation {
    pub group_file_name: String,
    pub prefix: String,
    pub group_id: String,
    pub group_size: u32,
    pub file_number: u32,
    pub node_index: String,
}

impl FromStr for GroupInformation {
    type Err = Error;

    fn from_str(file_name: &str) -> Result<Self, Self::Err> {
        let re = Regex::new(r".*?-([0-9a-fA-F]+)-(\d+)-(\d+)-(\d+)(\.[^.]+)$")
            .map_err(|e| Error::msg(format!("Failed to compile regex: {}", e)))?;

        let caps = re
            .captures(file_name)
            .ok_or_else(|| Error::msg("File name does not match expected group format"))?;

        let groupid_start = caps
            .get(1)
            .ok_or_else(|| Error::msg("Failed to extract group ID"))?
            .start();
        let prefix = file_name[..groupid_start - 1].to_string();

        let group_id = caps
            .get(1)
            .ok_or_else(|| Error::msg("Failed to extract group ID"))?
            .as_str()
            .to_string();

        let group_size = caps
            .get(2)
            .ok_or_else(|| Error::msg("Failed to extract group size"))?
            .as_str()
            .parse::<u32>()
            .map_err(|e| Error::msg(format!("Failed to parse group size: {}", e)))?;

        let file_number = caps
            .get(3)
            .ok_or_else(|| Error::msg("Failed to extract file number"))?
            .as_str()
            .parse::<u32>()
            .map_err(|e| Error::msg(format!("Failed to parse file number: {}", e)))?;

        let node_index = caps
            .get(4)
            .ok_or_else(|| Error::msg("Failed to extract node index"))?
            .as_str()
            .to_string();

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
            group_id,
            group_size,
            file_number,
            node_index,
        })
    }
}

impl GroupInformation {
    pub fn is_group_file(file_name: &str) -> bool {
        let re = Regex::new(r".*?-([0-9a-fA-F]+)-(\d+)-(\d+)-(\d+)(\.[^.]+)$");
        match re {
            Ok(regex) => regex.is_match(file_name),
            Err(_) => false,
        }
    }

    pub fn extract_group_id_quick(file_name: &str) -> Option<String> {
        let re = Regex::new(r".*?-([0-9a-fA-F]+)-(\d+)-(\d+)-(\d+)(\.[^.]+)$");
        match re {
            Ok(regex) => regex
                .captures(file_name)
                .and_then(|caps| caps.get(1))
                .map(|m| m.as_str().to_string()),
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_group_file_name() {
        let file_name = "Qwen/Qwen3-14B-abc123def-4-1-0.json";
        let group_info: GroupInformation = file_name.parse().unwrap();

        assert_eq!(group_info.prefix, "Qwen/Qwen3-14B");
        assert_eq!(group_info.group_id, "abc123def");
        assert_eq!(group_info.group_size, 4);
        assert_eq!(group_info.file_number, 1);
        assert_eq!(group_info.node_index, "0");
        assert_eq!(
            group_info.group_file_name,
            "Qwen/Qwen3-14B-abc123def-4-1.json"
        );
    }

    #[test]
    fn test_is_group_file() {
        assert!(GroupInformation::is_group_file("prefix-abc123-4-1-0.json"));
        assert!(GroupInformation::is_group_file(
            "complex/path-def456-8-2-3.txt"
        ));
        assert!(!GroupInformation::is_group_file("regular-file.json"));
        assert!(!GroupInformation::is_group_file("incomplete-abc123-4.json"));
    }

    #[test]
    fn test_extract_group_id_quick() {
        assert_eq!(
            GroupInformation::extract_group_id_quick("prefix-abc123-4-1-0.json"),
            Some("abc123".to_string())
        );
        assert_eq!(
            GroupInformation::extract_group_id_quick("regular-file.json"),
            None
        );
    }
}
