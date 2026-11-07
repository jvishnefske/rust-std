//! panic=unwind: catch_unwind, destructors running during unwinding, panics
//! crossing a thread join, resume_unwind and nested panics. Under panic=abort
//! the first panic aborts the process.

use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

static DROPS: AtomicUsize = AtomicUsize::new(0);

struct Guard(&'static str);

impl Drop for Guard {
    fn drop(&mut self) {
        DROPS.fetch_add(1, Ordering::SeqCst);
        println!("dropped {}", self.0);
    }
}

#[inline(never)]
fn deep(n: u32) -> u32 {
    let _g = Guard(if n == 0 { "innermost" } else { "frame" });
    if n == 0 {
        panic!("boom at the bottom");
    }
    deep(n - 1) + 1
}

fn main() {
    println!("strategy unwind: {}", cfg!(panic = "unwind"));
    // keep the default hook's messages off stdout, but count them
    let hook_calls = Arc::new(AtomicUsize::new(0));
    let h = hook_calls.clone();
    panic::set_hook(Box::new(move |info| {
        h.fetch_add(1, Ordering::SeqCst);
        eprintln!("hook: {info}");
    }));

    let r = panic::catch_unwind(|| deep(5));
    let msg = r.unwrap_err().downcast::<&str>().map(|s| *s).unwrap_or("?");
    println!("caught: {msg:?}, drops: {}", DROPS.load(Ordering::SeqCst));

    // a String payload, from a formatted panic
    let r = panic::catch_unwind(|| {
        let v: Vec<u8> = Vec::new();
        v[std::env::args().count() + 2]
    });
    let msg = r.unwrap_err().downcast::<String>().map(|s| *s).unwrap_or_default();
    println!("index panic: {msg}");

    // a panic in a thread comes back from join
    let n = std::env::args().count() + 6;
    let t = std::thread::spawn(move || -> u32 { panic!("thread {n}") });
    let msg = t.join().unwrap_err().downcast::<String>().map(|s| *s).unwrap_or_default();
    println!("thread join: {msg:?}");

    // a panic while holding a lock poisons it
    let m = Arc::new(Mutex::new(1));
    let m2 = m.clone();
    let _ = std::thread::spawn(move || {
        let _g = m2.lock().unwrap();
        panic!("poison");
    })
    .join();
    println!("poisoned: {}", m.is_poisoned());

    // resume_unwind carries any payload and skips the hook
    let before = hook_calls.load(Ordering::SeqCst);
    let r = panic::catch_unwind(|| panic::resume_unwind(Box::new(42u64)));
    println!(
        "resumed: {:?} hook skipped: {}",
        r.unwrap_err().downcast::<u64>().map(|b| *b).ok(),
        hook_calls.load(Ordering::SeqCst) == before
    );

    // catch inside catch, then rethrow to the outer one
    let r = panic::catch_unwind(|| {
        let inner = panic::catch_unwind(|| panic!("inner"));
        println!("inner caught: {}", inner.is_err());
        panic::resume_unwind(inner.unwrap_err());
    });
    println!("outer caught: {:?}", r.unwrap_err().downcast::<&str>().map(|s| *s).ok());

    // unwinding through a closure that captured state by &mut
    let mut counter = 0;
    let r = panic::catch_unwind(AssertUnwindSafe(|| {
        for i in 0..10 {
            counter += i;
            if i == 4 {
                panic!("stop");
            }
        }
    }));
    println!("partial sum: {counter} err: {}", r.is_err());

    println!("hook calls: {}", hook_calls.load(Ordering::SeqCst));

    // finally an uncaught panic: main returns with exit status 101
    let _g = Guard("main's guard");
    panic!("uncaught at the end");
}
