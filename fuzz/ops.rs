// Fuzz operations, compiled twice: against the real std (in the harness
// driver) and against rust_std (in the `fuzzee` binary). Each target decodes
// arbitrary input bytes into a sequence of API calls and returns a transcript
// of the results; any difference between the two transcripts is a bug.
//
// This file is `include!`d, so it has no inner attributes and relies on the
// includer to bring std's prelude into scope.

use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, BufRead, Read, Seek, SeekFrom, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

pub const TARGETS: &[&str] = &["path", "osstr_fmt", "bufread", "writers", "cursor", "alloc", "fs"];

/// Run fuzz target `target` on `input`. `scratch` is a directory the `fs`
/// target may use; transcripts never contain it.
pub fn run(target: usize, input: &[u8], scratch: &Path) -> String {
    let mut u = Un { data: input, pos: 0 };
    let mut out = String::new();
    match target {
        0 => path(&mut u, &mut out),
        1 => osstr_fmt(&mut u, &mut out),
        2 => bufread(&mut u, &mut out),
        3 => writers(&mut u, &mut out),
        4 => cursor(&mut u, &mut out),
        5 => alloc(&mut u, &mut out),
        6 => fs(&mut u, &mut out, scratch),
        _ => out.push_str("unknown target"),
    }
    out
}

/// Decodes structured choices from raw bytes (zeros once exhausted).
struct Un<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Un<'_> {
    fn byte(&mut self) -> u8 {
        let b = self.data.get(self.pos).copied().unwrap_or(0);
        self.pos += 1;
        b
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { self.byte() as usize % n }
    }
    fn done(&self) -> bool {
        self.pos >= self.data.len()
    }
    fn bytes(&mut self, max: usize) -> Vec<u8> {
        let n = self.below(max + 1);
        (0..n).map(|_| self.byte()).collect()
    }
}

fn kind(e: &io::Error) -> String {
    match e.raw_os_error() {
        Some(code) => format!("{:?}/{code}", e.kind()),
        None => format!("{:?}", e.kind()),
    }
}

fn res<T: std::fmt::Debug>(r: &io::Result<T>) -> String {
    match r {
        Ok(v) => format!("Ok({v:?})"),
        Err(e) => format!("Err({})", kind(e)),
    }
}

// ---------------- path ----------------

const PATH_TOKENS: &[&[u8]] =
    &[b"/", b"//", b".", b"..", b"a", b"b", b"a.b", b".a", b"a.", b"b..c", b"x.tar.gz", b"\xc3\xa9", b"\xff", b"", b"./", b"/."];

fn gen_path(u: &mut Un) -> PathBuf {
    let mut v = Vec::new();
    for _ in 0..u.below(7) {
        v.extend_from_slice(PATH_TOKENS[u.below(PATH_TOKENS.len())]);
    }
    PathBuf::from(OsString::from_vec(v))
}

/// A file name or extension: path tokens without separators.
fn gen_name(u: &mut Un) -> OsString {
    let mut v = Vec::new();
    for _ in 0..u.below(3) {
        let t = PATH_TOKENS[u.below(PATH_TOKENS.len())];
        v.extend(t.iter().filter(|&&b| b != b'/'));
    }
    OsString::from_vec(v)
}

