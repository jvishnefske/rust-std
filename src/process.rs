//! Processes: spawning children with `clone`+`execve`, exiting.

use alloc_crate::borrow::ToOwned;
use alloc_crate::ffi::CString;
use alloc_crate::vec::Vec;
use core::fmt;

use crate::ffi::{OsStr, OsString};
use crate::fs::File;
use crate::io::{self, Read, Write};
use crate::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use crate::path::Path;
use crate::sys::fd::{self as sfd, cvt, cvt_r};
use crate::sys::linux;

// ---- exiting ----

pub fn exit(code: i32) -> ! {
    crate::rt::exit(code)
}

pub fn abort() -> ! {
    crate::rt::abort()
}

pub fn id() -> u32 {
    linux::getpid()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitCode(u8);

impl ExitCode {
    pub const SUCCESS: ExitCode = ExitCode(0);
    pub const FAILURE: ExitCode = ExitCode(1);

    pub fn exit_process(self) -> ! {
        exit(self.0 as i32)
    }

    #[doc(hidden)]
    pub fn to_i32(self) -> i32 {
        self.0 as i32
    }
}

impl Default for ExitCode {
    fn default() -> Self {
        ExitCode::SUCCESS
    }
}

impl From<u8> for ExitCode {
    fn from(code: u8) -> Self {
        ExitCode(code)
    }
}

/// A value `main` may return.
pub trait Termination {
    fn report(self) -> ExitCode;
}

impl Termination for () {
    fn report(self) -> ExitCode {
        ExitCode::SUCCESS
    }
}

impl Termination for ExitCode {
    fn report(self) -> ExitCode {
        self
    }
}

impl Termination for core::convert::Infallible {
    fn report(self) -> ExitCode {
        match self {}
    }
}

impl<T: Termination, E: fmt::Debug> Termination for Result<T, E> {
    fn report(self) -> ExitCode {
        match self {
            Ok(val) => val.report(),
            Err(err) => {
                crate::io::_eprint(format_args!("Error: {err:?}\n"));
                ExitCode::FAILURE
            }
        }
    }
}

// ---- Stdio ----

enum StdioKind {
    Inherit,
    Null,
    MakePipe,
    Fd(OwnedFd),
}

pub struct Stdio(StdioKind);

impl Stdio {
    pub fn piped() -> Stdio {
        Stdio(StdioKind::MakePipe)
    }
    pub fn inherit() -> Stdio {
        Stdio(StdioKind::Inherit)
    }
    pub fn null() -> Stdio {
        Stdio(StdioKind::Null)
    }
    pub fn makes_pipe(&self) -> bool {
        matches!(self.0, StdioKind::MakePipe)
    }
}

impl fmt::Debug for Stdio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            StdioKind::Inherit => "Inherit",
            StdioKind::Null => "Null",
            StdioKind::MakePipe => "MakePipe",
            StdioKind::Fd(_) => "Fd",
        })
    }
}

impl From<OwnedFd> for Stdio {
    fn from(fd: OwnedFd) -> Stdio {
        Stdio(StdioKind::Fd(fd))
    }
}

impl From<File> for Stdio {
    fn from(f: File) -> Stdio {
        Stdio(StdioKind::Fd(f.into()))
    }
}

impl From<io::Stdout> for Stdio {
    fn from(_: io::Stdout) -> Stdio {
        let _ = io::stdout().flush();
        Stdio(StdioKind::Fd(sfd::dup_cloexec(1).map(|fd| unsafe { OwnedFd::from_raw_fd(fd) }).expect("failed to dup stdout")))
    }
}

impl From<io::Stderr> for Stdio {
    fn from(_: io::Stderr) -> Stdio {
        Stdio(StdioKind::Fd(sfd::dup_cloexec(2).map(|fd| unsafe { OwnedFd::from_raw_fd(fd) }).expect("failed to dup stderr")))
    }
}

macro_rules! child_pipe {
    ($name:ident) => {
        pub struct $name {
            fd: OwnedFd,
        }

        crate::os::fd::impl_owned_fd!($name, fd);

        impl From<$name> for Stdio {
            fn from(p: $name) -> Stdio {
                Stdio(StdioKind::Fd(p.fd))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($name)).finish_non_exhaustive()
            }
        }
    };
}

child_pipe!(ChildStdin);
child_pipe!(ChildStdout);
child_pipe!(ChildStderr);

