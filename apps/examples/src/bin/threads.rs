use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Barrier, Condvar, LazyLock, Mutex, OnceLock, RwLock};
use std::thread;
use std::time::{Duration, Instant};

thread_local! {
    static COUNTER: Cell<u32> = const { Cell::new(0) };
    static NAMES: RefCell<Vec<String>> = RefCell::new(Vec::new());
}

static LAZY: LazyLock<Vec<u32>> = LazyLock::new(|| (0..10).collect());
static ONCE: OnceLock<String> = OnceLock::new();

struct DropFlag(Arc<AtomicUsize>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
thread_local! {
    static DROP_FLAG: RefCell<Option<DropFlag>> = const { RefCell::new(None) };
}

fn main() {
    // many threads hammering one mutex
    let counter = Arc::new(Mutex::new(0u64));
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let c = Arc::clone(&counter);
            thread::Builder::new()
                .name(format!("worker-{i}"))
                .spawn(move || {
                    for _ in 0..10_000 {
                        *c.lock().unwrap() += 1;
                    }
                    COUNTER.set(COUNTER.get() + 1);
                    thread::current().name().unwrap().to_string()
                })
                .unwrap()
        })
        .collect();
    let mut names: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    names.sort();
    println!("mutex total: {}", *counter.lock().unwrap());
    println!("names: {names:?}");
    println!("main tls counter untouched: {}", COUNTER.get());
    println!("main thread name: {:?}", thread::current().name());

    // thread-local destructors run when a thread exits
    let drops = Arc::new(AtomicUsize::new(0));
    let d = drops.clone();
    thread::spawn(move || DROP_FLAG.with_borrow_mut(|f| *f = Some(DropFlag(d)))).join().unwrap();
    println!("tls dtor ran: {}", drops.load(Ordering::SeqCst));

    NAMES.with_borrow_mut(|v| v.push("x".into()));
    println!("tls refcell: {:?}", NAMES.with_borrow(|v| v.clone()));

    // channels
    let (tx, rx) = mpsc::channel();
    for id in 0..4 {
        let tx = tx.clone();
        thread::spawn(move || {
            for j in 0..100 {
                tx.send(id * 1000 + j).unwrap();
            }
        });
    }
    drop(tx);
    let got: Vec<i32> = rx.iter().collect();
    println!("mpsc received: {} sum={}", got.len(), got.iter().sum::<i32>());

    let (stx, srx) = mpsc::sync_channel(2);
    let producer = thread::spawn(move || {
        for i in 0..50 {
            stx.send(i).unwrap();
        }
    });
    let total: i32 = srx.iter().sum();
    producer.join().unwrap();
    println!("sync_channel sum: {total}");

    // condvar
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let p2 = pair.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        *p2.0.lock().unwrap() = true;
        p2.1.notify_all();
    });
    let ready = pair.1.wait_while(pair.0.lock().unwrap(), |r| !*r).unwrap();
    println!("condvar ready: {}", *ready);
    drop(ready);
    let m = Mutex::new(());
    let (_g, to) = pair.1.wait_timeout(m.lock().unwrap(), Duration::from_millis(10)).unwrap();
    println!("condvar timed out: {}", to.timed_out());

    // rwlock + barrier
    let lock = Arc::new(RwLock::new(Vec::new()));
    let barrier = Arc::new(Barrier::new(4));
    let hs: Vec<_> = (0..4)
        .map(|i| {
            let (l, b) = (lock.clone(), barrier.clone());
            thread::spawn(move || {
                b.wait();
                l.write().unwrap().push(i);
                let _len = l.read().unwrap().len();
            })
        })
        .collect();
    hs.into_iter().for_each(|h| h.join().unwrap());
    let mut v = lock.read().unwrap().clone();
    v.sort();
    println!("rwlock: {v:?}");

    // scoped threads borrowing locals
    let mut data = vec![1, 2, 3, 4];
    let total = AtomicUsize::new(0);
    thread::scope(|s| {
        for chunk in data.chunks(2) {
            let total = &total;
            s.spawn(move || total.fetch_add(chunk.iter().sum::<usize>(), Ordering::SeqCst));
        }
    });
    data.push(5);
    println!("scoped sum: {}", total.load(Ordering::SeqCst));

    // lazy statics
    let h = thread::spawn(|| LAZY.iter().sum::<u32>());
    println!("lazy: {} {}", LAZY.len(), h.join().unwrap());
    println!("once: {}", ONCE.get_or_init(|| "init".to_string()));

    // detached threads clean up after themselves
    for _ in 0..50 {
        thread::spawn(|| {});
    }

    // park/unpark
    let main = thread::current();
    let t = thread::spawn(move || main.unpark());
    thread::park();
    t.join().unwrap();
    println!("parked and unparked");

    let start = Instant::now();
    thread::sleep(Duration::from_millis(30));
    println!("slept enough: {}", start.elapsed() >= Duration::from_millis(30));
    println!("parallelism > 0: {}", thread::available_parallelism().unwrap().get() > 0);

    // thread stacks are unmapped after join and by detached threads themselves
    let maps = || std::fs::read_to_string("/proc/self/maps").unwrap().lines().count();
    let before = maps();
    for _ in 0..2000 {
        thread::spawn(|| {}).join().unwrap();
    }
    let done = Arc::new(AtomicUsize::new(0));
    for _ in 0..2000 {
        let d = done.clone();
        thread::spawn(move || {
            d.fetch_add(1, Ordering::SeqCst);
        });
    }
    while done.load(Ordering::SeqCst) < 2000 {
        thread::yield_now();
    }
    thread::sleep(Duration::from_millis(100));
    let after = maps();
    println!("no stack leak: {}", after < before + 20);
}
