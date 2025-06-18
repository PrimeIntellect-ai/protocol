fn main() {
    // If WORKER_VERSION is set during the build (e.g., in CI),
    // pass it to the rustc compiler.
    if let Ok(version) = std::env::var("WORKER_VERSION") {
        println!("cargo:rustc-env=WORKER_VERSION={}", version);
    }

    // Set version check URLs based on build environment
    let github_releases_url = "https://api.github.com/repos/PrimeProtocol/protocol/releases/latest";
    let gcs_manifest_url =
        "https://storage.googleapis.com/protocol-repo-assets/builds/latest/manifest.json";

    println!(
        "cargo:rustc-env=VERSION_CHECK_GITHUB_URL={}",
        github_releases_url
    );
    println!("cargo:rustc-env=VERSION_CHECK_GCS_URL={}", gcs_manifest_url);
}
