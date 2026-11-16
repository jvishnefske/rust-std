//! `cargo xtask syscalls [--docker FILE] [--systemd FILE] -- PROGRAM [ARGS...]`:
//! run PROGRAM under strace (following threads and children) and print the
//! distinct syscalls it made, one per line. That list is an allowlist for
//! rt's seccomp filter (`LINUX_NONE_SECCOMP`, see rt/src/seccomp.rs).
//! `--docker` also writes it as a Docker/OCI seccomp profile, and
//! `--systemd` as a `SystemCallFilter=` line for a unit file.
//!
//! A trace only sees what the run exercised, so run the workload the
//! program will see in production.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::process::Command;
use std::{env, fs, io};

use crate::err;

pub struct Options {
    pub docker: Option<PathBuf>,
    pub systemd: Option<PathBuf>,
    pub program: Vec<OsString>,
}

/// Parse `[--docker FILE] [--systemd FILE] [--] PROGRAM [ARGS...]`.
pub fn options(args: &[String]) -> Option<Options> {
    let mut o = Options { docker: None, systemd: None, program: Vec::new() };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--docker" => o.docker = Some(it.next()?.into()),
            "--systemd" => o.systemd = Some(it.next()?.into()),
            "--" => break,
            _ => {
                o.program.push(a.into());
                break;
            }
        }
    }
    o.program.extend(it.map(OsString::from));
    (!o.program.is_empty()).then_some(o)
}

pub fn syscalls(o: &Options) -> io::Result<()> {
    let names = trace(&o.program)?;
    for n in &names {
        println!("{n}");
    }
    if let Some(p) = &o.docker {
        fs::write(p, docker(&names))?;
    }
    if let Some(p) = &o.systemd {
        fs::write(p, systemd(&names))?;
    }
    Ok(())
}

/// The distinct syscalls `program` makes, by strace's (the kernel's) names.
pub fn trace(program: &[OsString]) -> io::Result<BTreeSet<String>> {
    let log = env::temp_dir().join(format!("xtask-syscalls-{}.strace", std::process::id()));
    let status = Command::new("strace")
        .args(["-f", "-qq", "-e", "trace=all", "-o"])
        .arg(&log)
        .arg("--")
        .args(program)
        // the list goes to stdout: the program's output goes to stderr
        .stdout(io::stderr().as_fd().try_clone_to_owned()?)
        .status()
        .map_err(|e| err(format!("could not run strace (is it installed?): {e}")))?;
    let text = fs::read_to_string(&log);
    let _ = fs::remove_file(&log);
    if !status.success() {
        eprintln!("xtask: note: the traced program exited with {status}");
    }
    Ok(parse(&text?))
}

/// Syscall names in `strace -f -o` output: `PID name(args) = ret`, with
/// `<... name resumed>` and `+++`/`---` lines for the rest.
pub fn parse(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim_start_matches(|c: char| c.is_ascii_digit()).trim_start();
            let end = l.find('(')?;
            let name = &l[..end];
            (!name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')).then(|| name.to_string())
        })
        .collect()
}

fn docker_arch() -> &'static str {
    match env::consts::ARCH {
        "aarch64" => "SCMP_ARCH_AARCH64",
        _ => "SCMP_ARCH_X86_64",
    }
}

/// What the container runtime itself calls between installing the filter
/// and `execve`ing the program: found by bisection with runc 1.4 (Docker
/// 29), plus `tgkill` and `getpid`, which runc's Go runtime uses to preempt
/// its threads. Without `rt_sigreturn`, runc dies on its first signal.
const RUNC: &[&str] = &[
    "capget", "capset", "chdir", "fchown", "fstat", "fstatfs", "getpid", "getppid", "newfstatat", "prctl", "rt_sigreturn",
    "setgid", "setgroups", "setuid", "tgkill",
];

/// A Docker/OCI seccomp profile allowing `names` and what runc needs (other
/// syscalls fail with EPERM). Each group is listed separately, so the
/// program's own surface stays readable.
pub fn docker(names: &BTreeSet<String>) -> String {
    let quote = |v: &mut dyn Iterator<Item = &str>| v.map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", ");
    let runtime: Vec<&str> = RUNC.iter().copied().filter(|n| !names.contains(*n)).collect();
    format!(
        "{{\n  \"defaultAction\": \"SCMP_ACT_ERRNO\",\n  \"defaultErrnoRet\": 1,\n  \"architectures\": [\"{}\"],\n  \"syscalls\": [\n    \
         {{ \"names\": [{}], \"action\": \"SCMP_ACT_ALLOW\", \"comment\": \"the program\" }},\n    \
         {{ \"names\": [{}], \"action\": \"SCMP_ACT_ALLOW\", \"comment\": \"runc, before execve\" }}\n  ]\n}}\n",
        docker_arch(),
        quote(&mut names.iter().map(String::as_str)),
        quote(&mut runtime.into_iter()),
    )
}

/// `SystemCallFilter=` for a systemd unit (other syscalls kill the process).
pub fn systemd(names: &BTreeSet<String>) -> String {
    format!("SystemCallFilter={}\n", names.iter().cloned().collect::<Vec<_>>().join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_strace_output() {
        let t = "123 execve(\"./x\", [\"./x\"], 0x7ff /* 3 vars */) = 0\n\
                 123 arch_prctl(ARCH_SET_FS, 0x4) = 0\n\
                 124 futex(0x1, FUTEX_WAIT, 0, NULL <unfinished ...>\n\
                 123 <... futex resumed>) = 0\n\
                 124 --- SIGCHLD {si_signo=SIGCHLD} ---\n\
                 123 write(1, \"hi(\\n\", 4) = 4\n\
                 124 +++ exited with 0 +++\n";
        let v: Vec<_> = parse(t).into_iter().collect();
        assert_eq!(v, ["arch_prctl", "execve", "futex", "write"]);
        assert!(systemd(&parse(t)).starts_with("SystemCallFilter=arch_prctl execve"));
        let d = docker(&parse(t));
        assert!(d.contains("\"names\": [\"arch_prctl\", \"execve\", \"futex\", \"write\"]"), "{d}");
        assert!(d.contains("\"names\": [\"capget\", "), "{d}");
    }
}
