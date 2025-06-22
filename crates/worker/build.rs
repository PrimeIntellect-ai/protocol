fn main() {
    // If WORKER_VERSION is set during the build (e.g., in CI),
    // pass it to the rustc compiler.
    if let Ok(version) = std::env::var("WORKER_VERSION") {
        println!("cargo:rustc-env=WORKER_VERSION={}", version);
    }

    println!("cargo:rustc-env=RPC_URL=https://sepolia.base.org");
}
