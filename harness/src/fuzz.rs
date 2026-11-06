//! Differential fuzzer: runs the operations in `fuzz/ops.rs` against the real
//! std (here, in-process) and against rust_std (the `fuzzee` binary, as a
//! child process) and reports inputs whose transcripts differ.

use std::cell::Cell;
use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

#[allow(dead_code)]
mod ops {
    include!("../../fuzz/ops.rs");
}

pub use ops::TARGETS;

const CASE_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Ok(String),
    /// panicked (std) or died (rust_std); the message or stderr tail
    Crash(String),
    Hang,
}

impl Outcome {
    fn same(&self, other: &Outcome) -> bool {
        match (self, other) {
            (Outcome::Ok(a), Outcome::Ok(b)) => a == b,
            // both sides panicking on the same input is agreement
            (Outcome::Crash(_), Outcome::Crash(_)) => true,
            (Outcome::Hang, Outcome::Hang) => true,
            _ => false,
        }
    }
}

// ---------------- std side ----------------

thread_local! {
    static QUIET: Cell<bool> = const { Cell::new(false) };
}

/// Run a case against the real std, catching panics without printing them.
pub fn reference(target: usize, input: &[u8]) -> Outcome {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !QUIET.with(Cell::get) {
                default(info)
            }
        }));
    });
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("fuzz-ref-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    QUIET.with(|q| q.set(true));
    let r = std::panic::catch_unwind(|| ops::run(target, input, &dir));
    QUIET.with(|q| q.set(false));
    let _ = std::fs::remove_dir_all(&dir);
    match r {
        Ok(s) => Outcome::Ok(s),
        Err(p) => Outcome::Crash(
            p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default(),
        ),
    }
}

// ---------------- rust_std side ----------------

/// A running `fuzzee` process (restarted after a crash or hang).
pub struct Fuzzee {
    bin: PathBuf,
    runner: Option<PathBuf>,
    proc: Option<Proc>,
}

struct Proc {
    child: Child,
    stdin: ChildStdin,
    replies: Receiver<Option<String>>,
    stderr: Arc<Mutex<Vec<u8>>>,
}

impl Fuzzee {
    pub fn new(bin: &Path, runner: Option<&Path>) -> Fuzzee {
        Fuzzee { bin: bin.to_path_buf(), runner: runner.map(Path::to_path_buf), proc: None }
    }

    fn spawn(&self) -> Proc {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "ulimit -c 0; exec \"$@\"", "sh"]);
        if let Some(r) = &self.runner {
            cmd.arg(r);
        }
        cmd.arg(&self.bin).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().expect("failed to start fuzzee");
        let stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut err = child.stderr.take().unwrap();
        let (tx, replies) = channel();
        std::thread::spawn(move || {
            loop {
                let mut len = [0u8; 4];
                if stdout.read_exact(&mut len).is_err() {
                    let _ = tx.send(None);
                    return;
                }
                let mut buf = vec![0; u32::from_le_bytes(len) as usize];
                if stdout.read_exact(&mut buf).is_err() {
                    let _ = tx.send(None);
                    return;
                }
                if tx.send(Some(String::from_utf8_lossy(&buf).into_owned())).is_err() {
                    return;
                }
            }
        });
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let sink = stderr.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = err.read(&mut buf) {
                if n == 0 {
                    break;
                }
                sink.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });
        Proc { child, stdin, replies, stderr }
    }

    pub fn call(&mut self, target: usize, input: &[u8]) -> Outcome {
        if self.proc.is_none() {
            self.proc = Some(self.spawn());
        }
        let p = self.proc.as_mut().unwrap();
        p.stderr.lock().unwrap().clear();
        let mut req = vec![target as u8];
        req.extend_from_slice(&(input.len() as u32).to_le_bytes());
        req.extend_from_slice(input);
        let sent = p.stdin.write_all(&req).and_then(|_| p.stdin.flush()).is_ok();
        let reply = if sent { p.replies.recv_timeout(CASE_TIMEOUT) } else { Ok(None) };
        match reply {
            Ok(Some(s)) => Outcome::Ok(s),
            Ok(None) | Err(RecvTimeoutError::Disconnected) => {
                let mut p = self.proc.take().unwrap();
                let status = p.child.wait();
                // give the stderr reader a moment to collect the panic message
                std::thread::sleep(Duration::from_millis(50));
                let err = String::from_utf8_lossy(&p.stderr.lock().unwrap()).into_owned();
                Outcome::Crash(format!("{status:?}: {}", err.trim()))
            }
            Err(RecvTimeoutError::Timeout) => {
                let mut p = self.proc.take().unwrap();
                let _ = p.child.kill();
                let _ = p.child.wait();
                Outcome::Hang
            }
        }
    }
}

