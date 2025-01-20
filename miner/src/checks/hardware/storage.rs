use crate::console::Console;
use std::env;

const BYTES_TO_GB: f64 = 1024.0 * 1024.0 * 1024.0;

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
        let blocks = u64::from(stat.f_blocks);
        let frsize = stat.f_frsize;
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
        let blocks = u64::from(stat.f_blocks);
        let frsize = u64::from(stat.f_frsize);
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
