#![no_std]
#![no_main]
extern crate rust_std as std;
use std::prelude::*;

std::main!(main);

fn main() {
    println!("about to panic");
    let v: Vec<i32> = Vec::new();
    let i = std::env::args().count() + 10;
    println!("{}", v[i]);
}