// ---------------- the fuzz loop ----------------

/// xorshift64*: a small deterministic generator for inputs.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

fn gen_input(rng: &mut Rng) -> Vec<u8> {
    // mostly short inputs; the decoders treat missing bytes as zero
    let len = match rng.below(4) {
        0 => rng.below(16),
        1 | 2 => rng.below(64),
        _ => rng.below(256),
    };
    (0..len).map(|_| rng.next_u64() as u8).collect()
}

pub struct Finding {
    pub target: usize,
    pub input: Vec<u8>,
    pub std: Outcome,
    pub rust_std: Outcome,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "target `{}`: rust_std differs from std", TARGETS[self.target])?;
        let hex: String = self.input.iter().map(|b| format!("{b:02x}")).collect();
        writeln!(f, "input (hex): {hex}")?;
        match (&self.std, &self.rust_std) {
            (Outcome::Ok(a), Outcome::Ok(b)) => {
                // show the transcripts up to and including the first difference
                let (al, bl): (Vec<_>, Vec<_>) = (a.lines().collect(), b.lines().collect());
                let first = al.iter().zip(&bl).position(|(x, y)| x != y).unwrap_or(al.len().min(bl.len()));
                for l in &al[..first] {
                    writeln!(f, "    {l}")?;
                }
                writeln!(f, "std:      {}", al.get(first).unwrap_or(&"<end>"))?;
                write!(f, "rust_std: {}", bl.get(first).unwrap_or(&"<end>"))
            }
            (a, b) => write!(f, "std:      {a:?}\nrust_std: {b:?}"),
        }
    }
}

/// Does rust_std disagree with std on this input?
fn check(fz: &mut Fuzzee, target: usize, input: &[u8]) -> Option<Finding> {
    let expected = reference(target, input);
    let actual = fz.call(target, input);
    (!expected.same(&actual)).then(|| Finding { target, input: input.to_vec(), std: expected, rust_std: actual })
}

/// Make a failing input smaller while it still fails.
fn shrink(fz: &mut Fuzzee, mut f: Finding) -> Finding {
    let mut budget = 2000;
    let mut progress = true;
    while progress && budget > 0 {
        progress = false;
        // remove chunks, largest first
        let mut chunk = f.input.len() / 2;
        while chunk > 0 && budget > 0 {
            let mut i = 0;
            while i + chunk <= f.input.len() && budget > 0 {
                let mut cand = f.input.clone();
                cand.drain(i..i + chunk);
                budget -= 1;
                match check(fz, f.target, &cand) {
                    Some(g) => {
                        f = g;
                        progress = true;
                    }
                    None => i += chunk,
                }
            }
            chunk /= 2;
        }
        // simplify bytes towards zero
        for i in 0..f.input.len() {
            for v in [0, f.input[i] / 2] {
                if budget == 0 || f.input[i] == v {
                    continue;
                }
                let mut cand = f.input.clone();
                cand[i] = v;
                budget -= 1;
                if let Some(g) = check(fz, f.target, &cand) {
                    f = g;
                    progress = true;
                }
            }
        }
    }
    f
}

pub struct Report {
    pub cases: usize,
    pub finding: Option<Finding>,
}

/// Fuzz one target until `iters` cases or `deadline`, stopping at the first
/// (shrunk) disagreement.
pub fn fuzz(fz: &mut Fuzzee, target: usize, seed: u64, iters: usize, deadline: Option<Instant>) -> Report {
    let mut rng = Rng::new(seed ^ (target as u64).wrapping_mul(0x9E3779B97F4A7C15));
    for i in 0..iters {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            return Report { cases: i, finding: None };
        }
        let input = gen_input(&mut rng);
        if let Some(f) = check(fz, target, &input) {
            return Report { cases: i + 1, finding: Some(shrink(fz, f)) };
        }
    }
    Report { cases: iters, finding: None }
}
