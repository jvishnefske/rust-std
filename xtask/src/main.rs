use std::env;
use std::process::exit;

use std::path::PathBuf;

use xtask::{Options, export_patches, libc_as_musl, musl_cfg, sysroot};

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
        Some("libc-fixup") if args.len() == 2 => libc_as_musl(&PathBuf::from(&args[1])),
        Some("musl-cfg") if args.len() == 2 => musl_cfg(&PathBuf::from(&args[1])),
        Some("export-patches") => export_patches(),
        _ => {
            eprintln!("usage: cargo xtask sysroot [--force] [--out DIR] [--no-toolchain] | libc-fixup DIR | musl-cfg DIR | export-patches");
            exit(2)
        }
    };
    if let Err(e) = r {
        eprintln!("xtask: {e}");
        exit(1);
    }
}

