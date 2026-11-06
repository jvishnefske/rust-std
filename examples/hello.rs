#![no_std]
#![no_main]
extern crate rust_std as std;
use std::prelude::*;

std::main!(main);

fn main() {
    println!("Hello, world!");
    let v: Vec<u32> = (1..=5).collect();
    println!("sum of {v:?} = {}", v.iter().sum::<u32>());
}