fn hash_of<T: Hash + ?Sized>(t: &T) -> u64 {
    let mut h = DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

fn path(u: &mut Un, out: &mut String) {
    let p = gen_path(u);
    let q = gen_path(u);
    let p: &Path = &p;
    let _ = writeln!(out, "p={p:?} display={} q={q:?}", p.display());
    let _ = writeln!(out, "components={:?}", p.components().collect::<Vec<_>>());
    let _ = writeln!(out, "rev={:?}", p.iter().rev().collect::<Vec<_>>());
    let (front, back) = (u.below(4), u.below(4));
    let mut c = p.components();
    for _ in 0..front {
        let _ = writeln!(out, "next={:?}", c.next());
    }
    for _ in 0..back {
        let _ = writeln!(out, "next_back={:?}", c.next_back());
    }
    let _ = writeln!(out, "as_path={:?}", c.as_path());
    let _ = writeln!(out, "parent={:?} file_name={:?}", p.parent(), p.file_name());
    let _ = writeln!(out, "stem={:?} prefix={:?} ext={:?}", p.file_stem(), p.file_prefix(), p.extension());
    let _ = writeln!(out, "absolute={} has_root={}", p.is_absolute(), p.has_root());
    let _ = writeln!(out, "ancestors={:?}", p.ancestors().collect::<Vec<_>>());
    let _ = writeln!(out, "join={:?}", p.join(&q));
    let _ = writeln!(out, "strip_prefix={:?}", p.strip_prefix(&q).ok());
    let _ = writeln!(out, "starts_with={} ends_with={}", p.starts_with(&q), p.ends_with(&q));
    let _ = writeln!(out, "eq={} cmp={:?}", p == q, p.cmp(&q));
    if p == q {
        let _ = writeln!(out, "hash_eq={}", hash_of(p) == hash_of(&q));
    }
    let name = gen_name(u);
    let _ = writeln!(out, "with_file_name={:?}", p.with_file_name(&name));
    let _ = writeln!(out, "with_extension={:?}", p.with_extension(&name));
    let _ = writeln!(out, "with_added_extension={:?}", p.with_added_extension(&name));
    let mut b = p.to_path_buf();
    for _ in 0..u.below(5) {
        match u.below(5) {
            0 => {
                let r = gen_path(u);
                b.push(&r);
                let _ = writeln!(out, "push {r:?} -> {b:?}");
            }
            1 => {
                let r = b.pop();
                let _ = writeln!(out, "pop {r} -> {b:?}");
            }
            2 => {
                let n = gen_name(u);
                b.set_file_name(&n);
                let _ = writeln!(out, "set_file_name {n:?} -> {b:?}");
            }
            3 => {
                let n = gen_name(u);
                let r = b.set_extension(&n);
                let _ = writeln!(out, "set_extension {n:?} {r} -> {b:?}");
            }
            _ => {
                let n = gen_name(u);
                let r = b.add_extension(&n);
                let _ = writeln!(out, "add_extension {n:?} {r} -> {b:?}");
            }
        }
    }
    if p.is_absolute() {
        let _ = writeln!(out, "path::absolute={:?}", std::path::absolute(p).ok());
    }
}

// ---------------- OsStr formatting ----------------

fn osstr_fmt(u: &mut Un, out: &mut String) {
    let mut v = Vec::new();
    for _ in 0..u.below(6) {
        match u.below(4) {
            0 => v.extend("aé'\"\\\n\t\u{1b}\u{301}🦀 ".as_bytes()[u.below(8)..].iter().take(3)),
            1 => v.push(u.byte()),
            2 => v.extend_from_slice("ab\u{301}c".as_bytes()),
            _ => v.push(u.byte() & 0x7f),
        }
    }
    let s = OsStr::from_bytes(&v);
    let w = u.below(12);
    let _ = writeln!(out, "debug={s:?}");
    let _ = writeln!(out, "display=[{}]", s.display());
    let _ = writeln!(out, "pad=[{:>w$}] [{:^w$}] [{:-<w$}]", s.display(), s.display(), s.display());
    let _ = writeln!(out, "path=[{:w$}] {:?}", Path::new(s).display(), Path::new(s));
    let _ = writeln!(out, "lossy={:?} to_str={:?} len={}", s.to_string_lossy(), s.to_str(), s.len());
}

// ---------------- BufReader / BufRead ----------------

/// A reader returning data in fuzzer-chosen chunk sizes, sometimes
/// failing with `Interrupted` (chunk size 0).
struct Chunked {
    data: Vec<u8>,
    pos: usize,
    sizes: Vec<usize>,
    i: usize,
    // never fail twice in a row, or retry loops (in std too) never end
    interrupted: bool,
}

impl Read for Chunked {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let size = self.sizes.get(self.i % self.sizes.len().max(1)).copied().unwrap_or(4);
        self.i += 1;
        if size == 0 && self.pos < self.data.len() && !self.interrupted {
            self.interrupted = true;
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        self.interrupted = false;
        let n = size.max(1).min(buf.len()).min(self.data.len() - self.pos);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

fn gen_text(u: &mut Un, max: usize) -> Vec<u8> {
    let n = u.below(max + 1);
    (0..n)
        .map(|_| match u.below(10) {
            0..=2 => b'\n',
            3 => b'x',
            4 => 0xff,
            5 => 0xc3,
            _ => b'a' + (u.byte() % 4),
        })
        .collect()
}

fn bufread(u: &mut Un, out: &mut String) {
    let data = gen_text(u, 40);
    let sizes: Vec<usize> = (0..u.below(4) + 1).map(|_| u.below(7)).collect();
    let cap = u.below(12) + 1;
    let mut r = io::BufReader::with_capacity(cap, Chunked { data, pos: 0, sizes, i: 0, interrupted: false });
    for _ in 0..12 {
        if u.done() {
            break;
        }
        // after an error, how much was consumed is unspecified: stop there
        let ok = match u.below(10) {
            0 => {
                let mut buf = vec![0; u.below(10)];
                let r = r.read(&mut buf).map(|n| buf[..n].to_vec());
                let _ = writeln!(out, "read {}", res(&r));
                r.is_ok()
            }
            1 => {
                let mut s = String::new();
                let r = r.read_line(&mut s);
                let _ = writeln!(out, "read_line {} {s:?}", res(&r));
                r.is_ok()
            }
            2 => {
                let d = [b'\n', b'a', b'x'][u.below(3)];
                let mut v = Vec::new();
                let r = r.read_until(d, &mut v);
                let _ = writeln!(out, "read_until {d} {} {v:?}", res(&r));
                r.is_ok()
            }
            3 => {
                let r2 = r.fill_buf().map(|b| b.to_vec());
                let _ = writeln!(out, "fill_buf {}", res(&r2));
                if let Ok(b) = &r2 {
                    let k = u.below(b.len() + 1);
                    r.consume(k);
                    let _ = writeln!(out, "consume {k}");
                }
                r2.is_ok()
            }
            4 => {
                let mut buf = vec![0; u.below(8)];
                let r = r.read_exact(&mut buf);
                let _ = writeln!(out, "read_exact {} {buf:?}", res(&r));
                r.is_ok()
            }
            5 => {
                let mut v = Vec::new();
                let r = r.read_to_end(&mut v);
                let _ = writeln!(out, "read_to_end {} {v:?}", res(&r));
                r.is_ok()
            }
            6 => {
                let mut s = String::new();
                let r = r.read_to_string(&mut s);
                let _ = writeln!(out, "read_to_string {} {s:?}", res(&r));
                r.is_ok()
            }
            7 => {
                let r = r.skip_until(b'\n');
                let _ = writeln!(out, "skip_until {}", res(&r));
                r.is_ok()
            }
            8 => {
                let n = u.below(6) as u64;
                let mut v = Vec::new();
                let r = r.by_ref().take(n).read_to_end(&mut v);
                let _ = writeln!(out, "take({n}) {} {v:?}", res(&r));
                r.is_ok()
            }
            _ => {
                let _ = writeln!(out, "buffer {:?}", r.buffer());
                true
            }
        };
        if !ok {
            break;
        }
    }
    // the bytes lines() would yield from a fresh reader over the same data
    let lines: Vec<_> = io::Cursor::new(r.into_inner().data).lines().map(|l| l.map_err(|e| kind(&e))).collect();
    let _ = writeln!(out, "lines {lines:?}");
}

// ---------------- BufWriter / LineWriter ----------------

/// Records what reaches it; accepts at most `limit` bytes per call and fails
/// with `Interrupted` on every `interrupt`-th call.
struct Recorder {
    got: std::rc::Rc<std::cell::RefCell<Vec<u8>>>,
    limit: usize,
    calls: usize,
    interrupt: usize,
}

impl Write for Recorder {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        // interrupt >= 2 when set, so a retry always gets through
        if self.interrupt != 0 && self.calls % self.interrupt == 0 {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        let n = buf.len().min(self.limit);
        self.got.borrow_mut().extend_from_slice(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn writers(u: &mut Un, out: &mut String) {
    let got = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let rec = Recorder { got: got.clone(), limit: u.below(8) + 1, calls: 0, interrupt: [0, 2, 3, 5][u.below(4)] };
    let line = u.below(2) == 0;
    let cap = u.below(10) + 1;
    let mut w: Box<dyn Write> =
        if line { Box::new(io::LineWriter::with_capacity(cap, rec)) } else { Box::new(io::BufWriter::with_capacity(cap, rec)) };
    let _ = writeln!(out, "{} cap={cap}", if line { "LineWriter" } else { "BufWriter" });
    let mut written = Vec::new();
    for _ in 0..10 {
        if u.done() {
            break;
        }
        let data = gen_text(u, 12);
        match u.below(4) {
            0 => {
                let r = w.write(&data);
                if let Ok(n) = r {
                    written.extend_from_slice(&data[..n]);
                }
                let _ = writeln!(out, "write {data:?} {}", res(&r));
            }
            1 | 2 => {
                let r = w.write_all(&data);
                if r.is_ok() {
                    written.extend_from_slice(&data);
                }
                let _ = writeln!(out, "write_all {data:?} {}", res(&r));
                if r.is_err() {
                    return;
                }
            }
            _ => {
                let r = w.flush();
                let _ = writeln!(out, "flush {}", res(&r));
                if r.is_err() {
                    return;
                }
            }
        }
        let _ = writeln!(out, "  inner has {:?}", got.borrow());
    }
    let r = w.flush();
    let _ = writeln!(out, "final flush {}", res(&r));
    drop(w);
    let _ = writeln!(out, "written {written:?}");
    let _ = writeln!(out, "inner {:?}", got.borrow());
}

// ---------------- Cursor ----------------

fn cursor(u: &mut Un, out: &mut String) {
    let mut c = io::Cursor::new(u.bytes(8));
    for _ in 0..12 {
        if u.done() {
            break;
        }
        match u.below(7) {
            0 => {
                let d = u.bytes(5);
                let r = c.write(&d);
                let _ = writeln!(out, "write {d:?} {}", res(&r));
            }
            1 => {
                let mut b = vec![0; u.below(6)];
                let r = c.read(&mut b).map(|n| b[..n].to_vec());
                let _ = writeln!(out, "read {}", res(&r));
            }
            2 => {
                let pos = match u.below(3) {
                    0 => SeekFrom::Start(u.below(20) as u64),
                    1 => SeekFrom::End(u.below(20) as i64 - 10),
                    _ => SeekFrom::Current(u.below(20) as i64 - 10),
                };
                let _ = writeln!(out, "seek {pos:?} {}", res(&c.seek(pos)));
            }
            3 => {
                let p = u.below(16) as u64;
                c.set_position(p);
                let _ = writeln!(out, "set_position {p}");
            }
            4 => {
                let r = c.fill_buf().map(|b| b.to_vec());
                let _ = writeln!(out, "fill_buf {}", res(&r));
            }
            5 => {
                let mut v = Vec::new();
                let _ = writeln!(out, "read_to_end {} {v:?}", res(&c.read_to_end(&mut v)));
            }
            _ => {
                let r = c.seek_relative(u.below(8) as i64 - 4);
                let _ = writeln!(out, "seek_relative {}", res(&r));
            }
        }
        let _ = writeln!(out, "  pos={} inner={:?}", c.position(), c.get_ref());
    }
}

// ---------------- allocator ----------------

fn alloc(u: &mut Un, out: &mut String) {
    use std::alloc::{Layout, alloc, dealloc, realloc};
    const SIZES: &[usize] = &[1, 7, 8, 15, 16, 17, 31, 64, 100, 255, 4095, 4096, 4097, 32768, 32769, 70000, 300_000];
    const ALIGNS: &[usize] = &[1, 2, 8, 16, 64, 4096, 8192];
    // (ptr, layout, fill byte)
    let mut live: Vec<(*mut u8, Layout, u8)> = Vec::new();
    let check = |p: *mut u8, l: Layout, fill: u8| -> bool {
        let s = unsafe { std::slice::from_raw_parts(p, l.size()) };
        s.iter().all(|&b| b == fill) && (p as usize) % l.align() == 0
    };
    for step in 0..24 {
        if u.done() {
            break;
        }
        match u.below(3) {
            0 => {
                let l = Layout::from_size_align(SIZES[u.below(SIZES.len())], ALIGNS[u.below(ALIGNS.len())]).unwrap();
                let p = unsafe { alloc(l) };
                if p.is_null() {
                    let _ = writeln!(out, "{step}: alloc failed");
                    continue;
                }
                let fill = step as u8 ^ 0x5a;
                unsafe { p.write_bytes(fill, l.size()) };
                live.push((p, l, fill));
            }
            1 if !live.is_empty() => {
                let i = u.below(live.len());
                let (p, l, fill) = live.swap_remove(i);
                if !check(p, l, fill) {
                    let _ = writeln!(out, "{step}: CORRUPT before free");
                }
                unsafe { dealloc(p, l) };
            }
            2 if !live.is_empty() => {
                let i = u.below(live.len());
                let (p, l, fill) = live[i];
                let new_size = SIZES[u.below(SIZES.len())];
                let q = unsafe { realloc(p, l, new_size) };
                if q.is_null() {
                    let _ = writeln!(out, "{step}: realloc failed");
                    continue;
                }
                let kept = l.size().min(new_size);
                let nl = Layout::from_size_align(new_size, l.align()).unwrap();
                let prefix_ok = unsafe { std::slice::from_raw_parts(q, kept) }.iter().all(|&b| b == fill);
                if !prefix_ok || (q as usize) % l.align() != 0 {
                    let _ = writeln!(out, "{step}: CORRUPT after realloc");
                }
                unsafe { q.write_bytes(fill, new_size) };
                live[i] = (q, nl, fill);
            }
            _ => {}
        }
        for &(p, l, fill) in &live {
            if !check(p, l, fill) {
                let _ = writeln!(out, "{step}: CORRUPT size={} align={}", l.size(), l.align());
            }
        }
    }
    for (p, l, _) in live {
        unsafe { dealloc(p, l) };
    }
    let _ = writeln!(out, "done");
}

// ---------------- fs ----------------

const NAMES: &[&str] = &["a", "b", "c", "a/b", "a/c", "b/a", "a/b/c", "a/../b"];

fn fs(u: &mut Un, out: &mut String, root: &Path) {
    use std::fs;
    let croot = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let rel = |p: &Path| p.strip_prefix(&croot).map(Path::to_path_buf).unwrap_or_else(|_| p.to_path_buf());
    for _ in 0..16 {
        if u.done() {
            break;
        }
        let n1 = NAMES[u.below(NAMES.len())];
        let n2 = NAMES[u.below(NAMES.len())];
        let (p1, p2) = (root.join(n1), root.join(n2));
        let line = match u.below(17) {
            0 => {
                let d = u.bytes(6);
                format!("write {n1} {d:?} {}", res(&fs::write(&p1, &d)))
            }
            1 => {
                let r = fs::OpenOptions::new().append(true).open(&p1).and_then(|mut f| f.write_all(b"+"));
                format!("append {n1} {}", res(&r))
            }
            2 => format!("read {n1} {}", res(&fs::read(&p1))),
            3 => format!("create_dir {n1} {}", res(&fs::create_dir(&p1))),
            4 => format!("create_dir_all {n1} {}", res(&fs::create_dir_all(&p1))),
            5 => format!("remove_file {n1} {}", res(&fs::remove_file(&p1))),
            6 => format!("remove_dir {n1} {}", res(&fs::remove_dir(&p1))),
            7 => format!("remove_dir_all {n1} {}", res(&fs::remove_dir_all(&p1))),
            8 => format!("rename {n1} {n2} {}", res(&fs::rename(&p1, &p2))),
            9 => {
                let r = fs::metadata(&p1).map(|m| (m.is_dir(), m.is_file(), m.is_symlink(), m.len()));
                format!("metadata {n1} {}", res(&r))
            }
            10 => {
                let r = fs::symlink_metadata(&p1).map(|m| (m.is_dir(), m.is_file(), m.is_symlink()));
                format!("symlink_metadata {n1} {}", res(&r))
            }
            11 => {
                let r = fs::read_dir(&p1).and_then(|d| {
                    let mut v = d.map(|e| e.map(|e| (e.file_name(), e.file_type().map(|t| t.is_dir()).ok()))).collect::<io::Result<Vec<_>>>()?;
                    v.sort();
                    Ok(v)
                });
                format!("read_dir {n1} {}", res(&r))
            }
            12 => format!("hard_link {n1} {n2} {}", res(&fs::hard_link(&p1, &p2))),
            13 => {
                // relative symlink target, so it is independent of `root`
                format!("symlink {n1} -> {n2} {}", res(&std::os::unix::fs::symlink(n2, &p1)))
            }
            14 => format!("read_link {n1} {}", res(&fs::read_link(&p1))),
            15 => format!("copy {n1} {n2} {}", res(&fs::copy(&p1, &p2))),
            _ => format!("canonicalize {n1} {}", res(&fs::canonicalize(&p1).map(|p| rel(&p)))),
        };
        let _ = writeln!(out, "{line}");
    }
}
