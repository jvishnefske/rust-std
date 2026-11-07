//! The C library functions the runtime provides (rt/src/clib.rs), called as
//! a crate using the libc crate calls them. Prints `name value` lines.

use std::ffi::{CStr, c_char, c_int, c_long, c_void};
use std::sync::atomic::{AtomicBool, Ordering};

#[repr(C)]
struct Passwd {
    name: *mut c_char,
    passwd: *mut c_char,
    uid: u32,
    gid: u32,
    gecos: *mut c_char,
    dir: *mut c_char,
    shell: *mut c_char,
}

#[repr(C)]
struct Group {
    name: *mut c_char,
    passwd: *mut c_char,
    gid: u32,
    mem: *mut *mut c_char,
}

#[repr(C)]
struct SigAction {
    handler: usize,
    mask: [u64; 16],
    flags: c_int,
    restorer: usize,
}

#[repr(C)]
struct DlPhdrInfo {
    addr: usize,
    name: *const c_char,
    phdr: *const c_void,
    phnum: u16,
}

// packed on x86_64 only, as the kernel's
#[cfg_attr(target_arch = "x86_64", repr(C, packed))]
#[cfg_attr(not(target_arch = "x86_64"), repr(C))]
struct EpollEvent {
    events: u32,
    data: u64,
}

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

unsafe extern "C" {
    fn __errno_location() -> *mut c_int;
    fn isatty(fd: c_int) -> c_int;
    fn sysconf(name: c_int) -> c_long;
    fn gethostname(name: *mut c_char, len: usize) -> c_int;
    fn getpid() -> c_int;
    fn getuid() -> u32;
    fn getgid() -> u32;
    fn syscall(n: c_long, ...) -> c_long;
    fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, off: i64) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
    fn getpwuid_r(uid: u32, pwd: *mut Passwd, buf: *mut c_char, len: usize, result: *mut *mut Passwd) -> c_int;
    fn getpwnam(name: *const c_char) -> *mut Passwd;
    fn getgrgid_r(gid: u32, grp: *mut Group, buf: *mut c_char, len: usize, result: *mut *mut Group) -> c_int;
    fn sigemptyset(set: *mut [u64; 16]) -> c_int;
    fn sigaction(sig: c_int, act: *const SigAction, old: *mut SigAction) -> c_int;
    fn raise(sig: c_int) -> c_int;
    fn sem_init(sem: *mut [c_int; 8], pshared: c_int, value: u32) -> c_int;
    fn sem_post(sem: *mut [c_int; 8]) -> c_int;
    fn sem_wait(sem: *mut [c_int; 8]) -> c_int;
    fn fork() -> c_int;
    fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
    fn execvp(file: *const c_char, argv: *const *const c_char) -> c_int;
    fn _exit(code: c_int) -> !;
    fn pipe(fds: *mut c_int) -> c_int;
    fn dup2(old: c_int, new: c_int) -> c_int;
    fn write(fd: c_int, buf: *const c_void, n: usize) -> isize;
    fn read(fd: c_int, buf: *mut c_void, n: usize) -> isize;
    fn close(fd: c_int) -> c_int;
    fn poll(fds: *mut PollFd, n: u64, timeout: c_int) -> c_int;
    fn epoll_create1(flags: c_int) -> c_int;
    fn epoll_ctl(epfd: c_int, op: c_int, fd: c_int, event: *mut EpollEvent) -> c_int;
    fn epoll_wait(epfd: c_int, events: *mut EpollEvent, max: c_int, timeout: c_int) -> c_int;
    fn dl_iterate_phdr(cb: unsafe extern "C" fn(*mut DlPhdrInfo, usize, *mut c_void) -> c_int, data: *mut c_void) -> c_int;
    fn getenv(name: *const c_char) -> *mut c_char;
    fn putenv(string: *mut c_char) -> c_int;
    fn fstat(fd: c_int, st: *mut [u64; 18]) -> c_int;
    fn statvfs(path: *const c_char, out: *mut [u64; 14]) -> c_int;
    fn setmntent(path: *const c_char, mode: *const c_char) -> *mut c_void;
    fn getmntent(f: *mut c_void) -> *mut [*mut c_char; 4];
    fn endmntent(f: *mut c_void) -> c_int;
    fn eventfd(initval: u32, flags: c_int) -> c_int;
    fn tcgetattr(fd: c_int, termios: *mut [u8; 60]) -> c_int;
    fn socket(domain: c_int, ty: c_int, proto: c_int) -> c_int;
    fn bind(fd: c_int, addr: *const c_void, len: u32) -> c_int;
    fn listen(fd: c_int, backlog: c_int) -> c_int;
    fn getsockopt(fd: c_int, level: c_int, name: c_int, val: *mut c_void, len: *mut u32) -> c_int;
    fn exp(x: f64) -> f64;
    fn pow(x: f64, y: f64) -> f64;
    fn sin(x: f64) -> f64;
    fn atan2(y: f64, x: f64) -> f64;
    fn log2(x: f64) -> f64;
}

