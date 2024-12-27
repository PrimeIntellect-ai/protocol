use super::types::SystemCheckError;
use sysinfo::System;

const BYTES_TO_GB: f64 = 1024.0 * 1024.0 * 1024.0;

pub fn get_memory_info(sys: &System) -> (u64, u64) {
    let total_memory = sys.total_memory();
    let free_memory = sys.available_memory();
    (total_memory, free_memory)
}

pub fn print_memory_info(total_memory: u64, free_memory: u64) {
    use colored::*;
    println!("\n{}", "Memory Information:".blue().bold());
    println!(
        "  Total Memory: {:.1} GB",
        total_memory as f64 / BYTES_TO_GB
    );
    println!("  Free Memory: {:.1} GB", free_memory as f64 / BYTES_TO_GB);
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysinfo::System;

    #[test]
    fn test_get_memory_info() {
        let sys = System::new_all();
        let (total, free) = get_memory_info(&sys);
        assert!(total > 0, "Total memory should be greater than 0");
        assert!(free > 0, "Free memory should be greater than 0");
        assert!(total >= free, "Total memory should be >= free memory");
    }

    #[test]
    fn test_print_memory_info() {
        // Since this is just printing, we'll test that it doesn't panic
        print_memory_info(8 * 1024 * 1024 * 1024, 4 * 1024 * 1024 * 1024); // 8GB total, 4GB free
    }
}
