use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

fn main() -> io::Result<()> {
    let dir: PathBuf = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("linux_none_fs_test"));
    // absolute, since it changes directory below
    let dir = std::path::absolute(dir)?;
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("a/b/c"))?;

    let file = dir.join("a/hello.txt");
    fs::write(&file, "line one\nline two\n")?;
    println!("read_to_string: {:?}", fs::read_to_string(&file)?);

    let mut f = OpenOptions::new().append(true).open(&file)?;
    writeln!(f, "line three")?;
    drop(f);

    let lines: Vec<String> = BufReader::new(File::open(&file)?).lines().collect::<io::Result<_>>()?;
    println!("lines: {lines:?}");

    let mut f = File::open(&file)?;
    f.seek(SeekFrom::Start(5))?;
    let mut word = [0u8; 3];
    f.read_exact(&mut word)?;
    println!("seek+read: {:?}", core::str::from_utf8(&word).unwrap());

    let meta = fs::metadata(&file)?;
    println!("len={} is_file={} is_dir={}", meta.len(), meta.is_file(), meta.is_dir());
    println!("modified is set: {}", meta.modified()?.duration_since(std::time::UNIX_EPOCH).is_ok());

    fs::copy(&file, dir.join("a/b/copy.txt"))?;
    fs::rename(dir.join("a/b/copy.txt"), dir.join("a/b/c/moved.txt"))?;
    std::os::unix::fs::symlink("hello.txt", dir.join("a/link"))?;
    println!("read_link: {:?}", fs::read_link(dir.join("a/link"))?);
    println!("via link: {}", fs::read_to_string(dir.join("a/link"))?.lines().count());

    let mut names: Vec<String> = fs::read_dir(dir.join("a"))?
        .map(|e| e.map(|e| e.file_name().into_string().unwrap()))
        .collect::<io::Result<_>>()?;
    names.sort();
    println!("read_dir: {names:?}");

    match File::open(dir.join("missing")) {
        Err(e) => println!("missing: kind={:?} msg={e}", e.kind()),
        Ok(_) => println!("missing: unexpectedly opened"),
    }

    let p = Path::new("/usr/lib/libfoo.so.1");
    println!("path: parent={:?} file_name={:?} stem={:?} ext={:?}", p.parent(), p.file_name(), p.file_stem(), p.extension());
    let comps: Vec<_> = Path::new("./a/../b//c/").components().collect();
    println!("components: {comps:?}");
    println!("join: {}", Path::new("/a").join("b").with_extension("txt").display());

    // canonicalize: symlink chains, "..", trailing slashes, dangling links
    // and loops (resolved without /proc)
    let a = dir.join("a");
    let base = fs::canonicalize(&dir)?;
    std::os::unix::fs::symlink("link", a.join("chain1"))?;
    std::os::unix::fs::symlink("chain1", a.join("chain2"))?;
    std::os::unix::fs::symlink("b/c/../c", a.join("dirlink"))?;
    std::os::unix::fs::symlink(base.join("a/b"), a.join("abs"))?;
    std::os::unix::fs::symlink("nowhere", a.join("dangling"))?;
    std::os::unix::fs::symlink("loop2", a.join("loop1"))?;
    std::os::unix::fs::symlink("loop1", a.join("loop2"))?;
    for p in [
        "a/chain2",
        "a/./b/../chain2",
        "a/dirlink/",
        "a/dirlink/..",
        "a/dirlink/../../a//b/c/",
        "a/abs/c/moved.txt",
        "a/hello.txt/",
        "a/hello.txt/..",
        "a/chain2/",
        "a/dangling",
        "a/loop1",
        "a/missing/..",
        "a/b/c/../../../a/b",
    ] {
        let r = fs::canonicalize(dir.join(p)).map(|c| c.strip_prefix(&base).map(Path::to_path_buf).unwrap_or(c));
        match r {
            Ok(c) => println!("canonicalize {p}: {c:?}"),
            Err(e) => println!("canonicalize {p}: {:?}", e.kind()),
        }
    }
    std::env::set_current_dir(dir.join("a/b"))?;
    println!("canonicalize relative: {:?}", fs::canonicalize("../dirlink/./moved.txt")?.strip_prefix(&base).ok());
    println!("canonicalize root: {:?} empty: {:?}", fs::canonicalize("/../..")?, fs::canonicalize("").map_err(|e| e.kind()));
    std::env::set_current_dir("/")?;

    fs::remove_dir_all(&dir)?;
    println!("cleaned up: {}", !dir.exists());
    Ok(())
}