#[cfg(target_arch = "x86_64")]
const SYS_GETPID: c_long = 39;
#[cfg(target_arch = "aarch64")]
const SYS_GETPID: c_long = 172;

const SIGUSR1: c_int = 10;
const EPOLLIN: u32 = 1;
const EPOLL_CTL_ADD: c_int = 1;

static GOT_SIGNAL: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: c_int) {
    GOT_SIGNAL.store(true, Ordering::SeqCst);
}

unsafe extern "C" fn count_objects(info: *mut DlPhdrInfo, _size: usize, data: *mut c_void) -> c_int {
    unsafe { *(data as *mut usize) += 1 };
    println!("dl_iterate_phdr phnum>0 {}", unsafe { (*info).phnum } > 0);
    0
}

fn cstr(p: *const c_char) -> String {
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

fn main() {
    unsafe {
        // stdin is a pipe or /dev/null under the harness, never a terminal
        let tty = isatty(0);
        println!("isatty {tty} errno {}", *__errno_location());
        println!("sysconf pagesize {}", sysconf(30));
        println!("sysconf cpus>0 {}", sysconf(84) > 0);
        println!("sysconf bogus {} errno {}", sysconf(-1), *__errno_location());

        let mut host = [0 as c_char; 256];
        assert_eq!(gethostname(host.as_mut_ptr(), host.len()), 0);
        println!("gethostname {}", cstr(host.as_ptr()));

        println!("syscall getpid {}", syscall(SYS_GETPID) == getpid() as c_long);
        println!("getpid {}", getpid() as u32 == std::process::id());

        let p = mmap(std::ptr::null_mut(), 8192, 3, 0x22, -1, 0);
        *(p as *mut u64) = 42;
        let v = *(p as *mut u64);
        println!("mmap {v} munmap {}", munmap(p, 8192));

        let mut pwd = std::mem::zeroed::<Passwd>();
        let mut res = std::ptr::null_mut();
        let mut buf = [0 as c_char; 1024];
        match getpwuid_r(getuid(), &mut pwd, buf.as_mut_ptr(), buf.len(), &mut res) {
            0 if !res.is_null() => {
                println!("getpwuid_r {} {} {}", cstr(pwd.name), pwd.uid, cstr(pwd.dir));
                let again = getpwnam(pwd.name);
                println!("getpwnam uid {}", if again.is_null() { -1 } else { (*again).uid as i64 });
            }
            e => println!("getpwuid_r none {e}"),
        }
        let mut tiny = [0 as c_char; 4];
        println!("getpwuid_r small {}", getpwuid_r(getuid(), &mut pwd, tiny.as_mut_ptr(), tiny.len(), &mut res));
        let mut grp = std::mem::zeroed::<Group>();
        let mut gres = std::ptr::null_mut();
        match getgrgid_r(getgid(), &mut grp, buf.as_mut_ptr(), buf.len(), &mut gres) {
            0 if !gres.is_null() => println!("getgrgid_r {} {}", cstr(grp.name), grp.gid),
            e => println!("getgrgid_r none {e}"),
        }

        let mut act = SigAction { handler: on_signal as *const () as usize, mask: [0; 16], flags: 0, restorer: 0 };
        sigemptyset(&mut act.mask);
        assert_eq!(sigaction(SIGUSR1, &act, std::ptr::null_mut()), 0);
        raise(SIGUSR1);
        println!("signal handled {}", GOT_SIGNAL.load(Ordering::SeqCst));

        let mut sem = [0 as c_int; 8];
        sem_init(&mut sem, 0, 0);
        let semp = &raw mut sem as usize;
        let t = std::thread::spawn(move || {
            for _ in 0..3 {
                sem_post(semp as *mut [c_int; 8]);
            }
        });
        for _ in 0..3 {
            sem_wait(&mut sem);
        }
        t.join().unwrap();
        println!("sem 3 waits");

        let mut fds = [0 as c_int; 2];
        pipe(fds.as_mut_ptr());
        let pid = fork();
        if pid == 0 {
            // the child: stdout to the pipe, then exec sh
            dup2(fds[1], 1);
            let argv = [c"sh".as_ptr(), c"-c".as_ptr(), c"echo from-exec; exit 5".as_ptr(), std::ptr::null()];
            execvp(c"sh".as_ptr(), argv.as_ptr());
            _exit(127);
        }
        close(fds[1]);
        let mut status = 0;
        waitpid(pid, &mut status, 0);
        let mut out = [0u8; 64];
        let n = read(fds[0], out.as_mut_ptr().cast(), out.len());
        close(fds[0]);
        println!("fork exec status {} out {:?}", (status >> 8) & 0xff, String::from_utf8_lossy(&out[..n.max(0) as usize]));

        pipe(fds.as_mut_ptr());
        write(fds[1], b"x".as_ptr().cast(), 1);
        let mut pfd = PollFd { fd: fds[0], events: 1, revents: 0 };
        println!("poll {} revents {}", poll(&mut pfd, 1, 1000), pfd.revents);
        let ep = epoll_create1(0);
        let mut ev = EpollEvent { events: EPOLLIN, data: 7 };
        epoll_ctl(ep, EPOLL_CTL_ADD, fds[0], &mut ev);
        let mut got = EpollEvent { events: 0, data: 0 };
        let n = epoll_wait(ep, &mut got, 1, 1000);
        let data = got.data;
        println!("epoll {n} data {data}");
        close(ep);
        close(fds[0]);
        close(fds[1]);

        let mut objects = 0usize;
        dl_iterate_phdr(count_objects, &raw mut objects as *mut c_void);
        println!("dl_iterate_phdr objects {objects}");

        let v = getenv(c"CLIB_TEST".as_ptr());
        println!("getenv {}", if v.is_null() { "(null)".into() } else { cstr(v) });
        println!("getenv missing {}", getenv(c"CLIB_TEST_MISSING".as_ptr()).is_null());
        putenv(c"CLIB_TEST=put".as_ptr() as *mut c_char);
        println!("putenv {}", cstr(getenv(c"CLIB_TEST".as_ptr())));

        // st_size is the seventh u64 of the kernel's struct stat
        let mut st = [0u64; 18];
        pipe(fds.as_mut_ptr());
        write(fds[1], b"abc".as_ptr().cast(), 3);
        let tmp = std::env::temp_dir().join(format!("clib-{}", std::process::id()));
        let f = std::fs::File::create(&tmp).unwrap();
        std::fs::remove_file(&tmp).unwrap();
        std::io::Write::write_all(&mut &f, b"12345").unwrap();
        use std::os::fd::AsRawFd;
        println!("fstat {} size {}", fstat(f.as_raw_fd(), &mut st), st[6]);
        close(fds[0]);
        close(fds[1]);
        let mut vfs = [0u64; 14];
        println!("statvfs {} bsize>0 {}", statvfs(c"/".as_ptr(), &mut vfs), vfs[0] > 0);
        let mounts = setmntent(c"/proc/self/mounts".as_ptr(), c"r".as_ptr());
        let (mut n, mut root) = (0, false);
        loop {
            let m = getmntent(mounts);
            if m.is_null() {
                break;
            }
            n += 1;
            root |= cstr((*m)[1]) == "/";
        }
        endmntent(mounts);
        println!("getmntent some {} root {root}", n > 0);
        let efd = eventfd(0, 0);
        write(efd, 5u64.to_ne_bytes().as_ptr().cast(), 8);
        let mut v = [0u8; 8];
        read(efd, v.as_mut_ptr().cast(), 8);
        close(efd);
        println!("eventfd {}", u64::from_ne_bytes(v));
        let mut termios = [0u8; 60];
        println!("tcgetattr {} errno {}", tcgetattr(0, &mut termios), *__errno_location());
        // a TCP listener on 127.0.0.1, port chosen by the kernel
        let sock = socket(2, 1, 0);
        let addr: [u8; 16] = [2, 0, 0, 0, 127, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0];
        let (mut accepting, mut len) = (0 as c_int, 4u32);
        let ok = bind(sock, addr.as_ptr().cast(), 16) == 0 && listen(sock, 1) == 0;
        // SOL_SOCKET, SO_ACCEPTCONN
        getsockopt(sock, 1, 30, (&raw mut accepting).cast(), &mut len);
        close(sock);
        println!("socket listen {ok} accepting {accepting}");

        println!("math {:.6} {} {} {:.6} {}", exp(1.0), pow(2.0, 10.0), sin(0.0), atan2(1.0, 1.0), log2(8.0));
    }
}
