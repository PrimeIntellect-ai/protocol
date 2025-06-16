use crate::console::Console;
#[cfg(target_os = "linux")]
use libc::{statvfs, statvfs as statvfs_t};
use std::env;
#[cfg(target_os = "linux")]
use std::ffi::CString;
#[cfg(target_os = "linux")]
use std::fs;
pub const BYTES_TO_GB: f64 = 1024.0 * 1024.0 * 1024.0;
pub const APP_DIR_NAME: &str = "prime-worker";

#[derive(Clone)]
pub struct MountPoint {
    pub path: String,
    pub available_space: u64,
}

#[cfg(unix)]
pub fn get_storage_info() -> Result<(f64, f64), std::io::Error> {
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };

    // Use current directory instead of hardcoded "."
    let current_dir = env::current_dir()?;
    let path_str = current_dir.to_string_lossy();

    if unsafe { libc::statvfs(path_str.as_ptr() as *const i8, &mut stat) } != 0 {
        return Err(std::io::Error::last_os_error());
    }

    #[cfg(target_os = "macos")]
    {
        #[allow(clippy::useless_conversion)]
        let blocks = u64::from(stat.f_blocks);
        let frsize = stat.f_frsize;
        #[allow(clippy::useless_conversion)]
        let bavail = u64::from(stat.f_bavail);
        let total_gb = (blocks * frsize) as f64 / BYTES_TO_GB;
        let free_gb = (bavail * frsize) as f64 / BYTES_TO_GB;
        Ok((total_gb, free_gb))
    }

    #[cfg(target_os = "linux")]
    {
        let total_gb = (stat.f_blocks * stat.f_frsize) as f64 / BYTES_TO_GB;
        let free_gb = (stat.f_bavail * stat.f_frsize) as f64 / BYTES_TO_GB;
        Ok((total_gb, free_gb))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        #[allow(clippy::useless_conversion)]
        let blocks = u64::from(stat.f_blocks);
        #[allow(clippy::useless_conversion)]
        let frsize = u64::from(stat.f_frsize);
        #[allow(clippy::useless_conversion)]
        let bavail = u64::from(stat.f_bavail);
        let total_gb = (blocks * frsize) as f64 / BYTES_TO_GB;
        let free_gb = (bavail * frsize) as f64 / BYTES_TO_GB;
        Ok((total_gb, free_gb))
    }
}

#[cfg(not(unix))]
pub fn get_storage_info() -> Result<(f64, f64), std::io::Error> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Storage detection not supported on this platform",
    ))
}
#[cfg(target_os = "linux")]
pub fn find_largest_storage() -> Option<MountPoint> {
    const VALID_FS: [&str; 4] = ["ext4", "xfs", "btrfs", "zfs"];
    const MIN_SPACE: u64 = 1_000_000_000; // 1GB minimum

    let mut mount_points = Vec::new();
    let username = std::env::var("USER").unwrap_or_else(|_| "ubuntu".to_string());

    if let Ok(mounts) = fs::read_to_string("/proc/mounts") {
        for line in mounts.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 3 {
                continue;
            }

            let mount_path = parts[1];
            let fstype = parts[2];

            // Skip if not a valid filesystem type
            if !VALID_FS.contains(&fstype) {
                continue;
            }

            // Skip system/special mounts
            if mount_path.starts_with("/proc")
                || mount_path.starts_with("/sys")
                || mount_path.starts_with("/dev")
                || mount_path.starts_with("/run")
            {
                continue;
            }

            // Check available space on this mount point
            let mut stats: statvfs_t = unsafe { std::mem::zeroed() };
            let path_c = match CString::new(mount_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            if unsafe { statvfs(path_c.as_ptr(), &mut stats) } != 0 {
                continue;
            }

            let available_space = stats.f_bsize * stats.f_bavail;
            if available_space <= MIN_SPACE {
                continue;
            }

            // Try to find the best writable location on this mount point
            if let Some(best_path) = find_best_writable_path(mount_path, &username) {
                mount_points.push(MountPoint {
                    path: best_path,
                    available_space,
                });
            }
        }
    }

    // Return the mount point with the most available space
    mount_points.into_iter().max_by_key(|m| m.available_space)
}

