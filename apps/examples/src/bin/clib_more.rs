//! More of the runtime's C library functions (rt/src/clib.rs and
//! rt/src/clib/), the ones C code in crates calls: strings, ctype, printf,
//! pthread keys and rwlocks, directories, terminals, locale. Prints
//! `name value` lines.

use std::ffi::{CStr, CString, c_char, c_int, c_long, c_void};
use std::sync::atomic::{AtomicUsize, Ordering};

unsafe extern "C" {
    fn __errno_location() -> *mut c_int;
    fn strchr(s: *const c_char, c: c_int) -> *mut c_char;
    fn strrchr(s: *const c_char, c: c_int) -> *mut c_char;
    fn strstr(h: *const c_char, n: *const c_char) -> *mut c_char;
    fn strcmp(a: *const c_char, b: *const c_char) -> c_int;
    fn strncasecmp(a: *const c_char, b: *const c_char, n: usize) -> c_int;
    fn strncpy(d: *mut c_char, s: *const c_char, n: usize) -> *mut c_char;
    fn strnlen(s: *const c_char, n: usize) -> usize;
    fn memchr(s: *const c_void, c: c_int, n: usize) -> *mut c_void;
    fn strerror(e: c_int) -> *mut c_char;
    fn strsignal(s: c_int) -> *mut c_char;
    fn __ctype_b_loc() -> *const *const u16;
    fn __ctype_toupper_loc() -> *const *const i32;
    fn __isoc23_strtol(s: *const c_char, end: *mut *mut c_char, base: c_int) -> c_long;
    fn strtoul(s: *const c_char, end: *mut *mut c_char, base: c_int) -> u64;
    fn snprintf(buf: *mut c_char, n: usize, fmt: *const c_char, ...) -> c_int;
    fn fputs(s: *const c_char, f: *mut c_void) -> c_int;
    static stdout: *mut c_void;
    static environ: *const *const c_char;
    fn pthread_key_create(key: *mut u32, dtor: Option<unsafe extern "C" fn(*mut c_void)>) -> c_int;
    fn pthread_getspecific(key: u32) -> *mut c_void;
    fn pthread_setspecific(key: u32, v: *const c_void) -> c_int;
    fn pthread_key_delete(key: u32) -> c_int;
    fn pthread_rwlock_rdlock(l: *mut [u64; 7]) -> c_int;
    fn pthread_rwlock_trywrlock(l: *mut [u64; 7]) -> c_int;
    fn pthread_rwlock_unlock(l: *mut [u64; 7]) -> c_int;
    fn pthread_self() -> usize;
    fn pthread_getattr_np(t: usize, attr: *mut [u64; 7]) -> c_int;
    fn pthread_attr_getstack(attr: *const [u64; 7], addr: *mut *mut c_void, size: *mut usize) -> c_int;
    fn pthread_atfork(prepare: Option<extern "C" fn()>, parent: Option<extern "C" fn()>, child: Option<extern "C" fn()>) -> c_int;
    fn fork() -> c_int;
    fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
    fn _exit(code: c_int) -> !;
    fn mkdir(path: *const c_char, mode: u32) -> c_int;
    fn rmdir(path: *const c_char) -> c_int;
    fn unlink(path: *const c_char) -> c_int;
    fn rename(old: *const c_char, new: *const c_char) -> c_int;
    fn link(old: *const c_char, new: *const c_char) -> c_int;
    fn readlink(path: *const c_char, buf: *mut c_char, len: usize) -> isize;
    fn realpath(path: *const c_char, out: *mut c_char) -> *mut c_char;
    fn mkfifo(path: *const c_char, mode: u32) -> c_int;
    fn opendir(path: *const c_char) -> *mut c_void;
    fn readdir(d: *mut c_void) -> *mut Dirent;
    fn closedir(d: *mut c_void) -> c_int;
    fn free(p: *mut c_void);
    fn getsid(pid: c_int) -> c_int;
    fn getpgid(pid: c_int) -> c_int;
    fn ttyname_r(fd: c_int, buf: *mut c_char, len: usize) -> c_int;
    fn tcgetpgrp(fd: c_int) -> c_int;
    fn openpty(m: *mut c_int, s: *mut c_int, name: *mut c_char, t: *const c_void, w: *const c_void) -> c_int;
    fn cfmakeraw(t: *mut [u32; 15]);
    fn time(t: *mut i64) -> i64;
    fn select(n: c_int, r: *mut u64, w: *mut u64, e: *mut u64, tv: *mut [i64; 2]) -> c_int;
    fn pipe(fds: *mut c_int) -> c_int;
    fn write(fd: c_int, buf: *const c_void, n: usize) -> isize;
    fn close(fd: c_int) -> c_int;
    fn getgrouplist(user: *const c_char, group: u32, groups: *mut u32, n: *mut c_int) -> c_int;
    fn nl_langinfo(item: c_int) -> *mut c_char;
    fn iconv_open(to: *const c_char, from: *const c_char) -> isize;
    fn iconv(cd: isize, inb: *mut *mut c_char, inl: *mut usize, outb: *mut *mut c_char, outl: *mut usize) -> usize;
    fn __popcountdi2(x: i64) -> c_int;
}

