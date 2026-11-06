// static, non-PIE, no crt files, no libc (as for rust_std's examples)
fn main() {
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let args: &[&str] = if arch == "aarch64" {
        &["-static", "--no-pie"]
    } else {
        &["-nostartfiles", "-nostdlib", "-static", "-no-pie"]
    };
    for a in args {
        println!("cargo:rustc-link-arg-bins={a}");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
