//! Build each rust_std example and check what it prints, on the host and
//! (when the target and qemu-aarch64 are available) on aarch64.

use std::path::{Path, PathBuf};

use harness::*;

const EXAMPLES: &[&str] = &["hello", "args_env", "fs", "threads", "process", "net", "collections", "panic", "stdin"];

struct Env {
    dir: PathBuf,
    runner: Option<PathBuf>,
}

impl Env {
    fn run(&self, name: &str, args: &[&str], stdin: &[u8], env: &[(&str, &str)]) -> Run {
        run(&self.dir, self.runner.as_deref(), name, args, stdin, env)
    }
}

fn host() -> Env {
    Env { dir: examples_dir(None), runner: None }
}

fn aarch64() -> Option<Env> {
    let Some(qemu) = qemu_aarch64() else {
        eprintln!("skipping aarch64: set QEMU_AARCH64 or put qemu-aarch64 on PATH");
        return None;
    };
    if !aarch64_target_installed() {
        eprintln!("skipping aarch64: run `rustup target add {AARCH64}`");
        return None;
    }
    Some(Env { dir: examples_dir(Some(AARCH64)), runner: Some(qemu) })
}

fn has(out: &str, needle: &str) {
    assert!(out.contains(needle), "expected {needle:?} in output:\n{out}");
}

fn check_hello(e: &Env) {
    let r = e.run("hello", &[], b"", &[]);
    assert_eq!(r.code(), Some(0));
    assert_eq!(r.stdout(), "Hello, world!\nsum of [1, 2, 3, 4, 5] = 15\n");
}

fn check_args_env(e: &Env) {
    let r = e.run("args_env", &["one", "two words"], b"", &[("RUST_STD_TEST", "orig")]);
    assert_eq!(r.code(), Some(0), "{}", r.stderr());
    let out = r.stdout();
    has(&out, r#"args: ["one", "two words"]"#);
    has(&out, r#"RUST_STD_TEST=Ok("orig")"#);
    has(&out, r#"after set: "changed""#);
    has(&out, "after remove: true");
    has(&out, "cwd absolute: true");
}

fn check_fs(e: &Env, tag: &str) {
    let dir = std::env::temp_dir().join(format!("rust_std_fs_{tag}_{}", std::process::id()));
    let r = e.run("fs", &[dir.to_str().unwrap()], b"", &[]);
    assert_eq!(r.code(), Some(0), "{}", r.stderr());
    let out = r.stdout();
    has(&out, r#"lines: ["line one", "line two", "line three"]"#);
    has(&out, r#"seek+read: "one""#);
    has(&out, "len=29 is_file=true is_dir=false");
    has(&out, r#"read_link: "hello.txt""#);
    has(&out, r#"read_dir: ["b", "hello.txt", "link"]"#);
    has(&out, "missing: kind=NotFound msg=No such file or directory (os error 2)");
    has(&out, r#"stem=Some("libfoo.so") ext=Some("1")"#);
    has(&out, r#"components: [CurDir, Normal("a"), ParentDir, Normal("b"), Normal("c")]"#);
    has(&out, "join: /a/b.txt");
    has(&out, "cleaned up: true");
    assert!(!dir.exists());
}

fn check_threads(e: &Env) {
    let r = e.run("threads", &[], b"", &[]);
    assert_eq!(r.code(), Some(0), "{}", r.stderr());
    let out = r.stdout();
    for line in [
        "mutex total: 80000",
        r#"names: ["worker-0", "worker-1", "worker-2", "worker-3", "worker-4", "worker-5", "worker-6", "worker-7"]"#,
        "main tls counter untouched: 0",
        r#"main thread name: Some("main")"#,
        "tls dtor ran: 1",
        "mpsc received: 400 sum=619800",
        "sync_channel sum: 1225",
        "condvar ready: true",
        "condvar timed out: true",
        "rwlock: [0, 1, 2, 3]",
        "scoped sum: 10",
        "lazy: 10 45",
        "once: init",
        "parked and unparked",
        "slept enough: true",
        "parallelism > 0: true",
        "no stack leak: true",
    ] {
        has(&out, line);
    }
}

fn check_process(e: &Env) {
    let r = e.run("process", &[], b"", &[]);
    assert_eq!(r.code(), Some(7), "{}", r.stderr());
    let out = r.stdout();
    has(&out, r#"echo: status=exit status: 0 stdout="hello from echo\n""#);
    has(&out, r#"sh: code=Some(3) stdout="out\n" stderr="err\n""#);
    has(&out, r#"env+cwd: "bar\n/\n""#);
    has(&out, r#"cat: "piped through cat" exit status: 0"#);
    has(&out, "spawn error: NotFound");
    has(&out, "killed: success=false code=None");
}

fn check_net(e: &Env) {
    let r = e.run("net", &[], b"", &[]);
    assert_eq!(r.code(), Some(0), "{}", r.stderr());
    let out = r.stdout();
    has(&out, r#"tcp echo: "HELLO TCP\n""#);
    has(&out, r#"tcp echo: "SECOND\n""#);
    has(&out, r#"udp: "ping" from a: true"#);
    has(&out, "refused: ConnectionRefused");
    has(&out, "dns: Unsupported");
}

fn check_collections(e: &Env) {
    let r = e.run("collections", &[], b"", &[]);
    assert_eq!(r.code(), Some(0), "{}", r.stderr());
    let out = r.stdout();
    has(&out, r#"("the", 3)"#);
    has(&out, "intersection: [5, 6, 7]");
    has(&out, "big: len=50000 v[400]=160000");
    has(&out, "alloc stress: kept=6667 big_sum=73400320");
    has(&out, "aligned box ok: true");
}

fn check_panic(e: &Env) {
    let r = e.run("panic", &[], b"", &[]);
    // SIGABRT: the shell convention is 128+6, but a status without a code
    // is what a signal-killed process reports
    assert_eq!(r.code(), None, "expected death by signal");
    has(&r.stdout(), "about to panic");
    has(&r.stderr(), "thread 'main' panicked at examples/panic.rs");
    has(&r.stderr(), "index out of bounds: the len is 0 but the index is 11");
}

fn check_stdin(e: &Env) {
    let r = e.run("stdin", &[], b"1\n2\n39\n", &[]);
    assert_eq!(r.code(), Some(0));
    assert_eq!(r.stdout(), "total: 42\n");
    let r = e.run("stdin", &[], b"1\nnope\n", &[]);
    assert_eq!(r.code(), Some(1));
    has(&r.stderr(), "Error: \"bad number \\\"nope\\\"");
}

fn check_all(e: &Env, tag: &str) {
    check_hello(e);
    check_args_env(e);
    check_fs(e, tag);
    check_threads(e);
    check_process(e);
    check_net(e);
    check_collections(e);
    check_panic(e);
    check_stdin(e);
}

fn check_static(dir: &Path, machine: u16) {
    for name in EXAMPLES {
        assert_static_elf(&dir.join(name), machine);
    }
}

#[test]
fn host_examples() {
    let e = host();
    check_static(&e.dir, if cfg!(target_arch = "x86_64") { EM_X86_64 } else { EM_AARCH64 });
    check_all(&e, "host");
}

#[test]
fn aarch64_examples() {
    let Some(e) = aarch64() else { return };
    check_static(&e.dir, EM_AARCH64);
    check_all(&e, "aarch64");
}
