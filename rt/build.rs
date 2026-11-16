// `linux_none`: the target is *-unknown-linux-none, where this crate is the
// whole C runtime (it defines `_start`). Elsewhere (the host, for unit tests
// and Kani) it builds without the entry point.
//
// `LINUX_NONE_SECCOMP=/abs/path`: an allowlist of syscall names that
// `_start` installs as a seccomp filter before `main` (see
// `src/seccomp.rs`). This script compiles it to a BPF program in
// `$OUT_DIR/seccomp.rs`; without it, the program is empty and nothing is
// installed.
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os == "linux" && target_env.is_empty() {
        println!("cargo:rustc-cfg=linux_none");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=LINUX_NONE_SECCOMP");

    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("seccomp.rs");
    let code = match env::var_os("LINUX_NONE_SECCOMP") {
        Some(p) if !p.is_empty() => {
            let p = PathBuf::from(p);
            if !p.is_absolute() {
                fail(&format!("LINUX_NONE_SECCOMP must be an absolute path, not {}", p.display()));
            }
            println!("cargo:rerun-if-changed={}", p.display());
            let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
            let text = fs::read_to_string(&p).unwrap_or_else(|e| fail(&format!("{}: {e}", p.display())));
            let table = syscall_table(&arch);
            let (action, nrs) = parse(&text, &table).unwrap_or_else(|e| fail(&format!("{}: {e}", p.display())));
            program(&arch, action, &nrs).unwrap_or_else(|e| fail(&format!("{}: {e}", p.display())))
        }
        _ => "pub(crate) const FILTER: &[SockFilter] = &[];\n".to_string(),
    };
    fs::write(out, code).unwrap();
}

fn fail(msg: &str) -> ! {
    eprintln!("linux_none_rt: {msg}");
    std::process::exit(1)
}

/// `syscalls/<arch>.txt`: `name number` per line.
fn syscall_table(arch: &str) -> Vec<(String, u32)> {
    let f = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("syscalls").join(format!("{arch}.txt"));
    println!("cargo:rerun-if-changed={}", f.display());
    let s = fs::read_to_string(&f).unwrap_or_else(|e| fail(&format!("{}: {e}", f.display())));
    s.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let (name, n) = l.split_once(' ').unwrap();
            (name.to_string(), n.trim().parse().unwrap())
        })
        .collect()
}

// seccomp return values (linux/seccomp.h)
const RET_KILL_PROCESS: u32 = 0x8000_0000;
const RET_TRAP: u32 = 0x0003_0000;
const RET_ERRNO: u32 = 0x0005_0000;
const RET_LOG: u32 = 0x7ffc_0000;
const RET_ALLOW: u32 = 0x7fff_0000;
const EPERM: u32 = 1;

/// Syscalls that are always allowed: without them, a process can neither
/// exit nor return from a signal handler.
const ALWAYS: &[&str] = &["exit", "exit_group", "rt_sigreturn", "restart_syscall"];

/// The allowlist format: syscall names separated by whitespace or commas,
/// `#` comments, and an optional `action: kill | trap | log | errno [N]` line
/// for syscalls not on the list (default `kill`, like systemd's
/// `SystemCallFilter=`; `errno` without N is EPERM).
fn parse(text: &str, table: &[(String, u32)]) -> Result<(u32, Vec<u32>), String> {
    let mut action = RET_KILL_PROCESS;
    let mut nrs = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap().trim();
        if let Some(a) = line.strip_prefix("action:") {
            let mut w = a.split_whitespace();
            action = match (w.next(), w.next()) {
                (Some("kill"), None) => RET_KILL_PROCESS,
                (Some("trap"), None) => RET_TRAP,
                (Some("log"), None) => RET_LOG,
                (Some("errno"), None) => RET_ERRNO | EPERM,
                (Some("errno"), Some(n)) => RET_ERRNO | n.parse::<u16>().map_err(|_| format!("bad errno `{n}`"))? as u32,
                _ => return Err(format!("bad action `{}`", a.trim())),
            };
            continue;
        }
        for name in line.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()) {
            let Some(&(_, n)) = table.iter().find(|(t, _)| t == name) else {
                return Err(format!("unknown syscall `{name}` for this architecture"));
            };
            nrs.push(n);
        }
    }
    nrs.extend(ALWAYS.iter().filter_map(|a| table.iter().find(|(t, _)| t == a)).map(|&(_, n)| n));
    nrs.sort();
    nrs.dedup();
    Ok((action, nrs))
}

/// A classic-BPF program over `struct seccomp_data` (`nr` at offset 0,
/// `arch` at 4):
///
/// ```text
///     ld  [4]; jeq AUDIT_ARCH, +1; ret KILL_PROCESS
///     ld  [0]; (x86_64: jge 0x40000000 (x32), KILL)
///     jeq nr_0, ALLOW ... jeq nr_n-1, ALLOW
///     ret <action>
/// ALLOW: ret ALLOW
/// KILL:  ret KILL_PROCESS
/// ```
fn program(arch: &str, action: u32, nrs: &[u32]) -> Result<String, String> {
    const LD_W_ABS: u16 = 0x20;
    const JEQ_K: u16 = 0x15;
    const JGE_K: u16 = 0x35;
    const RET_K: u16 = 0x06;
    let audit_arch: u32 = match arch {
        "x86_64" => 0xc000_003e,
        "aarch64" => 0xc000_00b7,
        _ => return Err(format!("no seccomp support for {arch}")),
    };
    let n = nrs.len();
    // jumps are 8-bit: the x32 check jumps over every jeq
    if n > 250 {
        return Err(format!("{n} syscalls allowed; at most 250 are supported"));
    }
    let mut p: Vec<(u16, u8, u8, u32)> = vec![(LD_W_ABS, 0, 0, 4), (JEQ_K, 1, 0, audit_arch), (RET_K, 0, 0, RET_KILL_PROCESS), (LD_W_ABS, 0, 0, 0)];
    if arch == "x86_64" {
        p.push((JGE_K, (n + 2) as u8, 0, 0x4000_0000));
    }
    for (i, &nr) in nrs.iter().enumerate() {
        p.push((JEQ_K, (n - i) as u8, 0, nr));
    }
    p.push((RET_K, 0, 0, action));
    p.push((RET_K, 0, 0, RET_ALLOW));
    p.push((RET_K, 0, 0, RET_KILL_PROCESS));

    let mut s = String::from("// generated by build.rs from $LINUX_NONE_SECCOMP\npub(crate) const FILTER: &[SockFilter] = &[\n");
    for (code, jt, jf, k) in p {
        writeln!(s, "    SockFilter {{ code: {code:#06x}, jt: {jt}, jf: {jf}, k: {k:#010x} }},").unwrap();
    }
    s.push_str("];\n");
    Ok(s)
}
