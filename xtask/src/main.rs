use std::env;
use std::process::exit;

use std::path::PathBuf;

use xtask::{Options, export_patches, sysroot};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("sysroot") => {
            let opts = Options {
                force: args.iter().any(|a| a == "--force"),
                out: args.iter().position(|a| a == "--out").and_then(|i| args.get(i + 1)).map(PathBuf::from),
                no_toolchain: args.iter().any(|a| a == "--no-toolchain"),
            };
            sysroot(&opts)
        }
        Some("export-patches") => export_patches(),
        _ => {
            eprintln!("usage: cargo xtask sysroot [--force] [--out DIR] [--no-toolchain] | export-patches");
            exit(2)
        }
    };
    if let Err(e) = r {
        eprintln!("xtask: {e}");
        exit(1);
    }
}

