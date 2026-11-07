//! Build each example (plain std programs, apps/examples) for linux-none and
//! check what it prints: x86_64 natively, aarch64 natively or under qemu.

use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use harness::*;

const EXAMPLES: &[&str] = &[
    "hello",
    "args-env",
    "fs",
    "threads",
    "process",
    "net",
    "collections",
    "panic",
    "stdin",
    "smoke",
    "parallelism",
    "stack-overflow",
    "kcopy",
];

struct Env {
    dir: PathBuf,
    runner: Option<PathBuf>,
}

impl Env {
    fn run(&self, name: &str, args: &[&str], stdin: &[u8], env: &[(&str, &str)]) -> Run {
        run(&self.dir, self.runner.as_deref(), name, args, stdin, env)
    }
}

/// The examples for `arch`, or None (with a note) if they cannot run here.
fn env(arch: Arch) -> Option<Env> {
    let Some(runner) = arch.runner() else {
        eprintln!("skipping {}: set QEMU_AARCH64 or put qemu-aarch64 on PATH", arch.name());
        return None;
    };
    Some(Env { dir: examples_dir(arch), runner })
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
    let r = e.run("args-env", &["one", "two words"], b"", &[("RUST_STD_TEST", "orig")]);
    assert_eq!(r.code(), Some(0), "{}", r.stderr());
    let out = r.stdout();
    has(&out, r#"args: ["one", "two words"]"#);
    has(&out, r#"RUST_STD_TEST=Ok("orig")"#);
    has(&out, r#"after set: "changed""#);
    has(&out, "after remove: true");
    has(&out, "cwd absolute: true");
    has(&out, "cpu feature detected: true");
}

fn check_fs(e: &Env, tag: &str) {
    let dir = std::env::temp_dir().join(format!("linux_none_fs_{tag}_{}", std::process::id()));
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
    // what glibc's realpath says for the same tree
    for line in [
        r#"canonicalize a/chain2: "a/hello.txt""#,
        r#"canonicalize a/./b/../chain2: "a/hello.txt""#,
        r#"canonicalize a/dirlink/: "a/b/c""#,
        r#"canonicalize a/dirlink/..: "a/b""#,
        "canonicalize a/dirlink/../../a//b/c/: NotFound",
        r#"canonicalize a/abs/c/moved.txt: "a/b/c/moved.txt""#,
        "canonicalize a/hello.txt/: NotADirectory",
        "canonicalize a/hello.txt/..: NotADirectory",
        "canonicalize a/chain2/: NotADirectory",
        "canonicalize a/dangling: NotFound",
        "canonicalize a/loop1: FilesystemLoop",
        "canonicalize a/missing/..: NotFound",
        r#"canonicalize a/b/c/../../../a/b: "a/b""#,
        r#"canonicalize relative: Some("a/b/c/moved.txt")"#,
        r#"canonicalize root: "/" empty: Err(NotFound)"#,
    ] {
        has(&out, line);
    }
    has(&out, "cleaned up: true");
    assert!(!dir.exists());
}

fn check_kcopy(e: &Env, tag: &str) {
    let dir = std::env::temp_dir().join(format!("linux_none_kcopy_{tag}_{}", std::process::id()));
    let r = e.run("kcopy", &[dir.to_str().unwrap()], b"", &[]);
    assert_eq!(r.code(), Some(0), "{}", r.stderr());
    let out = r.stdout();
    for line in [
        "file->file: copied=300000 ok=true",
        "file->file offset+take: copied=200000 ok=true",
        "file->append: copied=300000 ok=true",
        "file->devnull: copied=300000",
        "pipe->pipe: copied=300000 ok=true",
        "pipe->file: copied=300000 ok=true",
        "child->file: copied=300000 ok=true",
        "child status: exit status: 0",
        "file->tcp: copied=300000 ok=true",
        "file->unix->file: copied=300000 ok=true",
    ] {
        has(&out, line);
    }
    assert!(!dir.exists());
}

/// Under strace (when it is installed and allowed to trace), the copies in
/// `kcopy` really go through `copy_file_range`, `sendfile` and `splice`.
fn check_kcopy_syscalls(e: &Env, tag: &str) {
    let Some(strace) = which("strace") else {
        eprintln!("skipping kcopy syscall check: no strace on PATH");
        return;
    };
    let dir = std::env::temp_dir().join(format!("linux_none_kcopy_strace_{tag}_{}", std::process::id()));
    let log = std::env::temp_dir().join(format!("linux_none_kcopy_strace_{tag}_{}.log", std::process::id()));
    // only the main thread (no -f): the copies run there, and `cat` (which
    // may splice too) stays out of the log
    let status = Command::new(strace)
        .args(["-qq", "-e", "trace=copy_file_range,sendfile,splice", "-o"])
        .arg(&log)
        .arg(e.dir.join("kcopy"))
        .arg(&dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .unwrap();
    let trace = std::fs::read_to_string(&log).unwrap_or_default();
    let _ = std::fs::remove_file(&log);
    if !status.status.success() && trace.is_empty() {
        eprintln!("skipping kcopy syscall check: strace failed: {}", String::from_utf8_lossy(&status.stderr));
        return;
    }
    assert!(status.status.success(), "kcopy failed under strace: {}", String::from_utf8_lossy(&status.stderr));
    for call in ["copy_file_range", "sendfile", "splice"] {
        let used = trace.lines().any(|l| {
            l.starts_with(&format!("{call}(")) && l.rsplit_once(" = ").is_some_and(|(_, r)| r.split(' ').next().unwrap().parse::<u64>().is_ok_and(|n| n > 0))
        });
        assert!(used, "kcopy never moved data with {call}:\n{trace}");
    }
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
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
        "pthread ids: stable=true distinct=true into=true",
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
    // no resolver: only numeric addresses and localhost
    has(&out, "dns: Uncategorized");
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
    has(&r.stderr(), "panicked at examples/src/bin/panic.rs");
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

fn check_stack_overflow(e: &Env) {
    const SIGABRT: i32 = 6;
    const SIGSEGV: i32 = 11;
    for (how, name) in [("main", "main"), ("thread", "deep")] {
        let r = e.run("stack-overflow", &[how], b"", &[]);
        let err = r.stderr();
        assert_eq!(r.output.status.signal(), Some(SIGABRT), "{how}: {:?}\n{err}", r.output.status);
        has(&err, &format!("\nthread '{name}' ("));
        has(&err, ") has overflowed its stack\nfatal runtime error: stack overflow");
    }
    // other faults, and SIGSEGV sent with kill, keep their default action
    for how in ["segv", "kill"] {
        let r = e.run("stack-overflow", &[how], b"", &[]);
        let err = r.stderr();
        assert_eq!(r.output.status.signal(), Some(SIGSEGV), "{how}: {:?}\n{err}", r.output.status);
        assert!(!err.contains("overflowed") && !err.contains("still alive"), "{how}: {err}");
    }
}

fn check_parallelism(e: &Env) {
    // the harness and the example share a cgroup, so host std must agree
    let host = std::thread::available_parallelism().unwrap().get();
    let r = e.run("parallelism", &[], b"", &[]);
    assert_eq!(r.code(), Some(0), "{}", r.stderr());
    assert_eq!(r.stdout().trim().parse::<usize>().unwrap(), host);

    // under a CPU quota of two cores, in a transient systemd unit
    let quota = |cmd: &[&Path]| {
        Command::new("systemd-run")
            .args(["--user", "--quiet", "--wait", "--pipe", "--collect", "-p", "CPUQuota=200%"])
            .args(cmd)
            .stdin(std::process::Stdio::null())
            .output()
    };
    if !quota(&[Path::new("true")]).is_ok_and(|o| o.status.success()) {
        eprintln!("skipping the cgroup quota check: systemd-run --user -p CPUQuota is not available");
        return;
    }
    let exe = e.dir.join("parallelism");
    let mut cmd: Vec<&Path> = e.runner.iter().map(|r| r.as_path()).collect();
    cmd.push(&exe);
    let o = quota(&cmd).unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let n: usize = String::from_utf8_lossy(&o.stdout).trim().parse().unwrap();
    if host >= 2 {
        assert_eq!(n, 2, "available_parallelism under CPUQuota=200%");
    } else {
        assert!(n <= 2, "available_parallelism under CPUQuota=200%: {n}");
    }
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
    check_stack_overflow(e);
    check_parallelism(e);
    check_kcopy(e, tag);
}

fn check_smoke(e: &Env) {
    let r = e.run("smoke", &[], b"", &[]);
    assert_eq!(r.code(), Some(0), "{}", r.stderr());
    let out = r.stdout();
    for line in [
        "thread sum 6, main count 0",
        "condvar 4",
        r#"readdir ["f.txt", "link", "sub"]"#,
        "canonicalize true",
        r#"child status exit status: 7 out "child got \"ping\" FOO=Some(\"bar\")""#,
        r#"sh Some(3) "via PATH\n""#,
        "missing Err(NotFound)",
        r#"tcp echo "hello""#,
        "udp6 Ok(1)",
        "localhost Ok(",
        r#"unix pair "uds""#,
    ] {
        has(&out, line);
    }
    // four spawned threads and the main thread run their TLS destructors
    assert_eq!(out.matches("tls dtor: thread").count(), 5, "{out}");
}

fn check_static(dir: &Path, machine: u16) {
    for name in EXAMPLES {
        assert_static_elf(&dir.join(name), machine);
    }
}

fn check_arch(arch: Arch) {
    let Some(e) = env(arch) else {
        // still check that it builds and links statically
        check_static(&examples_dir(arch), arch.machine());
        return;
    };
    check_static(&e.dir, arch.machine());
    check_all(&e, arch.name());
    if e.runner.is_none() {
        check_kcopy_syscalls(&e, arch.name());
    }
    // smoke runs itself as a child process
    if arch.can_spawn_self() {
        check_smoke(&e);
    } else {
        eprintln!("skipping {} smoke: qemu-aarch64 is not registered with binfmt_misc", arch.name());
    }
}

#[test]
fn x86_64_examples() {
    check_arch(Arch::X86_64);
}

#[test]
fn aarch64_examples() {
    check_arch(Arch::Aarch64);
}