impl Write for ChildStdin {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        sfd::write(self.fd.as_raw_fd(), buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Write for &ChildStdin {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        sfd::write(self.fd.as_raw_fd(), buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for ChildStdout {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        sfd::read(self.fd.as_raw_fd(), buf)
    }
}

impl Read for ChildStderr {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        sfd::read(self.fd.as_raw_fd(), buf)
    }
}

// ---- ExitStatus / Output ----

/// Raw `wait4` status.
#[derive(PartialEq, Eq, Clone, Copy, Default)]
pub struct ExitStatus(pub(crate) i32);

impl ExitStatus {
    pub fn success(&self) -> bool {
        self.0 == 0
    }

    pub fn code(&self) -> Option<i32> {
        if self.0 & 0x7f == 0 { Some((self.0 >> 8) & 0xff) } else { None }
    }

    pub fn exit_ok(&self) -> Result<(), ExitStatusError> {
        if self.success() { Ok(()) } else { Err(ExitStatusError(*self)) }
    }
}

impl fmt::Debug for ExitStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("unix_wait_status").field(&self.0).finish()
    }
}

impl fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.code() {
            Some(c) => write!(f, "exit status: {c}"),
            None => {
                let sig = self.0 & 0x7f;
                write!(f, "signal: {sig}")?;
                if self.0 & 0x80 != 0 {
                    f.write_str(" (core dumped)")?;
                }
                Ok(())
            }
        }
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub struct ExitStatusError(ExitStatus);

impl ExitStatusError {
    pub fn code(&self) -> Option<core::num::NonZeroI32> {
        self.0.code().and_then(core::num::NonZeroI32::new)
    }
    pub fn into_status(&self) -> ExitStatus {
        self.0
    }
}

impl fmt::Display for ExitStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "process exited unsuccessfully: {}", self.0)
    }
}

impl core::error::Error for ExitStatusError {}

#[derive(PartialEq, Eq, Clone)]
pub struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl fmt::Debug for Output {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let out = core::str::from_utf8(&self.stdout);
        let err = core::str::from_utf8(&self.stderr);
        let mut d = f.debug_struct("Output");
        d.field("status", &self.status);
        match out {
            Ok(s) => d.field("stdout", &s),
            Err(_) => d.field("stdout", &self.stdout),
        };
        match err {
            Ok(s) => d.field("stderr", &s),
            Err(_) => d.field("stderr", &self.stderr),
        };
        d.finish()
    }
}

// ---- Child ----

pub struct Child {
    pid: i32,
    status: Option<ExitStatus>,
    pub stdin: Option<ChildStdin>,
    pub stdout: Option<ChildStdout>,
    pub stderr: Option<ChildStderr>,
}

impl Child {
    pub fn id(&self) -> u32 {
        self.pid as u32
    }

    pub fn kill(&mut self) -> io::Result<()> {
        if self.status.is_some() {
            return Ok(());
        }
        cvt(linux::kill(self.pid, linux::SIGKILL)).map(drop)
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        drop(self.stdin.take());
        if let Some(s) = self.status {
            return Ok(s);
        }
        let mut status = 0i32;
        cvt_r(|| unsafe { linux::wait4(self.pid, &mut status, 0) })?;
        self.status = Some(ExitStatus(status));
        Ok(ExitStatus(status))
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        const WNOHANG: u32 = 1;
        if let Some(s) = self.status {
            return Ok(Some(s));
        }
        let mut status = 0i32;
        let pid = cvt_r(|| unsafe { linux::wait4(self.pid, &mut status, WNOHANG) })?;
        if pid == 0 {
            Ok(None)
        } else {
            self.status = Some(ExitStatus(status));
            Ok(Some(ExitStatus(status)))
        }
    }

    pub fn wait_with_output(mut self) -> io::Result<Output> {
        drop(self.stdin.take());
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        match (self.stdout.take(), self.stderr.take()) {
            (None, None) => {}
            (Some(mut out), None) => {
                out.read_to_end(&mut stdout)?;
            }
            (None, Some(mut err)) => {
                err.read_to_end(&mut stderr)?;
            }
            (Some(out), Some(err)) => read2(out.fd, &mut stdout, err.fd, &mut stderr)?,
        }
        let status = self.wait()?;
        Ok(Output { status, stdout, stderr })
    }
}

