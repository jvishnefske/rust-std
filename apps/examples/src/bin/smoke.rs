use std::cell::Cell;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::{env, fs, net, process, thread, time};

struct Noisy(&'static str);
impl Drop for Noisy {
    fn drop(&mut self) {
        println!("tls dtor: {}", self.0);
    }
}
thread_local! {
    static COUNT: Cell<u32> = const { Cell::new(0) };
    static NOISY: Noisy = Noisy("thread");
}

fn main() {
    if env::args().nth(1).as_deref() == Some("child") {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).unwrap();
        print!("child got {:?} FOO={:?}", s, env::var("FOO").ok());
        process::exit(7);
    }
    // threads, TLS, sync
    let pair = Arc::new((Mutex::new(0), Condvar::new()));
    let hs: Vec<_> = (0..4)
        .map(|i| {
            let pair = pair.clone();
            thread::Builder::new().name(format!("w{i}")).spawn(move || {
                NOISY.with(|_| {});
                COUNT.with(|c| c.set(c.get() + i));
                let (m, cv) = &*pair;
                *m.lock().unwrap() += 1;
                cv.notify_all();
                COUNT.with(|c| c.get())
            }).unwrap()
        })
        .collect();
    let sum: u32 = hs.into_iter().map(|h| h.join().unwrap()).sum();
    println!("thread sum {sum}, main count {}", COUNT.with(|c| c.get()));
    let (m, cv) = &*pair;
    let g = cv.wait_while(m.lock().unwrap(), |n| *n < 4).unwrap();
    println!("condvar {}", *g);
    drop(g);
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || tx.send(thread::current().name().map(String::from)).unwrap());
    println!("mpsc {:?}", rx.recv().unwrap());
    println!("parallelism {}", thread::available_parallelism().unwrap());
    // detached thread
    thread::spawn(|| thread::sleep(time::Duration::from_millis(5)));
    // hashmap (random keys)
    let mut h = HashMap::new();
    h.insert("a", 1);
    println!("hashmap {:?}", h);
    // fs
    let d = env::temp_dir().join(format!("smoke-{}", process::id()));
    fs::create_dir_all(d.join("sub")).unwrap();
    fs::write(d.join("f.txt"), b"hello fs").unwrap();
    std::os::unix::fs::symlink("f.txt", d.join("link")).unwrap();
    let mut names: Vec<_> = fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name()).collect();
    names.sort();
    println!("readdir {:?}", names);
    let md = fs::metadata(d.join("f.txt")).unwrap();
    println!("meta len {} file {} modified {}", md.len(), md.is_file(), md.modified().is_ok());
    println!("canonicalize {}", fs::canonicalize(d.join("link")).unwrap().ends_with("f.txt"));
    println!("read_link {:?}", fs::read_link(d.join("link")).unwrap());
    fs::copy(d.join("f.txt"), d.join("g.txt")).unwrap();
    println!("copy {:?}", fs::read_to_string(d.join("g.txt")).unwrap());
    fs::remove_dir_all(&d).unwrap();
    println!("removed {}", !d.exists());
    // env
    unsafe { env::set_var("FOO", "bar") };
    println!("env FOO {:?} cwd ok {}", env::var("FOO"), env::current_dir().is_ok());
    println!("current_exe {}", env::current_exe().unwrap().ends_with("smoke"));
    // process
    let mut c = process::Command::new(env::current_exe().unwrap())
        .arg("child")
        .stdin(process::Stdio::piped())
        .stdout(process::Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(b"ping").unwrap();
    let out = c.wait_with_output().unwrap();
    println!("child status {} out {:?}", out.status, String::from_utf8_lossy(&out.stdout));
    let o = process::Command::new("sh").arg("-c").arg("echo via PATH; exit 3").output().unwrap();
    println!("sh {:?} {:?}", o.status.code(), String::from_utf8_lossy(&o.stdout));
    println!("missing {:?}", process::Command::new("does-not-exist-xyz").status().map_err(|e| e.kind()));
    // net
    let l = net::TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap();
    let t = thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut b = [0; 5];
        s.read_exact(&mut b).unwrap();
        s.write_all(&b).unwrap();
    });
    let mut s = net::TcpStream::connect(a).unwrap();
    s.write_all(b"hello").unwrap();
    let mut b = [0; 5];
    s.read_exact(&mut b).unwrap();
    t.join().unwrap();
    println!("tcp echo {:?}", std::str::from_utf8(&b).unwrap());
    let u = net::UdpSocket::bind("[::1]:0").unwrap();
    u.send_to(b"x", u.local_addr().unwrap()).unwrap();
    let mut b = [0; 1];
    println!("udp6 {:?}", u.recv_from(&mut b).map(|r| r.0));
    println!("localhost {:?}", net::ToSocketAddrs::to_socket_addrs("localhost:80").map(|i| i.count()));
    let (us1, mut us2) = std::os::unix::net::UnixStream::pair().unwrap();
    (&us1).write_all(b"uds").unwrap();
    let mut b = [0; 3];
    us2.read_exact(&mut b).unwrap();
    println!("unix pair {:?}", std::str::from_utf8(&b).unwrap());
    println!("time {:?}", time::SystemTime::now().duration_since(time::UNIX_EPOCH).is_ok());
    NOISY.with(|_| {});
}
