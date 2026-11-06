fn main() {
    println!("Hello, world!");
    let v: Vec<u32> = (1..=5).collect();
    println!("sum of {v:?} = {}", v.iter().sum::<u32>());
}
