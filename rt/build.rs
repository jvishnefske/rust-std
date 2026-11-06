// `linux_none`: the target is *-unknown-linux-none, where this crate is the
// whole C runtime. `entry`: the crate defines the process entry point
// (`_start` and friends): always on linux-none, elsewhere with the `entry`
// feature, for programs linked without crt files.
fn main() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let linux_none = os == "linux" && env.is_empty();
    if linux_none {
        println!("cargo:rustc-cfg=linux_none");
    }
    if linux_none || std::env::var_os("CARGO_FEATURE_ENTRY").is_some() {
        println!("cargo:rustc-cfg=entry");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
