use crate::console::Console; // Import Console for logging

const BYTES_TO_GB: f64 = 1024.0 * 1024.0 * 1024.0;

pub fn get_storage_info() -> Result<(u64, u64), Box<dyn std::error::Error>> {
    #[cfg(unix)]
    {
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c".".as_ptr(), &mut stat) } != 0 {
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Failed to get storage stats",
            )));
        }

        let total = (stat.f_blocks as u64) * (stat.f_frsize as u64);
        let free = (stat.f_bavail as u64) * (stat.f_frsize as u64);
        Ok((total, free))
    }

    #[cfg(not(unix))]
    {
        Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Storage detection not supported on this platform",
        )))
    }
}

pub fn print_storage_info(total_storage: u64, free_storage: u64) {
    Console::section("Storage Information:");
    Console::info(
        "Total Storage",
        &format!("{:.1} GB", total_storage as f64 / BYTES_TO_GB),
    );
    Console::info(
        "Free Storage",
        &format!("{:.1} GB", free_storage as f64 / BYTES_TO_GB),
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
        // Test that the function doesn't panic
        print_storage_info(500 * 1024 * 1024 * 1024, 100 * 1024 * 1024 * 1024); // 500GB total, 100GB free
    }
}
