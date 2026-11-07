//! `io::copy` between std's fd types, which std hands to the kernel
//! (`copy_file_range`, `sendfile`, `splice`) where it can, and the bytes that
//! arrive. The harness also checks under strace that the syscalls are used.
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;

/// Distinct bytes at every offset, so misplaced data shows.
fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 + i / 251) as u8).collect()
}

fn report(what: &str, copied: u64, got: &[u8], want: &[u8]) {
    println!("{what}: copied={copied} ok={}", got == want);
}

fn main() -> io::Result<()> {
    let dir: PathBuf = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("linux_none_kcopy"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir)?;
    let data = pattern(300_000);
    let src = dir.join("src");
    fs::write(&src, &data)?;

    // file -> file: copy_file_range
    let n = io::copy(&mut File::open(&src)?, &mut File::create(dir.join("dst"))?)?;
    report("file->file", n, &fs::read(dir.join("dst"))?, &data);

    // from an offset, through a part-consumed BufReader, limited by Take
    let mut f = File::open(&src)?;
    f.seek(SeekFrom::Start(1000))?;
    let mut r = BufReader::new(f);
    r.fill_buf()?;
    r.consume(10);
    let n = io::copy(&mut r.take(200_000), &mut File::create(dir.join("dst2"))?)?;
    report("file->file offset+take", n, &fs::read(dir.join("dst2"))?, &data[1010..201_010]);

    // into an O_APPEND file: copy_file_range refuses, the fallbacks do it
    fs::write(dir.join("app"), b"head")?;
    let n = io::copy(&mut File::open(&src)?, &mut OpenOptions::new().append(true).open(dir.join("app"))?)?;
    report("file->append", n, &fs::read(dir.join("app"))?, &[&b"head"[..], &data].concat());

    // file -> character device: sendfile
    let n = io::copy(&mut File::open(&src)?, &mut File::create("/dev/null")?)?;
    println!("file->devnull: copied={n}");

    // pipe -> pipe and pipe -> file: splice
    let (mut r1, mut w1) = io::pipe()?;
    let (mut r2, mut w2) = io::pipe()?;
    let feed = {
        let data = data.clone();
        thread::spawn(move || w1.write_all(&data))
    };
    let drain = thread::spawn(move || {
        let mut v = Vec::new();
        r2.read_to_end(&mut v).map(|_| v)
    });
    let n = io::copy(&mut r1, &mut w2)?;
    drop(w2);
    feed.join().unwrap()?;
    report("pipe->pipe", n, &drain.join().unwrap()?, &data);

    let (mut r, mut w) = io::pipe()?;
    let feed = {
        let data = data.clone();
        thread::spawn(move || w.write_all(&data))
    };
    let n = io::copy(&mut r, &mut File::create(dir.join("from_pipe"))?)?;
    feed.join().unwrap()?;
    report("pipe->file", n, &fs::read(dir.join("from_pipe"))?, &data);

    // ChildStdout -> file: splice
    let mut child = Command::new("cat").arg(&src).stdout(Stdio::piped()).spawn()?;
    let n = io::copy(child.stdout.as_mut().unwrap(), &mut File::create(dir.join("from_child"))?)?;
    let status = child.wait()?;
    report("child->file", n, &fs::read(dir.join("from_child"))?, &data);
    println!("child status: {status}");

    // file -> TcpStream
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let server = thread::spawn(move || -> io::Result<Vec<u8>> {
        let (mut s, _) = listener.accept()?;
        let mut v = Vec::new();
        s.read_to_end(&mut v)?;
        Ok(v)
    });
    let mut s = TcpStream::connect(addr)?;
    let n = io::copy(&mut File::open(&src)?, &mut s)?;
    drop(s);
    report("file->tcp", n, &server.join().unwrap()?, &data);

    // file -> UnixStream, then UnixStream -> file
    let (mut a, mut b) = UnixStream::pair()?;
    let mut out = File::create(dir.join("from_unix"))?;
    let reader = thread::spawn(move || io::copy(&mut b, &mut out));
    let n = io::copy(&mut File::open(&src)?, &mut a)?;
    drop(a);
    let m = reader.join().unwrap()?;
    assert_eq!(n, m);
    report("file->unix->file", n, &fs::read(dir.join("from_unix"))?, &data);

    fs::remove_dir_all(&dir)?;
    Ok(())
}