impl fmt::Debug for Child {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Child")
            .field("pid", &self.pid)
            .field("stdin", &self.stdin)
            .field("stdout", &self.stdout)
            .field("stderr", &self.stderr)
            .finish_non_exhaustive()
    }
}

/// Drain two pipes concurrently so neither child pipe can fill and block.
fn read2(a: OwnedFd, out_a: &mut Vec<u8>, b: OwnedFd, out_b: &mut Vec<u8>) -> io::Result<()> {
    let mut fds = [
        linux::PollFd { fd: a.as_raw_fd(), events: linux::POLLIN, revents: 0 },
        linux::PollFd { fd: b.as_raw_fd(), events: linux::POLLIN, revents: 0 },
    ];
    let mut open = [true, true];
    let mut buf = [0u8; 8192];
    while open[0] || open[1] {
        for (i, f) in fds.iter_mut().enumerate() {
            f.fd = if open[i] { [a.as_raw_fd(), b.as_raw_fd()][i] } else { -1 };
            f.revents = 0;
        }
        cvt_r(|| unsafe { linux::ppoll(fds.as_mut_ptr(), 2, core::ptr::null()) })?;
        for i in 0..2 {
            if open[i] && fds[i].revents != 0 {
                let n = sfd::read(fds[i].fd, &mut buf)?;
                if n == 0 {
                    open[i] = false;
                } else if i == 0 {
                    out_a.extend_from_slice(&buf[..n]);
                } else {
                    out_b.extend_from_slice(&buf[..n]);
                }
            }
        }
    }
    Ok(())
}

// ---- Command ----

pub struct Command {
    program: OsString,
    pub(crate) arg0: Option<OsString>,
    args: Vec<OsString>,
    env_clear: bool,
    env: Vec<(OsString, Option<OsString>)>,
    cwd: Option<OsString>,
    stdin: Option<Stdio>,
    stdout: Option<Stdio>,
    stderr: Option<Stdio>,
}

impl Command {
    pub fn new<S: AsRef<OsStr>>(program: S) -> Command {
        Command {
            program: program.as_ref().to_os_string(),
            arg0: None,
            args: Vec::new(),
            env_clear: false,
            env: Vec::new(),
            cwd: None,
            stdin: None,
            stdout: None,
            stderr: None,
        }
    }

    pub fn arg<S: AsRef<OsStr>>(&mut self, arg: S) -> &mut Command {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    pub fn args<I, S>(&mut self, args: I) -> &mut Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        for a in args {
            self.arg(a);
        }
        self
    }

    pub fn env<K: AsRef<OsStr>, V: AsRef<OsStr>>(&mut self, key: K, val: V) -> &mut Command {
        self.set_env(key.as_ref(), Some(val.as_ref()));
        self
    }

