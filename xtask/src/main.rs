use std::env;
use std::process::exit;

use xtask::{export_patches, sysroot};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("sysroot") => sysroot(args.iter().any(|a| a == "--force")),
        Some("export-patches") => export_patches(),
        _ => {
            eprintln!("usage: cargo xtask sysroot [--force] | export-patches");
            exit(2)
        }
    };
    if let Err(e) = r {
        eprintln!("xtask: {e}");
        exit(1);
    }
}

