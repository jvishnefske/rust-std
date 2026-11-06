#![no_std]
#![no_main]
extern crate rust_std as std;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::prelude::*;

std::main!(main);

fn main() -> io::Result<()> {
    let dir: PathBuf = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("rust_std_fs_test"));
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

    fs::remove_dir_all(&dir)?;
    println!("cleaned up: {}", !dir.exists());
    Ok(())
}
