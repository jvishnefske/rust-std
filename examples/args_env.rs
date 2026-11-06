#![no_std]
#![no_main]
extern crate rust_std as std;
use std::prelude::*;

std::main!(main);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    println!("args: {args:?}");
    println!("RUST_STD_TEST={:?}", std::env::var("RUST_STD_TEST"));
    unsafe { std::env::set_var("RUST_STD_TEST", "changed") };
    println!("after set: {:?}", std::env::var("RUST_STD_TEST").unwrap());
    unsafe { std::env::remove_var("RUST_STD_TEST") };
    println!("after remove: {}", std::env::var_os("RUST_STD_TEST").is_none());
    let cwd = std::env::current_dir().unwrap();
    println!("cwd absolute: {}", cwd.is_absolute());
    println!("pid nonzero: {}", std::process::id() != 0);
}
