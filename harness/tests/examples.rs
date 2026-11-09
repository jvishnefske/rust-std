//! Build each example (plain std programs, apps/examples) for linux-none and
//! check what it prints: x86_64 natively, aarch64 natively or under qemu.
//! Each runs built with `panic=abort` and again with `panic=unwind`.

use std::net::{IpAddr, ToSocketAddrs};
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
    "resolve",
    "collections",
    "panic",
    "stdin",
    "time",
    "smoke",
    "parallelism",
    "stack-overflow",
    "kcopy",
    "backtrace",
    "unwind",
    "clib",
    "clib-more",
];

struct Env {
    dir: PathBuf,
    runner: Option<PathBuf>,
    panic: Panic,
}

impl Env {
    fn run(&self, name: &str, args: &[&str], stdin: &[u8], env: &[(&str, &str)]) -> Run {
        run(&self.dir, self.runner.as_deref(), name, args, stdin, env)
    }

    fn run_unset(&self, name: &str, args: &[&str], unset: &[&str]) -> Run {
        run_unset(&self.dir, self.runner.as_deref(), name, args, b"", &[], unset)
    }
}

/// The examples for `arch`, or None (with a note) if they cannot run here.
fn env(arch: Arch, panic: Panic) -> Option<Env> {
    let Some(runner) = arch.runner() else {
        eprintln!("skipping {}: set QEMU_AARCH64 or put qemu-aarch64 on PATH", arch.name());
        return None;
    };
    Some(Env { dir: examples_dir(arch, panic), runner, panic })
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
    let r = e.run("args-env", &["one", "two words"], b"", &[("RUST_STD_TEST", "orig"), ("HOME", "/from/env")]);
    assert_eq!(r.code(), Some(0), "{}", r.stderr());
    let out = r.stdout();
    has(&out, r#"args: ["one", "two words"]"#);
    has(&out, r#"RUST_STD_TEST=Ok("orig")"#);
    has(&out, r#"after set: "changed""#);
    has(&out, "after remove: true");
    has(&out, "cwd absolute: true");
    has(&out, "cpu feature detected: true");
    has(&out, r#"home: Some("/from/env")"#);

    // without $HOME, or with it empty: /etc/passwd's entry for the user
    let want = format!("home: {:?}", passwd_home());
    let r = e.run_unset("args-env", &[], &["HOME"]);
    assert_eq!(r.code(), Some(0), "{}", r.stderr());
    has(&r.stdout(), &want);
    has(&e.run("args-env", &[], b"", &[("HOME", "")]).stdout(), &want);
}

/// The home directory `getent` reports for the current user, if it comes
/// from /etc/passwd (the only database linux-none reads).
fn passwd_home() -> Option<PathBuf> {
    let uid = String::from_utf8(Command::new("id").arg("-u").output().unwrap().stdout).unwrap();
    let out = Command::new("getent").args(["passwd", uid.trim()]).output().unwrap();
    let entry = String::from_utf8(out.stdout).unwrap();
    let entry = entry.trim_end();
    let in_files = std::fs::read_to_string("/etc/passwd").unwrap().lines().any(|l| l == entry);
    if in_files { entry.split(':').nth(5).map(PathBuf::from) } else { None }
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

/// The /etc/passwd entry of the current user, if it has one.
fn passwd_entry() -> Option<Vec<String>> {
    let uid = String::from_utf8(Command::new("id").arg("-u").output().unwrap().stdout).unwrap();
    std::fs::read_to_string("/etc/passwd")
        .ok()?
        .lines()
        .map(|l| l.split(':').map(String::from).collect::<Vec<_>>())
        .find(|f| f.len() == 7 && f[2] == uid.trim())
}

fn check_clib_more(e: &Env) {
    let r = e.run("clib-more", &[], b"", &[("CLIB_MORE", "1")]);
    assert_eq!(r.code(), Some(0), "{}", r.stderr());
    let out = r.stdout();
    for line in [
        "strchr o, world strrchr orld strstr world strstr-miss NULL\n",
        "strcmp -1 0 strncasecmp 0 strncpy [97, 98, 0, 0, 0, 85] strnlen 3 memchr 5\n",
        "strerror No such file or directory strsignal Segmentation fault\n",
        "ctype a true space true 7 true -1 0 toupper Q\n",
        "strtol -31 rest z big true erange true strtoul 511\n",
        "snprintf 78 [-42|   ab|ff  |003.1|Z|18446744073709551615|(null)|%|1.235e+04|0.0001|0x1234]\n",
        "snprintf truncated 8 long-\n",
        "fputs stdout\n",
        "pthread_key main 7 thread 0 dtor 99 delete 0\n",
        "pthread_rwlock 16 0 0\n",
        "pthread_getattr_np main true thread (true, true) true\n",
        "pthread_atfork log 11 child exit 42\n",
        r#"files mkdir 0 rename 0 link 0 readlink 1 realpath true mkfifo 0 readdir [".", "..", "b", "c", "f", "l"] rmdir 0 missing -1 errno 2"#,
        "session getsid true getpgid true ttyname_r 9 tcgetpgrp -1 errno 9\n",
        "cfmakeraw iflag 0x0 lflag 0x0 cflag 0x30\n",
        "select 0 1 time true\n",
        "getgrouplist first 12345 n true short -1 true\n",
        "langinfo UTF-8 Wednesday Dec %a %b %e %H:%M:%S %Y\n",
        "iconv open true copy true left 1 2big true other true\n",
        "environ 1 popcount 64\n",
        "sscanf 4 31 42 word! x n 17 none -1 stop 1 7\n",
        "pthread_once ran 1 [0, 0, 0, 0] again 1\n",
        "getauxval 4096 0 nanosleep 0 true\n",
        "sched_setaffinity 0\n",
    ] {
        has(&out, line);
    }
    has(&r.stderr(), "perror: No such file or directory\n");
    // a pseudo-terminal, where the system has them
    if std::path::Path::new("/dev/ptmx").exists() && !out.contains("openpty none") {
        has(&out, "openpty true ttyname_r 0 same true\n");
    }
}

fn check_clib(e: &Env) {
    let r = e.run("clib", &[], b"", &[("CLIB_TEST", "hello")]);
    assert_eq!(r.code(), Some(0), "{}", r.stderr());
    let out = r.stdout();
    for line in [
        "isatty 0 errno 25",
        "sysconf pagesize 4096",
        "sysconf cpus>0 true",
        "sysconf bogus -1 errno 22",
        "syscall getpid true",
        "getpid true",
        "mmap 42 munmap 0",
        "signal handled true",
        "sem 3 waits",
        r#"fork exec status 5 out "from-exec\n""#,
        "poll 1 revents 1",
        "epoll 1 data 7",
        "dl_iterate_phdr phnum>0 true",
        "dl_iterate_phdr objects 1",
        "getenv hello",
        "getenv missing true",
        "putenv put",
        "fstat 0 size 5",
        "statvfs 0 bsize>0 true",
        "getmntent some true root true",
        "eventfd 5",
        "tcgetattr -1 errno 25",
        "socket listen true accepting 1",
        "math 2.718282 1024 0 0.785398 3",
        "malloc realloc true calloc true memalign 0 0",
        "pthread_mutex 0 16 0 0",
        "strerror_r 0 No such file or directory short 34 No ",
        "lseek 1 5 errno 9",
        "getsockname 0 port>0 true peer -1",
        "gmtime_r 2023-11-14 22:13:20 wday 2 yday 317 UTC",
        "localtime_r posix 09:13 isdst 1 gmtoff 39600 AEDT",
    ] {
        has(&out, line);
    }
    // the zone file, where the system has one (not in Nix's build sandbox)
    let chicago = ["/usr/share/zoneinfo", "/share/zoneinfo", "/etc/zoneinfo"]
        .iter()
        .any(|d| std::path::Path::new(d).join("America/Chicago").exists());
    has(&out, if chicago { "localtime_r file 16:13 isdst 0 gmtoff -21600 CST" } else { "localtime_r file 22:13 isdst 0 gmtoff 0 UTC" });
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap();
    has(&out, &format!("gethostname {}\n", host.trim()));
    match passwd_entry() {
        Some(f) => {
            has(&out, &format!("getpwuid_r {} {} {}\n", f[0], f[2], f[5]));
            has(&out, &format!("getpwnam uid {}\n", f[2]));
            has(&out, "getpwuid_r small 34");
        }
        None => has(&out, "getpwuid_r none 0"),
    }
    let gid = String::from_utf8(Command::new("id").arg("-g").output().unwrap().stdout).unwrap();
    let group = std::fs::read_to_string("/etc/group")
        .ok()
        .and_then(|g| g.lines().map(|l| l.split(':').map(String::from).collect::<Vec<_>>()).find(|f| f.len() == 4 && f[2] == gid.trim()));
    match group {
        Some(f) => has(&out, &format!("getgrgid_r {} {}\n", f[0], f[2])),
        None => has(&out, "getgrgid_r none 0"),
    }
}

fn check_net(e: &Env) {
    let r = e.run("net", &[], b"", &[]);
    assert_eq!(r.code(), Some(0), "{}", r.stderr());
    let out = r.stdout();
    has(&out, r#"tcp echo: "HELLO TCP\n""#);
    has(&out, r#"tcp echo: "SECOND\n""#);
    has(&out, r#"udp: "ping" from a: true"#);
    has(&out, "refused: ConnectionRefused");
    // `.invalid` names never resolve, with or without a network
    has(&out, "dns: Uncategorized");
}

/// Name lookup through std, with the system's configuration: /etc/hosts,
/// and DNS if the host (glibc, in this process) can resolve a name.
fn check_resolve(e: &Env) {
    let r = e.run("resolve", &["localhost", "nonexistent.invalid", "a..b"], b"", &[]);
    assert_eq!(r.code(), Some(0), "{}", r.stderr());
    let out = r.stdout();
    has(&out, "localhost: Ok([127.0.0.1");
    has(&out, "nonexistent.invalid: Err(Uncategorized) failed to lookup address information");
    has(&out, "a..b: Err(Uncategorized) failed to lookup address information: Name does not resolve");

    const NAME: &str = "example.com";
    let host: Vec<IpAddr> = match (NAME, 0).to_socket_addrs() {
        Ok(addrs) => addrs.map(|a| a.ip()).collect(),
        Err(err) => {
            eprintln!("skipping DNS with the system resolver: the host cannot resolve {NAME}: {err}");
            return;
        }
    };
    let r = e.run("resolve", &[NAME], b"", &[]);
    let out = r.stdout();
    let ours: Vec<IpAddr> = out
        .trim()
        .strip_prefix(&format!("{NAME}: Ok(["))
        .and_then(|l| l.strip_suffix("])"))
        .unwrap_or_else(|| panic!("{NAME} does not resolve on linux-none, but does on the host ({host:?}):\n{out}"))
        .split(", ")
        .map(|a| a.parse().unwrap())
        .collect();
    // the answers may rotate, but the families match
    for v6 in [false, true] {
        assert_eq!(
            host.iter().any(|a| a.is_ipv6() == v6),
            ours.iter().any(|a| a.is_ipv6() == v6),
            "host {host:?}, linux-none {ours:?}"
        );
    }
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

/// The exit status of a process whose main thread panicked.
fn assert_panicked(e: &Env, r: &Run) {
    match e.panic {
        // SIGABRT: the shell convention is 128+6, but a status without a
        // code is what a signal-killed process reports
        Panic::Abort => assert_eq!(r.code(), None, "expected death by signal\n{}", r.stderr()),
        // the panic unwound out of main, and lang_start exits with 101
        Panic::Unwind => assert_eq!(r.code(), Some(101), "{}", r.stderr()),
    }
}

fn check_panic(e: &Env) {
    let r = e.run("panic", &[], b"", &[]);
    assert_panicked(e, &r);
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

fn check_time(e: &Env, tag: &str) {
    let r = e.run("time", &[], b"", &[]);
    assert_eq!(r.code(), Some(0), "{}", r.stderr());
    assert_eq!(
        r.stdout(),
        "monotonic: true\nrealtime plausible: true\nelapsed after sleep: true\nthread clock: true\n"
    );

    // Instant::now and SystemTime::now go through the vDSO: no clock_gettime
    // syscalls. qemu's vDSO (when it has one) makes the syscall itself, so
    // under qemu this only reports them.
    let exe = e.dir.join("time");
    let trace = std::env::temp_dir().join(format!("linux_none_time_{tag}_{}", std::process::id()));
    let status = match &e.runner {
        None => which("strace").map(|s| Command::new(s).args(["-f", "-qq", "-e", "trace=clock_gettime", "-o"]).arg(&trace).arg(&exe).output()),
        Some(qemu) => {
            Some(Command::new(qemu).arg("-strace").arg(&exe).output().inspect(|o| std::fs::write(&trace, &o.stderr).unwrap()))
        }
    };
    let log = std::fs::read_to_string(&trace);
    let _ = std::fs::remove_file(&trace);
    match (status, log) {
        (Some(Ok(o)), Ok(log)) if o.status.success() => {
            let calls = log.lines().filter(|l| l.contains("clock_gettime(")).count();
            if e.runner.is_none() {
                assert_eq!(calls, 0, "clock_gettime syscalls despite the vDSO:\n{log}");
            } else {
                eprintln!("{tag}: {calls} clock_gettime syscalls under qemu");
            }
        }
        _ => eprintln!("skipping the {tag} clock_gettime syscall check: cannot trace syscalls"),
    }
}

fn check_backtrace(e: &Env) {
    let r = e.run("backtrace", &[], b"", &[]);
    assert_eq!(r.code(), Some(0), "{}", r.stderr());
    let out = r.stdout();
    for line in [
        "status: true",
        "frames > 3: true",
        "has backtrace::inner_frame: true",
        "has backtrace::outer_frame: true",
        "has backtrace::main: true",
        "has file:line: true",
        "inner before outer: true",
    ] {
        has(&out, line);
    }

    // the panic hook's backtrace: symbolized, short (between std's
    // __rust_end_short_backtrace and __rust_begin_short_backtrace markers)
    let r = e.run("backtrace", &["panic"], b"", &[("RUST_BACKTRACE", "1")]);
    assert_panicked(e, &r);
    let err = r.stderr();
    has(&err, "index out of bounds: the len is 0 but the index is 12");
    has(&err, "stack backtrace:");
    has(&err, "core::panicking::panic_bounds_check");
    has(&err, ": backtrace::panicking_frame\n");
    has(&err, ": backtrace::main\n");
    has(&err, "examples/src/bin/backtrace.rs:20:");
    assert!(!err.contains("lang_start_internal"), "not a short backtrace:\n{err}");
    assert!(err.find("panicking_frame").unwrap() < err.find(": backtrace::main").unwrap(), "{err}");

    // RUST_BACKTRACE=full: every frame with its address, down to _start
    let r = e.run("backtrace", &["panic"], b"", &[("RUST_BACKTRACE", "full")]);
    let err = r.stderr();
    // (full symbol names carry the crate's hash: `std[0123abcd]::rt::...`)
    has(&err, "]::rt::lang_start_internal\n");
    has(&err, "]::start::start\n");
    let last_frame = err.lines().rev().find(|l| l.trim_start().split_once(": ").is_some_and(|(n, _)| n.parse::<u32>().is_ok()));
    assert!(last_frame.is_some_and(|l| l.ends_with(" - _start")), "the walk should end at _start:\n{err}");
}

fn check_unwind(e: &Env) {
    let r = e.run("unwind", &[], b"", &[]);
    assert_panicked(e, &r);
    let (out, err) = (r.stdout(), r.stderr());
    if e.panic == Panic::Abort {
        assert_eq!(out, "strategy unwind: false\n");
        has(&err, "boom at the bottom");
        return;
    }
    let expected = "\
strategy unwind: true
dropped innermost
dropped frame
dropped frame
dropped frame
dropped frame
dropped frame
caught: \"boom at the bottom\", drops: 6
index panic: index out of bounds: the len is 0 but the index is 3
thread join: \"thread 7\"
poisoned: true
resumed: Some(42) hook skipped: true
inner caught: true
outer caught: Some(\"inner\")
partial sum: 10 err: true
hook calls: 6
dropped main's guard
";
    assert_eq!(out, expected, "stderr:\n{err}");
    has(&err, "hook: panicked at examples/src/bin/unwind.rs");
    has(&err, "uncaught at the end");
}

fn check_all(e: &Env, tag: &str) {
    check_hello(e);
    check_args_env(e);
    check_fs(e, tag);
    check_threads(e);
    check_process(e);
    check_net(e);
    check_resolve(e);
    check_collections(e);
    check_panic(e);
    check_stdin(e);
    check_stack_overflow(e);
    check_parallelism(e);
    check_kcopy(e, tag);
    check_backtrace(e);
    check_unwind(e);
    check_clib(e);
    check_clib_more(e);
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

fn check_arch(arch: Arch, panic: Panic) {
    let Some(e) = env(arch, panic) else {
        // still check that it builds and links statically
        check_static(&examples_dir(arch, panic), arch.machine());
        return;
    };
    check_static(&e.dir, arch.machine());
    let tag = format!("{}_{panic:?}", arch.name());
    check_all(&e, &tag);
    if e.runner.is_none() {
        check_kcopy_syscalls(&e, &tag);
    }
    check_time(&e, &tag);
    // smoke runs itself as a child process
    if arch.can_spawn_self() {
        check_smoke(&e);
    } else {
        eprintln!("skipping {} smoke: qemu-aarch64 is not registered with binfmt_misc", arch.name());
    }
}

#[test]
fn x86_64_examples() {
    check_arch(Arch::X86_64, Panic::Abort);
}

#[test]
fn aarch64_examples() {
    check_arch(Arch::Aarch64, Panic::Abort);
}

#[test]
fn x86_64_examples_unwind() {
    check_arch(Arch::X86_64, Panic::Unwind);
}

#[test]
fn aarch64_examples_unwind() {
    check_arch(Arch::Aarch64, Panic::Unwind);
}
