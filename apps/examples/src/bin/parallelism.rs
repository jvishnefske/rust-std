//! Print `available_parallelism` (the harness also runs this under a cgroup
//! CPU quota).

fn main() {
    println!("{}", std::thread::available_parallelism().unwrap());
}