#[repr(C)]
struct Dirent {
    ino: u64,
    off: i64,
    reclen: u16,
    kind: u8,
    name: [c_char; 256],
}

fn s(p: *const c_char) -> String {
    if p.is_null() { "NULL".into() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
}

fn errno() -> c_int {
    unsafe { *__errno_location() }
}

static DTOR_RAN: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn dtor(v: *mut c_void) {
    DTOR_RAN.store(v as usize, Ordering::SeqCst);
}

static FORK_LOG: AtomicUsize = AtomicUsize::new(0);
extern "C" fn prepare() {
    FORK_LOG.fetch_add(1, Ordering::SeqCst);
}
extern "C" fn parent() {
    FORK_LOG.fetch_add(10, Ordering::SeqCst);
}
extern "C" fn child() {
    unsafe { _exit(42) }
}

fn main() {
    unsafe {
        let hay = c"hello, world";
        println!(
            "strchr {} strrchr {} strstr {} strstr-miss {}",
            s(strchr(hay.as_ptr(), b'o' as c_int)),
            s(strrchr(hay.as_ptr(), b'o' as c_int)),
            s(strstr(hay.as_ptr(), c"wor".as_ptr())),
            s(strstr(hay.as_ptr(), c"xyz".as_ptr()))
        );
        let mut nine = [0x55 as c_char; 8];
        strncpy(nine.as_mut_ptr(), c"ab".as_ptr(), 5);
        println!(
            "strcmp {} {} strncasecmp {} strncpy {:?} strnlen {} memchr {}",
            strcmp(c"abc".as_ptr(), c"abd".as_ptr()).signum(),
            strcmp(c"abc".as_ptr(), c"abc".as_ptr()),
            strncasecmp(c"HeLLo".as_ptr(), c"hello!".as_ptr(), 5),
            &nine[..6],
            strnlen(c"abcdef".as_ptr(), 3),
            memchr(hay.as_ptr().cast(), b',' as c_int, 12) as usize - hay.as_ptr() as usize
        );
        println!("strerror {} strsignal {}", s(strerror(2)), s(strsignal(11)));

        let classes = *__ctype_b_loc();
        let upper = *__ctype_toupper_loc();
        // glibc's isalpha bit is 0x400, isspace's 0x2000, isdigit's 0x800
        println!(
            "ctype a {} space {} 7 {} -1 {} toupper {}",
            *classes.offset(b'a' as isize) & 0x400 != 0,
            *classes.offset(b' ' as isize) & 0x2000 != 0,
            *classes.offset(b'7' as isize) & 0x800 != 0,
            *classes.offset(-1),
            char::from_u32(*upper.offset(b'q' as isize) as u32).unwrap()
        );

        let mut end = std::ptr::null_mut();
        let v = __isoc23_strtol(c"  -0x1fz".as_ptr(), &mut end, 0);
        let big = __isoc23_strtol(c"99999999999999999999".as_ptr(), std::ptr::null_mut(), 10);
        let e = errno();
        println!("strtol {v} rest {} big {} erange {} strtoul {}", s(end), big == c_long::MAX, e == 34, strtoul(c"0777".as_ptr(), std::ptr::null_mut(), 0));

        let mut buf = [0 as c_char; 128];
        let n = snprintf(
            buf.as_mut_ptr(),
            buf.len(),
            c"[%d|%5s|%-4x|%05.1f|%c|%lu|%s|%%|%.3e|%g|%p]".as_ptr(),
            -42 as c_int,
            c"ab".as_ptr(),
            255 as c_int,
            3.14159f64,
            b'Z' as c_int,
            18446744073709551615u64,
            std::ptr::null::<c_char>(),
            12345.678f64,
            0.0001f64,
            0x1234usize as *const c_void,
        );
        println!("snprintf {n} {}", s(buf.as_ptr()));
        let mut small = [0 as c_char; 6];
        let n = snprintf(small.as_mut_ptr(), small.len(), c"%s-%03d".as_ptr(), c"long".as_ptr(), 7 as c_int);
        println!("snprintf truncated {n} {}", s(small.as_ptr()));
        use std::io::Write;
        std::io::stdout().flush().unwrap();
        fputs(c"fputs stdout\n".as_ptr(), stdout);

        let mut key = 0;
        pthread_key_create(&mut key, Some(dtor));
        pthread_setspecific(key, 7 as *const c_void);
        let main_v = pthread_getspecific(key) as usize;
        let thread_v = std::thread::spawn(move || {
            let before = pthread_getspecific(key) as usize;
            pthread_setspecific(key, 99 as *const c_void);
            before
        })
        .join()
        .unwrap();
        println!("pthread_key main {main_v} thread {thread_v} dtor {} delete {}", DTOR_RAN.load(Ordering::SeqCst), pthread_key_delete(key));

        let mut rw = [0u64; 7];
        pthread_rwlock_rdlock(&mut rw);
        let busy = pthread_rwlock_trywrlock(&mut rw);
        pthread_rwlock_unlock(&mut rw);
        let got = pthread_rwlock_trywrlock(&mut rw);
        pthread_rwlock_unlock(&mut rw);
        println!("pthread_rwlock {busy} {got} {}", pthread_rwlock_trywrlock(&mut rw));

        let local = 0u8;
        let here = &local as *const u8 as usize;
        let on_stack = |t| {
            let mut attr = [0u64; 7];
            let (mut addr, mut size) = (std::ptr::null_mut(), 0);
            let r = pthread_getattr_np(t, &mut attr);
            pthread_attr_getstack(&attr, &mut addr, &mut size);
            let here = &attr as *const _ as usize;
            r == 0 && (addr as usize) < here && here < addr as usize + size
        };
        let main_ok = on_stack(pthread_self());
        let self_main = pthread_self();
        let thread_ok = std::thread::spawn(move || (on_stack(pthread_self()), pthread_self() != self_main)).join().unwrap();
        println!("pthread_getattr_np main {main_ok} thread {thread_ok:?} {}", here != 0);

        pthread_atfork(Some(prepare), Some(parent), Some(child));
        let pid = fork();
        let mut status = 0;
        waitpid(pid, &mut status, 0);
        println!("pthread_atfork log {} child exit {}", FORK_LOG.load(Ordering::SeqCst), (status >> 8) & 0xff);

        let dir = std::env::temp_dir().join(format!("clib-more-{}", std::process::id()));
        let d = CString::new(dir.to_str().unwrap()).unwrap();
        let p = |name: &str| CString::new(format!("{}/{name}", dir.display())).unwrap();
        let made = mkdir(d.as_ptr(), 0o755);
        std::fs::write(dir.join("a"), b"x").unwrap();
        let renamed = rename(p("a").as_ptr(), p("b").as_ptr());
        let linked = link(p("b").as_ptr(), p("c").as_ptr());
        std::os::unix::fs::symlink("b", dir.join("l")).unwrap();
        let mut target = [0 as c_char; 16];
        let n = readlink(p("l").as_ptr(), target.as_mut_ptr(), target.len());
        let real = realpath(p("l").as_ptr(), std::ptr::null_mut());
        let real_ok = s(real) == format!("{}/b", std::fs::canonicalize(&dir).unwrap().display());
        free(real.cast());
        let fifo = mkfifo(p("f").as_ptr(), 0o600);
        let mut names = Vec::new();
        let dp = opendir(d.as_ptr());
        loop {
            let e = readdir(dp);
            if e.is_null() {
                break;
            }
            names.push(s((*e).name.as_ptr()));
        }
        closedir(dp);
        names.sort();
        for n in ["b", "c", "f", "l"] {
            unlink(p(n).as_ptr());
        }
        println!(
            "files mkdir {made} rename {renamed} link {linked} readlink {n} realpath {real_ok} mkfifo {fifo} readdir {names:?} rmdir {} missing {} errno {}",
            rmdir(d.as_ptr()),
            rmdir(d.as_ptr()),
            errno()
        );

        let mut name = [0 as c_char; 64];
        println!(
            "session getsid {} getpgid {} ttyname_r {} tcgetpgrp {} errno {}",
            getsid(0) > 0,
            getpgid(0) > 0,
            ttyname_r(-1, name.as_mut_ptr(), name.len()),
            tcgetpgrp(-1),
            errno()
        );
        let (mut m, mut sl) = (0, 0);
        match openpty(&mut m, &mut sl, name.as_mut_ptr(), std::ptr::null(), std::ptr::null()) {
            0 => {
                let mut tty = [0 as c_char; 64];
                let r = ttyname_r(sl, tty.as_mut_ptr(), tty.len());
                println!("openpty {} ttyname_r {r} same {}", s(name.as_ptr()).starts_with("/dev/pts/"), s(tty.as_ptr()) == s(name.as_ptr()));
                close(m);
                close(sl);
            }
            _ => println!("openpty none"),
        }
        let mut t = [u32::MAX; 15];
        cfmakeraw(&mut t);
        println!("cfmakeraw iflag {:#x} lflag {:#x} cflag {:#x}", t[0] & 0x5eb, t[3] & 0x804b, t[2] & 0x130);

        let fds = &mut [0; 2];
        pipe(fds.as_mut_ptr());
        let mut rset = 1u64 << fds[0];
        let mut tv = [0i64, 0];
        let empty = select(fds[0] + 1, &mut rset, std::ptr::null_mut(), std::ptr::null_mut(), &mut tv);
        write(fds[1], b"x".as_ptr().cast(), 1);
        rset = 1 << fds[0];
        let ready = select(fds[0] + 1, &mut rset, std::ptr::null_mut(), std::ptr::null_mut(), &mut tv);
        close(fds[0]);
        close(fds[1]);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        println!("select {empty} {ready} time {}", (time(std::ptr::null_mut()) - now).abs() <= 1);

        let mut groups = [0u32; 64];
        let mut n = groups.len() as c_int;
        let r = getgrouplist(c"root".as_ptr(), 12345, groups.as_mut_ptr(), &mut n);
        let mut one = 0 as c_int;
        let short = getgrouplist(c"root".as_ptr(), 12345, groups.as_mut_ptr(), &mut one);
        println!("getgrouplist first {} n {} short {short} {}", groups[0], r == n && n >= 1, one == n);

        println!(
            "langinfo {} {} {} {}",
            s(nl_langinfo(14)),
            s(nl_langinfo(0x20007 + 3)),
            s(nl_langinfo(0x2000e + 11)),
            s(nl_langinfo(0x20028))
        );
        let cd = iconv_open(c"UTF-8".as_ptr(), c"utf8".as_ptr());
        let mut input = *b"h\xc3\xa9!";
        let mut out = [0u8; 3];
        let (mut ip, mut il) = (input.as_mut_ptr().cast::<c_char>(), input.len());
        let (mut op, mut ol) = (out.as_mut_ptr().cast::<c_char>(), out.len());
        let r = iconv(cd, &mut ip, &mut il, &mut op, &mut ol);
        println!(
            "iconv open {} copy {} left {il} 2big {} other {}",
            cd != -1,
            r == usize::MAX,
            errno() == 7,
            iconv_open(c"UTF-16".as_ptr(), c"UTF-8".as_ptr()) == -1
        );

        let mut count = 0;
        let mut p = environ;
        while !(*p).is_null() {
            if s(*p).starts_with("CLIB_MORE=") {
                count += 1;
            }
            p = p.add(1);
        }
        println!("environ {count} popcount {}", __popcountdi2(-1));
    }
}
