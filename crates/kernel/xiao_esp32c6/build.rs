fn main() {
    let ld_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("link");
    println!(
        "cargo:rustc-link-arg=-T{}",
        ld_dir.join("xiao-esp32c6.x").display()
    );
    println!("cargo:rerun-if-changed=link/xiao-esp32c6.x");

    // Re-run if WiFi credentials change.
    println!("cargo:rerun-if-env-changed=VEEROS_WIFI_SSID");
    println!("cargo:rerun-if-env-changed=VEEROS_WIFI_PASS");
}