    pub fn envs<I, K, V>(&mut self, vars: I) -> &mut Command
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        for (k, v) in vars {
            self.env(k, v);
        }
        self
    }

    pub fn env_remove<K: AsRef<OsStr>>(&mut self, key: K) -> &mut Command {
        self.set_env(key.as_ref(), None);
        self
    }

    pub fn env_clear(&mut self) -> &mut Command {
        self.env_clear = true;
        self.env.clear();
        self
    }

    fn set_env(&mut self, key: &OsStr, val: Option<&OsStr>) {
        let val = val.map(OsStr::to_os_string);
        match self.env.iter_mut().find(|(k, _)| **k == *key) {
            Some(e) => e.1 = val,
            None => self.env.push((key.to_os_string(), val)),
        }
    }

    pub fn current_dir<P: AsRef<Path>>(&mut self, dir: P) -> &mut Command {
        self.cwd = Some(dir.as_ref().as_os_str().to_os_string());
        self
    }

    pub fn stdin<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Command {
        self.stdin = Some(cfg.into());
        self
    }

    pub fn stdout<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Command {
        self.stdout = Some(cfg.into());
        self
    }

    pub fn stderr<T: Into<Stdio>>(&mut self, cfg: T) -> &mut Command {
        self.stderr = Some(cfg.into());
        self
    }

    pub fn get_program(&self) -> &OsStr {
        &self.program
    }

    pub fn get_args(&self) -> impl Iterator<Item = &OsStr> {
        self.args.iter().map(|a| a.as_os_str())
    }

    pub fn get_envs(&self) -> impl Iterator<Item = (&OsStr, Option<&OsStr>)> {
        self.env.iter().map(|(k, v)| (k.as_os_str(), v.as_deref()))
    }

    pub fn get_current_dir(&self) -> Option<&Path> {
        self.cwd.as_deref().map(Path::new)
    }

    pub fn spawn(&mut self) -> io::Result<Child> {
        self.spawn_with(StdioKind::Inherit, StdioKind::Inherit)
    }

    pub fn output(&mut self) -> io::Result<Output> {
        self.spawn_with(StdioKind::Null, StdioKind::MakePipe)?.wait_with_output()
    }

    pub fn status(&mut self) -> io::Result<ExitStatus> {
        self.spawn_with(StdioKind::Inherit, StdioKind::Inherit)?.wait()
    }

    fn final_env(&self) -> Vec<(OsString, OsString)> {
        let mut env = if self.env_clear { Vec::new() } else { crate::env::env_snapshot() };
        for (k, v) in &self.env {
            env.retain(|(ek, _)| ek != k);
            if let Some(v) = v {
                env.push((k.clone(), v.clone()));
            }
        }
        env
    }

    /// Resolve the program through `PATH` (from the child's environment).
    fn resolve_program(&self, env: &[(OsString, OsString)]) -> OsString {
        if self.program.as_bytes().contains(&b'/') {
            return self.program.clone();
        }
        let path = env.iter().find(|(k, _)| k == "PATH").map(|(_, v)| v.clone());
        let path = path.unwrap_or_else(|| OsString::from("/usr/local/bin:/usr/bin:/bin"));
        for dir in crate::env::split_paths(&path) {
            let candidate = dir.join(&self.program);
            let ok = crate::sys::path::with_path(&candidate, |p| {
                cvt(unsafe { linux::faccessat(linux::AT_FDCWD, p, linux::X_OK) })
            });
            if ok.is_ok() && candidate.is_file() {
                return candidate.into_os_string();
            }
        }
        self.program.clone()
    }

    fn spawn_with(&mut self, default_in: StdioKind, default_out: StdioKind) -> io::Result<Child> {
        fn cstr(s: &OsStr) -> io::Result<CString> {
            CString::new(s.as_bytes().to_owned()).map_err(io::Error::from)
        }

        let env = self.final_env();
        let program = cstr(&self.resolve_program(&env))?;
        let mut argv_c = Vec::with_capacity(self.args.len() + 1);
        argv_c.push(cstr(self.arg0.as_deref().unwrap_or(&self.program))?);
        for a in &self.args {
            argv_c.push(cstr(a)?);
        }
        let mut envp_c = Vec::with_capacity(env.len());
        for (k, v) in &env {
            let mut kv = k.as_bytes().to_vec();
            kv.push(b'=');
            kv.extend_from_slice(v.as_bytes());
            envp_c.push(CString::new(kv)?);
        }
        let mut argv: Vec<*const u8> = argv_c.iter().map(|c| c.as_ptr().cast::<u8>()).collect();
        argv.push(core::ptr::null());
        let mut envp: Vec<*const u8> = envp_c.iter().map(|c| c.as_ptr().cast::<u8>()).collect();
        envp.push(core::ptr::null());
        let cwd = match &self.cwd {
            Some(c) => Some(cstr(c)?),
            None => None,
        };

        // set up the child's stdio: (fd the child gets, our end of a pipe)
        let take = |s: &mut Option<Stdio>, default: StdioKind| s.take().map(|s| s.0).unwrap_or(default);
        let stdin = setup(take(&mut self.stdin, default_in), true)?;
        let stdout = setup(take(&mut self.stdout, clone_kind(&default_out)), false)?;
        let stderr = setup(take(&mut self.stderr, default_out), false)?;

        // the child reports exec failure as an errno over a CLOEXEC pipe
        let [err_r, err_w] = cvt(linux::pipe2(linux::O_CLOEXEC))?;
        let (err_r, err_w) = unsafe { (OwnedFd::from_raw_fd(err_r), OwnedFd::from_raw_fd(err_w)) };

        let child_fds = [
            stdin.0.as_ref().map_or(-1, |f| f.as_raw_fd()),
            stdout.0.as_ref().map_or(-1, |f| f.as_raw_fd()),
            stderr.0.as_ref().map_or(-1, |f| f.as_raw_fd()),
        ];

        let pid = cvt(unsafe { linux::fork() })?;
        if pid == 0 {
            // child: only async-signal-safe raw syscalls from here on, no allocation
            unsafe { child_exec(&child_fds, cwd.as_ref(), &program, &argv, &envp, err_w.as_raw_fd()) }
        }

        drop(err_w);
        drop((stdin.0, stdout.0, stderr.0));
        let mut errno = [0u8; 4];
        let mut got = 0;
        loop {
            match sfd::read(err_r.as_raw_fd(), &mut errno[got..]) {
                Ok(0) => break,
                Ok(n) => {
                    got += n;
                    if got == 4 {
                        break;
                    }
                }
                Err(e) => return Err(e),
            }
        }
        if got == 4 {
            let mut status = 0;
            let _ = cvt_r(|| unsafe { linux::wait4(pid as i32, &mut status, 0) });
            return Err(io::Error::from_raw_os_error(i32::from_ne_bytes(errno)));
        }

        Ok(Child {
            pid: pid as i32,
            status: None,
            stdin: stdin.1.map(|fd| ChildStdin { fd }),
            stdout: stdout.1.map(|fd| ChildStdout { fd }),
            stderr: stderr.1.map(|fd| ChildStderr { fd }),
        })
    }
}

