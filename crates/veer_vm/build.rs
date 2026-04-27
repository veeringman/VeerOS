use std::env;
use std::path::PathBuf;

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    if target_os == "macos" {
        println!("cargo:rerun-if-changed=veer-vm.entitlements");
        println!(
            "cargo:rustc-env=VEER_VM_ENTITLEMENTS={}",
            PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
                .join("veer-vm.entitlements")
                .display()
        );
    }
}
