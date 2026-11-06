// Link flags for this package's examples: static, non-PIE (the runtime does not
// relocate itself), no crt startup files, no libc.
// Downstream binaries need the same flags (see the crate docs in src/lib.rs).
fn main() {
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let args: &[&str] = if arch == "aarch64" {
        // linked directly with rust-lld (ld.lld flavor), see .cargo/config.toml
        &["-static", "--no-pie"]
    } else {
        // linked through the cc driver
        &["-nostartfiles", "-nostdlib", "-static", "-no-pie"]
    };
    for a in args {
        println!("cargo:rustc-link-arg-examples={a}");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
