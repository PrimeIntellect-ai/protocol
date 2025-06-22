fn main() {
    // If WORKER_VERSION is set during the build (e.g., in CI),
    // pass it to the rustc compiler.
    if let Ok(version) = std::env::var("WORKER_VERSION") {
        println!("cargo:rustc-env=WORKER_VERSION={}", version);
    }
    if let Ok(rpc_url) = std::env::var("WORKER_RPC_URL") {
        println!("cargo:rustc-env=WORKER_RPC_URL={}", rpc_url);
    }
}
