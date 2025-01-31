use crate::console::Console;
#[cfg(target_os = "linux")]
use libc::{statvfs, statvfs as statvfs_t};
use std::env;
#[cfg(target_os = "linux")]
use std::ffi::CString;
#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt;
pub const BYTES_TO_GB: f64 = 1024.0 * 1024.0 * 1024.0;

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
    const MIN_SPACE: u64 = 1_000_000_000;

    let mut mount_points = Vec::new();
    let username = std::env::var("USER").unwrap_or_else(|_| "ubuntu".to_string());

    if let Ok(mounts) = fs::read_to_string("/proc/mounts") {
        'mount_loop: for line in mounts.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                continue;
            }

            let path = parts[1];
            let fstype = parts[2];

            if !VALID_FS.contains(&fstype) {
                continue;
            }

            // Check available space
            unsafe {
                let mut stats: statvfs_t = std::mem::zeroed();
                let path_c = CString::new(path).unwrap();
                if statvfs(path_c.as_ptr(), &mut stats) == 0 {
                    let available = stats.f_bsize * stats.f_bavail;
                    if available <= MIN_SPACE {
                        continue;
                    }

                    // Check common writable locations
                    let base_path = path.trim_end_matches('/');
                    let paths_to_check = vec![
                        base_path.to_string(),
                        format!("{}/home/{}", base_path, username),
                        format!("{}/var/lib", base_path),
                    ];

                    println!("Paths to check: {:?}", paths_to_check);
                    for check_path in paths_to_check {
                        if let Ok(check_path_c) = CString::new(check_path.clone()) {
                            if unsafe { libc::access(check_path_c.as_ptr(), libc::W_OK) } == 0 {
                                println!("Path is writable: {}", check_path);
                                mount_points.push(MountPoint {
                                    path: check_path,
                                    available_space: available,
                                });
                                break 'mount_loop;
                            }
                        }
                    }
                }
            }
        }
    }

    mount_points.into_iter().max_by_key(|m| m.available_space)
}

#[cfg(not(target_os = "linux"))]
pub fn find_largest_storage() -> Option<MountPoint> {
    None
}

#[allow(dead_code)]
pub fn print_storage_info() {
    match get_storage_info() {
        Ok((total, free)) => {
            Console::section("Storage Information:");
            Console::info("Total Storage", &format!("{:.1} GB", total));
            Console::info("Free Storage", &format!("{:.1} GB", free));
        }
        Err(e) => Console::error(&format!("Storage Error: {}", e)),
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
