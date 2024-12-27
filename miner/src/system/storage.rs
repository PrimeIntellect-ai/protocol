use super::types::SystemCheckError;
use std::fs;
use std::path::Path;

const BYTES_TO_GB: f64 = 1024.0 * 1024.0 * 1024.0;

pub fn get_storage_info() -> Result<(u64, u64), SystemCheckError> {
    let path = Path::new(".");
    let fs_stats = fs::metadata(path)
        .map_err(|e| SystemCheckError::Other(format!("Failed to get storage info: {}", e)))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let statvfs = unsafe {
            let mut stat: libc::statvfs = std::mem::zeroed();
            if libc::statvfs(b".\0".as_ptr() as *const i8, &mut stat) == 0 {
                Ok(stat)
            } else {
                Err(SystemCheckError::Other("Failed to get storage stats".to_string()))
            }
        }?;

        let total = statvfs.f_blocks * statvfs.f_frsize;
        let free = statvfs.f_bavail * statvfs.f_frsize;
        Ok((total as u64, free as u64))
    }

    #[cfg(not(unix))]
    {
        Err(SystemCheckError::Other(
            "Storage detection not supported on this platform".to_string(),
        ))
    }
}

pub fn print_storage_info(total_storage: u64, free_storage: u64) {
    use colored::*;
    println!("\n{}", "Storage Information:".blue().bold());
    println!(
        "  Total Storage: {:.1} GB",
        total_storage as f64 / BYTES_TO_GB
    );
    println!(
        "  Free Storage: {:.1} GB",
        free_storage as f64 / BYTES_TO_GB
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn test_get_storage_info() {
        let (total, free) = get_storage_info().unwrap();
        assert!(total > 0, "Total storage should be greater than 0");
        assert!(free > 0, "Free storage should be greater than 0");
        assert!(total >= free, "Total storage should be >= free storage");
    }

    #[test]
    fn test_print_storage_info() {
        // Since this is just printing, we'll test that it doesn't panic
        print_storage_info(500 * 1024 * 1024 * 1024, 100 * 1024 * 1024 * 1024); // 500GB total, 100GB free
    }
}
