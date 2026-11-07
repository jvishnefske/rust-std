//! The runtime's vDSO parser (linux_none_rt::vdso) against the host's own
//! vDSO and a copy of qemu-user's aarch64 one (`data/`, linked at its load
//! address): it finds `clock_gettime` there (also through `DT_HASH` alone,
//! and without version tables), and survives corrupted and truncated copies.

use harness::fuzz::Rng;
use linux_none_rt::sys::Timespec;
use linux_none_rt::vdso::{self, AT_SYSINFO_EHDR, CLOCK_GETTIME};

const PT_DYNAMIC: u32 = 2;
const DT_GNU_HASH: u64 = 0x6fff_fef5;
const DT_VERSYM: u64 = 0x6fff_fff0;

/// The host's vDSO: its address and a copy of its image.
fn host_vdso() -> Option<(usize, Vec<u8>)> {
    let auxv = std::fs::read("/proc/self/auxv").unwrap();
    let base = auxv
        .as_chunks::<16>()
        .0
        .iter()
        .map(|e| (u64::from_ne_bytes(e[..8].try_into().unwrap()), u64::from_ne_bytes(e[8..].try_into().unwrap())))
        .find(|&(k, _)| k == AT_SYSINFO_EHDR as u64)?
        .1 as usize;
    // SAFETY: AT_SYSINFO_EHDR from our own auxv
    let img = unsafe { vdso::image(base) }.expect("host vDSO does not parse");
    Some((base, img.to_vec()))
}

fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}

/// Offsets of the dynamic entries in `img` whose tag is `tag`.
fn dyn_entries(img: &[u8], tag: u64) -> Vec<usize> {
    let (phoff, phnum) = (u64_at(img, 32) as usize, u16::from_le_bytes([img[56], img[57]]) as usize);
    let p = (0..phnum).map(|i| phoff + i * 56).find(|&p| u32::from_le_bytes(img[p..p + 4].try_into().unwrap()) == PT_DYNAMIC).unwrap();
    let (off, size) = (u64_at(img, p + 8) as usize, u64_at(img, p + 32) as usize);
    (off..off + size).step_by(16).filter(|&d| u64_at(img, d) == tag).collect()
}

/// Replace the entries tagged `tag` with an unknown tag the parser skips.
fn drop_dyn(img: &mut [u8], tag: u64) {
    for d in dyn_entries(img, tag) {
        img[d..d + 8].copy_from_slice(&0x6fff_f000u64.to_le_bytes());
    }
}

fn host_or_skip() -> Option<(usize, Vec<u8>)> {
    let v = host_vdso();
    if v.is_none() {
        eprintln!("skipping: the host has no vDSO");
    }
    v
}

/// The images to parse, with the symbol to look up in each.
fn images() -> Vec<(Vec<u8>, &'static [u8], &'static [u8])> {
    let qemu = include_bytes!("data/qemu-aarch64-vdso.so").to_vec();
    let mut v = vec![(qemu, &b"__kernel_clock_gettime"[..], &b"LINUX_2.6.39"[..])];
    if let Some((_, img)) = host_or_skip() {
        v.push((img, CLOCK_GETTIME.0, CLOCK_GETTIME.1));
    }
    v
}

#[test]
fn qemu_aarch64_image() {
    let img = include_bytes!("data/qemu-aarch64-vdso.so");
    assert_eq!(vdso::image_size(img), Some(img.len()));
    // __kernel_clock_gettime@@LINUX_2.6.39 at 0x...24c8, the image at 0x...2000
    assert_eq!(vdso::lookup(img, b"__kernel_clock_gettime", b"LINUX_2.6.39"), Some(0x4c8));
    assert_eq!(vdso::lookup(img, b"__kernel_clock_getres", b"LINUX_2.6.39"), Some(0x4d8));
    assert_eq!(vdso::lookup(img, b"__kernel_clock_gettime", b"LINUX_2.6"), None);
    // an OBJECT, not a function
    assert_eq!(vdso::lookup(img, b"LINUX_2.6.39", b"LINUX_2.6.39"), None);
}

