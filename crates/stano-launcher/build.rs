fn main() {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is before UNIX epoch")
        .as_secs();
    println!("cargo:rustc-env=BUILD_TIMESTAMP={secs}");
    println!("cargo:rerun-if-changed=build.rs");
}
