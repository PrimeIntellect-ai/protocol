use crate::console::Console;
use sysinfo::System;

const BYTES_TO_GB: u64 = 1024 * 1024 * 1024;

pub fn get_memory_info(sys: &System) -> (u64, u64) {
    let total_memory = sys.total_memory();
    let free_memory = sys.available_memory();
    (total_memory, free_memory)
}

pub fn convert_to_mb(memory: u64) -> u64 {
    memory / (1024 * 1024)
}

pub fn print_memory_info(total_memory: u64, free_memory: u64) {
    let total_gb = (total_memory + BYTES_TO_GB / 2) / BYTES_TO_GB;
    let free_gb = (free_memory + BYTES_TO_GB / 2) / BYTES_TO_GB;
    Console::title("Memory Information:");
    Console::info("Total Memory", &format!("{:.1} GB", total_gb));
    Console::info("Free Memory", &format!("{:.1} GB", free_gb));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_print_memory_info() {
        // Since this is just printing, we'll test that it doesn't panic
        print_memory_info(8 * 1024 * 1024 * 1024, 4 * 1024 * 1024 * 1024); // 8GB total, 4GB free
    }
}