/// Find the best writable path on a given mount point
#[cfg(target_os = "linux")]
fn find_best_writable_path(mount_path: &str, username: &str) -> Option<String> {
    // List of potential base directories to try, in order of preference
    let potential_bases = vec![
        // User home directory (highest preference if on this mount)
        format!("{}/home/{}", mount_path.trim_end_matches('/'), username),
        // Standard application directories
        format!("{}/opt", mount_path.trim_end_matches('/')),
        format!("{}/var/lib", mount_path.trim_end_matches('/')),
        // Cloud/ephemeral storage (common in cloud environments)
        format!("{}/workspace", mount_path.trim_end_matches('/')),
        format!("{}/ephemeral", mount_path.trim_end_matches('/')),
        format!("{}/tmp", mount_path.trim_end_matches('/')),
        // Mount root as last resort
        mount_path.trim_end_matches('/').to_string(),
    ];

    for base_dir in potential_bases {
        // First check if the base directory exists and is writable
        if !test_directory_writable(&base_dir) {
            continue;
        }

        // Try to create our app directory within this base
        let app_path = if base_dir == mount_path.trim_end_matches('/') {
            // If using mount root, create the app directory directly
            format!("{}/{}", base_dir, APP_DIR_NAME)
        } else {
            // Otherwise, nest it properly
            format!("{}/{}", base_dir, APP_DIR_NAME)
        };

        // Test if we can create and write to our app directory
        if test_or_create_app_directory(&app_path) {
            return Some(app_path);
        }
    }

    None
}

/// Test if a directory is writable
#[cfg(target_os = "linux")]
fn test_directory_writable(path: &str) -> bool {
    // Check if directory exists
    if !std::path::Path::new(path).is_dir() {
        return false;
    }

    // Test write access using libc::access
    match CString::new(path) {
        Ok(path_c) => {
            let result = unsafe { libc::access(path_c.as_ptr(), libc::W_OK) };
            result == 0
        }
        Err(_) => false,
    }
}
/// Test if we can create and use our app directory
#[cfg(target_os = "linux")]
fn test_or_create_app_directory(path: &str) -> bool {
    let path_buf = std::path::Path::new(path);

    // If directory doesn't exist, try to create it
    if !path_buf.exists() && std::fs::create_dir_all(path_buf).is_err() {
        return false;
    }

    // Verify it's a directory
    if !path_buf.is_dir() {
        return false;
    }

    // Test actual write permissions by creating a temporary file
    let test_file = path_buf.join(".write_test");
    match std::fs::write(&test_file, b"test") {
        Ok(_) => {
            // Clean up test file
            let _ = std::fs::remove_file(&test_file);
            true
        }
        Err(_) => false,
    }
}

#[cfg(not(target_os = "linux"))]
pub fn find_largest_storage() -> Option<MountPoint> {
    None
}

#[cfg(target_os = "linux")]
pub fn get_available_space(path: &str) -> Option<u64> {
    let mut stats: statvfs_t = unsafe { std::mem::zeroed() };
    if let Ok(path_c) = CString::new(path) {
        if unsafe { statvfs(path_c.as_ptr(), &mut stats) } == 0 {
            let available = stats.f_bsize * stats.f_bavail;
            return Some(available);
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
pub fn get_available_space(_path: &str) -> Option<u64> {
    None
}

#[allow(dead_code)]
pub fn print_storage_info() {
    match get_storage_info() {
        Ok((total, free)) => {
            Console::title("Storage Information:");
            Console::info("Total Storage", &format!("{:.1} GB", total));
            Console::info("Free Storage", &format!("{:.1} GB", free));
        }
        Err(e) => log::error!("Storage Error: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    #[cfg(unix)]
    fn test_storage_info() {
        // Ensure we're in a directory we can read
        let test_dir = env::temp_dir();
        env::set_current_dir(test_dir).expect("Failed to change to temp directory");

        let (total, free) = get_storage_info().unwrap();
        assert!(total > 0.0, "Total storage should be greater than 0");
        assert!(free >= 0.0, "Free storage should be non-negative");
        assert!(total >= free, "Total storage should be >= free storage");
    }
}