fn clone_kind(k: &StdioKind) -> StdioKind {
    match k {
        StdioKind::Inherit => StdioKind::Inherit,
        StdioKind::Null => StdioKind::Null,
        StdioKind::MakePipe => StdioKind::MakePipe,
        StdioKind::Fd(_) => unreachable!(),
    }
}

/// Returns (fd for the child, parent's end of a pipe).
fn setup(kind: StdioKind, readable_by_child: bool) -> io::Result<(Option<OwnedFd>, Option<OwnedFd>)> {
    match kind {
        StdioKind::Inherit => Ok((None, None)),
        StdioKind::Fd(fd) => Ok((Some(fd), None)),
        StdioKind::Null => {
            let f = crate::fs::OpenOptions::new().read(true).write(true).open("/dev/null")?;
            Ok((Some(f.into()), None))
        }
        StdioKind::MakePipe => {
            let [r, w] = cvt(linux::pipe2(linux::O_CLOEXEC))?;
            let (r, w) = unsafe { (OwnedFd::from_raw_fd(r), OwnedFd::from_raw_fd(w)) };
            if readable_by_child { Ok((Some(r), Some(w))) } else { Ok((Some(w), Some(r))) }
        }
    }
}

unsafe fn child_exec(
    fds: &[i32; 3],
    cwd: Option<&CString>,
    program: &CString,
    argv: &[*const u8],
    envp: &[*const u8],
    err_fd: i32,
) -> ! {
    let fail = |e: i32| -> ! {
        let b = e.to_ne_bytes();
        let _ = unsafe { linux::write(err_fd, b.as_ptr(), 4) };
        linux::exit_group(127)
    };
    for (target, &fd) in fds.iter().enumerate() {
        if fd < 0 {
            continue;
        }
        let target = target as i32;
        let r = if fd == target {
            // already in place: just clear close-on-exec
            linux::fcntl(fd, linux::F_SETFD, 0).map(drop)
        } else {
            linux::dup3(fd, target, 0).map(drop)
        };
        if let Err(e) = r {
            fail(e);
        }
    }
    if let Some(cwd) = cwd
        && let Err(e) = unsafe { linux::chdir(cwd.as_ptr().cast::<u8>()) } {
            fail(e);
        }
    unsafe {
        // restore what the runtime changed at startup
        let act = crate::sys::arch::KSigaction { handler: linux::SIG_DFL, flags: 0, restorer: 0, mask: 0 };
        let _ = linux::rt_sigaction(linux::SIGPIPE, &act, core::ptr::null_mut());
        let empty: u64 = 0;
        let _ = linux::rt_sigprocmask(linux::SIG_SETMASK, &empty, core::ptr::null_mut());
        let e = linux::execve(program.as_ptr().cast::<u8>(), argv.as_ptr(), envp.as_ptr()).unwrap_err();
        fail(e)
    }
}

impl fmt::Debug for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.program)?;
        for a in &self.args {
            write!(f, " {a:?}")?;
        }
        Ok(())
    }
}
