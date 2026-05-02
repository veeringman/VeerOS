use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    if target_os == "macos" {
        build_vmnet_shim();
        println!("cargo:rerun-if-changed=veer-vm.entitlements");
        println!(
            "cargo:rustc-env=VEER_VM_ENTITLEMENTS={}",
            PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
                .join("veer-vm.entitlements")
                .display()
        );
    }
}

fn build_vmnet_shim() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let src = manifest_dir.join("src/vmnet_shim.c");
    let obj = out_dir.join("vmnet_shim.o");
    let lib = out_dir.join("libveer_vmnet_shim.a");

    println!("cargo:rerun-if-changed={}", src.display());

    let cc = env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let status = Command::new(&cc)
        .args(["-fblocks", "-Wall", "-Wextra", "-c"])
        .arg(&src)
        .arg("-o")
        .arg(&obj)
        .status()
        .expect("running C compiler for vmnet shim");
    assert!(status.success(), "C compiler failed for vmnet shim");

    let ar = env::var("AR").unwrap_or_else(|_| "ar".to_string());
    let status = Command::new(&ar)
        .args(["crus"])
        .arg(&lib)
        .arg(&obj)
        .status()
        .expect("running ar for vmnet shim");
    assert!(status.success(), "ar failed for vmnet shim");

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=veer_vmnet_shim");
    println!("cargo:rustc-link-lib=framework=vmnet");
}
