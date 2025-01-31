use crate::console::Console;
use libc::{statvfs, statvfs as statvfs_t};
use std::env;
use std::ffi::CString;
use std::fs;
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
pub fn find_largest_storage() -> std::io::Result<MountPoint> {
    let mut mount_points = Vec::new();
    let mounts = fs::read_to_string("/proc/mounts")?;

    for line in mounts.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }

        let path = parts[1];
        let fstype = parts[2];

        // Only consider real filesystems, skip tmpfs, devfs, etc
        if !["ext4", "xfs", "btrfs", "zfs"].contains(&fstype) {
            continue;
        }

        unsafe {
            let mut stats: statvfs_t = std::mem::zeroed();
            let path_c = CString::new(path).unwrap();
            if statvfs(path_c.as_ptr(), &mut stats) == 0 {
                let available = stats.f_bsize as u64 * stats.f_bavail as u64;

                // Only consider mount points with significant space (e.g., > 1GB)
                if available > 1_000_000_000 {
                    mount_points.push(MountPoint {
                        path: path.to_string(),
                        available_space: available,
                    });
                }
            }
        }
    }

    mount_points.sort_by_key(|m| std::cmp::Reverse(m.available_space));

    mount_points
        .first()
        .map(|m| MountPoint {
            path: m.path.clone(),
            available_space: m.available_space,
        })
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "No valid mount points found")
        })
}

#[cfg(not(target_os = "linux"))]
pub fn find_largest_storage() -> std::io::Result<MountPoint> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Finding largest storage is only supported on Linux",
    ))
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