#[test]
fn finds_clock_gettime() {
    let Some((base, img)) = host_or_skip() else { return };
    let (name, version) = CLOCK_GETTIME;
    let off = vdso::lookup(&img, name, version).expect("clock_gettime not found");
    assert_eq!(vdso::lookup(&img, name, b"LINUX_1.0"), None);
    assert_eq!(vdso::lookup(&img, b"no_such_function", version), None);
    assert_eq!(vdso::lookup(&img, &name[..name.len() - 1], version), None);

    // call it, and compare with std's clock
    let f: vdso::ClockGettime = unsafe { std::mem::transmute(base + off) };
    let mut ts = Timespec::default();
    assert_eq!(unsafe { f(linux_none_rt::sys::CLOCK_REALTIME as i32, &mut ts) }, 0);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap();
    assert!((now.as_secs() as i64 - ts.sec).abs() <= 1, "vDSO says {ts:?}, std {now:?}");
    let mut a = Timespec::default();
    let mut b = Timespec::default();
    unsafe {
        f(linux_none_rt::sys::CLOCK_MONOTONIC as i32, &mut a);
        f(linux_none_rt::sys::CLOCK_MONOTONIC as i32, &mut b);
    }
    assert!((b.sec, b.nsec) >= (a.sec, a.nsec));
}

#[test]
fn hash_and_version_table_variants() {
    for (img, name, version) in images() {
        variants(&img, name, version);
    }
}

fn variants(img: &[u8], name: &[u8], version: &[u8]) {
    let want = vdso::lookup(img, name, version);
    assert!(want.is_some());

    // DT_HASH only (when the image has one)
    let mut sysv = img.to_vec();
    drop_dyn(&mut sysv, DT_GNU_HASH);
    if dyn_entries(img, 4).is_empty() {
        assert_eq!(vdso::lookup(&sysv, name, version), None);
    } else {
        assert_eq!(vdso::lookup(&sysv, name, version), want);
    }

    // no version tables: any version matches
    let mut unversioned = img.to_vec();
    drop_dyn(&mut unversioned, DT_VERSYM);
    assert_eq!(vdso::lookup(&unversioned, name, b"LINUX_1.0"), want);
}

#[test]
fn malformed_images() {
    for (img, name, version) in images() {
        malformed(&img, name, version);
    }
}

fn malformed(img: &[u8], name: &[u8], version: &[u8]) {
    let check = |b: &[u8]| {
        let _ = vdso::image_size(b);
        let found = vdso::lookup(b, name, version);
        if let Some(off) = found {
            assert!(off < b.len());
        }
        found.is_some()
    };

    // every truncation
    let found = (0..img.len()).filter(|&len| check(&img[..len])).count();
    // (the symbol is found once the image reaches past its tables)
    assert!(found > 0 && check(img));

    let mut found = 0;
    let mut rng = Rng::new(0x7664_736f);
    let iters = std::env::var("VDSO_FUZZ_ITERS").ok().and_then(|s| s.parse().ok()).unwrap_or(20_000);
    for _ in 0..iters {
        let mut b = img.to_vec();
        // a few random bytes or whole little-endian words in the image
        for _ in 0..1 + rng.next_u64() % 8 {
            let at = (rng.next_u64() % b.len() as u64) as usize;
            let v = rng.next_u64().to_le_bytes();
            let n = [1, 2, 4, 8][(rng.next_u64() % 4) as usize].min(b.len() - at);
            b[at..at + n].copy_from_slice(&v[..n]);
        }
        let len = if rng.next_u64().is_multiple_of(4) { (rng.next_u64() % b.len() as u64) as usize } else { b.len() };
        found += check(&b[..len]) as usize;
    }
    // most corruptions miss the headers and tables: the parser still works
    eprintln!("found in {found} of {iters} corrupted images");
    assert!(found > iters / 4);

    // random bytes behind a valid ELF header prefix
    for _ in 0..iters / 4 {
        let mut b: Vec<u8> = (0..rng.next_u64() % 512).map(|_| rng.next_u64() as u8).collect();
        let n = b.len().min(64);
        b[..n].copy_from_slice(&img[..n]);
        check(&b);
    }
}

#[test]
fn syscall_fallback() {
    // on the host the runtime's startup (and so vdso::init) never ran
    assert!(vdso::clock_gettime().is_none());
    let ts = linux_none_rt::sys::clock_gettime(linux_none_rt::sys::CLOCK_REALTIME);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap();
    assert!((now.as_secs() as i64 - ts.sec).abs() <= 1, "syscall says {ts:?}, std {now:?}");
}
