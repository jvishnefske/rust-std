use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn main() {
    let start = Instant::now();
    let mut last = start;
    let mut monotonic = true;
    for _ in 0..10_000 {
        let now = Instant::now();
        monotonic &= now >= last;
        last = now;
    }
    println!("monotonic: {monotonic}");

    let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    // later than 2020-01-01
    println!("realtime plausible: {}", since_epoch.as_secs() > 1_577_836_800);

    std::thread::sleep(Duration::from_millis(20));
    println!("elapsed after sleep: {}", start.elapsed() >= Duration::from_millis(20));
    let t = std::thread::spawn(Instant::now).join().unwrap();
    println!("thread clock: {}", t >= last);
}
