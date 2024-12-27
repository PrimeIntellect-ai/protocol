use super::types::GpuInfo;

pub fn detect_gpu() -> Option<GpuInfo> {
    let output = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total,driver_version",
            "--format=csv,noheader",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let output_str = String::from_utf8_lossy(&output.stdout);
    let parts: Vec<&str> = output_str.trim().split(',').collect();

    if parts.len() != 3 {
        return None;
    }

    Some(GpuInfo {
        name: parts[0].trim().to_string(),
        memory: parse_gpu_memory(parts[1].trim()),
        cuda_version: parts[2].trim().to_string(),
    })
}

fn parse_gpu_memory(mem_str: &str) -> u64 {
    const MIB_TO_BYTES: u64 = 1024 * 1024;

    let parts: Vec<&str> = mem_str.split_whitespace().collect();
    if parts.len() != 2 {
        return 0;
    }

    parts[0]
        .parse::<u64>()
        .map(|mib| mib * MIB_TO_BYTES)
        .unwrap_or(0)
}
