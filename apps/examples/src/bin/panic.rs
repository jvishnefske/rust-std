fn main() {
    println!("about to panic");
    let v: Vec<i32> = Vec::new();
    let i = std::env::args().count() + 10;
    println!("{}", v[i]);
}
