fn main() {
    let ld_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("link");
    println!(
        "cargo:rustc-link-arg=-T{}",
        ld_dir.join("aarch64-virt.x").display()
    );
    println!("cargo:rerun-if-changed=link/aarch64-virt.x");
}
