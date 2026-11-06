#![no_std]
#![no_main]
extern crate rust_std as std;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::prelude::*;

std::main!(main);

fn main() {
    let text = "the quick brown fox jumps over the lazy dog the end";
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for w in text.split_whitespace() {
        *counts.entry(w).or_insert(0) += 1;
    }
    let mut sorted: Vec<_> = counts.iter().map(|(k, v)| (*k, *v)).collect();
    sorted.sort();
    println!("counts: {sorted:?}");
    println!("the={} missing={:?}", counts["the"], counts.get("cat"));

    let a: HashSet<i32> = (0..10).collect();
    let b: HashSet<i32> = HashSet::from([5, 6, 7, 20]);
    let mut i: Vec<_> = (&a & &b).into_iter().collect();
    i.sort();
    println!("intersection: {i:?}");

    let mut big = HashMap::new();
    for n in 0..100_000u64 {
        big.insert(n, n * n);
    }
    big.retain(|k, _| k % 2 == 0);
    println!("big: len={} v[400]={}", big.len(), big[&400]);

    let bt: BTreeMap<_, _> = [(3, "c"), (1, "a"), (2, "b")].into_iter().collect();
    println!("btree: {bt:?}");
    let mut dq: VecDeque<_> = (1..=3).collect();
    dq.push_front(0);
    println!("deque: {dq:?}");

    // allocator stress: many sizes, interleaved frees, big blocks
    let mut keep = Vec::new();
    for i in 0..20_000usize {
        let s = "x".repeat(i % 300);
        if i % 3 == 0 {
            keep.push(s);
        }
    }
    let big_block = vec![7u8; 10 << 20];
    println!("alloc stress: kept={} big_sum={}", keep.len(), big_block.iter().map(|&b| b as u64).sum::<u64>());
    #[repr(align(8192))]
    struct Aligned([u8; 16]);
    let al = Box::new(Aligned([1; 16]));
    println!("aligned box ok: {}", (&*al as *const Aligned as usize).is_multiple_of(8192) && al.0[0] == 1);
}
