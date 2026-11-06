//! Turns the verbatim upstream std test files in `upstream/` into modules
//! that compile against rust_std, plus a registry of their tests.
//!
//! Per file this:
//! * drops inner attributes (`#![feature(..)]`) and inner doc comments,
//! * points `mod common;` / `use rand::` at the local shims in `src/main.rs`,
//! * removes skipped items (see `SUITES`) by prefixing them with `#[cfg(any())]`,
//! * makes every `#[test]` function `pub` and records it (with its `#[cfg]`s,
//!   `#[should_panic]` and `#[ignore]`) in the generated registry.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

struct Suite {
    /// module path in the binary, e.g. "sync::mutex"
    module: &'static str,
    /// file under upstream/
    file: &'static str,
    /// (item name, reason): test or helper items that cannot work here
    skip: &'static [(&'static str, &'static str)],
    /// textual (from, to) replacements, e.g. to drop an unstable import
    edits: &'static [(&'static str, &'static str)],
}

const UNWIND: &str = "needs unwinding (catch_unwind / panicking threads / poisoning)";

const NONPOISON: &str = "std::sync::nonpoison is unstable";
const MAPPED: &str = "mapped lock guards are unstable";
const VALUE_ACCESSORS: &str = "Mutex/RwLock set/replace/get_cloned are unstable";

const SUITES: &[Suite] = &[
    Suite {
        module: "path",
        file: "path.rs",
        skip: &[
            ("clone_to_uninit", "CloneToUninit is unstable"),
            ("normalize_lexically", "Path::normalize_lexically is unstable"),
            ("test_trim_trailing_sep", "Path::trim_trailing_sep is unstable"),
        ],
        edits: &[("use std::clone::CloneToUninit;\n", "")],
    },
    Suite { module: "env", file: "env.rs", skip: &[], edits: &[] },
    Suite { module: "env_modify", file: "env_modify.rs", skip: &[], edits: &[] },
    Suite {
        module: "time",
        file: "time.rs",
        skip: &[
            ("instant_checked_duration_since_nopanic", "Duration::SECOND is unstable"),
            ("instant_duration_since_saturates", "Duration::from_days is unstable"),
            ("instant_math", "Duration::SECOND is unstable"),
            ("instant_saturating_duration_since_nopanic", "Duration::SECOND is unstable"),
            ("since_epoch", "Duration::SECOND is unstable"),
            ("system_time_math", "Duration::SECOND is unstable"),
            ("system_time_duration_since_max_range_on_unix", "SystemTime::MIN/MAX are unstable"),
            ("system_time_max_min", "SystemTime::MIN/MAX are unstable"),
            ("system_time_saturating", "SystemTime::MIN/MAX are unstable"),
        ],
        edits: &[],
    },
    Suite { module: "thread", file: "thread.rs", skip: &[], edits: &[] },
    Suite {
        module: "thread_local",
        file: "thread_local.rs",
        skip: &[],
        // cfg(target_thread_local) is feature-gated; it holds on Linux
        edits: &[("#[cfg(target_thread_local)]\n", "")],
    },
    Suite { module: "istr", file: "istr.rs", skip: &[], edits: &[] },
    Suite { module: "seq_compare", file: "seq-compare.rs", skip: &[], edits: &[] },
    Suite { module: "builtin_clone", file: "builtin-clone.rs", skip: &[], edits: &[] },
    Suite { module: "create_dir_all_bare", file: "create_dir_all_bare.rs", skip: &[], edits: &[] },
    Suite { module: "num", file: "num.rs", skip: &[], edits: &[] },
    Suite { module: "sync::barrier", file: "sync/barrier.rs", skip: &[], edits: &[] },
    Suite {
        module: "sync::condvar",
        file: "sync/condvar.rs",
        skip: &[
            ("nonpoison_notify_all", NONPOISON),
            ("nonpoison_notify_one", NONPOISON),
            ("nonpoison_test_mutex_arc_condvar", NONPOISON),
            ("nonpoison_timeout_nanoseconds", NONPOISON),
            ("nonpoison_wait_timeout_wait", NONPOISON),
            ("nonpoison_wait_timeout_wake", NONPOISON),
            ("nonpoison_wait_timeout_while_instant_satisfy", NONPOISON),
            ("nonpoison_wait_timeout_while_wait", NONPOISON),
            ("nonpoison_wait_timeout_while_wake", NONPOISON),
            ("nonpoison_wait_while", NONPOISON),
            ("test_arc_condvar_poison", UNWIND),
        ],
        // std::sync::poison is an unstable alias for the std::sync locks
        edits: &[("std::sync::poison::", "std::sync::")],
    },
    Suite {
        module: "sync::lazy_lock",
        file: "sync/lazy_lock.rs",
        skip: &[
            ("const_lazy_default", "const Default is unstable"),
            ("lazy_lock_deref_mut_panic", UNWIND),
            ("lazy_lock_deref_panic", UNWIND),
            ("lazy_poisoning", UNWIND),
        ],
        edits: &[],
    },
    Suite {
        module: "sync::mpsc",
        file: "sync/mpsc.rs",
        skip: &[
            ("oneshot_single_thread_recv_chan_close", UNWIND),
            ("oneshot_multi_task_recv_then_close", UNWIND),
            ("oneshot_multi_thread_send_close_stress", UNWIND),
        ],
        edits: &[],
    },
    Suite {
        module: "sync::mpsc_sync",
        file: "sync/mpsc_sync.rs",
        skip: &[
            ("send_timeout", "SyncSender::send_timeout is unstable"),
            ("oneshot_single_thread_recv_chan_close", UNWIND),
            ("oneshot_multi_task_recv_then_close", UNWIND),
            ("oneshot_multi_thread_send_close_stress", UNWIND),
        ],
        edits: &[("use std::sync::mpmc::SendTimeoutError;\n", "")],
    },
    Suite {
        module: "sync::mutex",
        file: "sync/mutex.rs",
        skip: &[
            ("new_poisoned_mutex", UNWIND),
            ("panic_while_mapping_unlocked_poison", UNWIND),
            ("test_mutex_arc_poison_mapped", UNWIND),
            ("test_get_cloned", VALUE_ACCESSORS),
            ("test_get_cloned_poison", VALUE_ACCESSORS),
            ("test_replace", VALUE_ACCESSORS),
            ("test_replace_poison", VALUE_ACCESSORS),
            ("test_set", VALUE_ACCESSORS),
            ("test_set_drop", VALUE_ACCESSORS),
            ("test_set_poison", VALUE_ACCESSORS),
            ("test_mapping_mapped_guard", MAPPED),
            ("test_mutex_with_mut", NONPOISON),
            ("test_get_mut_poison", UNWIND),
            ("test_into_inner_poison", UNWIND),
            ("test_mutex_arc_poison", UNWIND),
        ],
        edits: &[("MappedMutexGuard, ", "")],
    },
    Suite { module: "sync::once", file: "sync/once.rs", skip: &[
            ("poison_bad", UNWIND),
            ("wait_force_on_poisoned", UNWIND),
            ("wait_for_force_to_finish", UNWIND),
            ("wait_on_poisoned", UNWIND),
        ], edits: &[] },
    Suite { module: "sync::once_lock", file: "sync/once_lock.rs", skip: &[("get_or_try_init", UNWIND)], edits: &[] },
    Suite {
        module: "sync::rwlock",
        file: "sync/rwlock.rs",
        skip: &[
            ("new_poisoned_rwlock", UNWIND),
            ("test_rw_arc_poison_wr", UNWIND),
            ("test_rw_arc_poison_ww", UNWIND),
            ("test_rw_arc_no_poison_rr", UNWIND),
            ("test_rw_arc_no_poison_rw", UNWIND),
            ("panic_while_mapping_read_unlocked_no_poison", UNWIND),
            ("panic_while_mapping_write_unlocked_poison", UNWIND),
            ("test_rw_arc_no_poison_mapped_r_r", UNWIND),
            ("test_rw_arc_no_poison_mapped_r_w", UNWIND),
            ("test_rw_arc_poison_mapped_w_r", UNWIND),
            ("test_rw_arc_poison_mapped_w_w", UNWIND),
            ("nonpoison_test_rwlock_try_write", NONPOISON),
            ("poison_test_rwlock_try_write", MAPPED),
            ("test_get_mut_poison", UNWIND),
            ("test_into_inner_poison", UNWIND),
            ("test_rwlock_with", NONPOISON),
            ("test_rwlock_with_mut", NONPOISON),
            ("test_downgrade_atomic", "RwLockWriteGuard::downgrade is unstable"),
            ("test_downgrade_basic", "RwLockWriteGuard::downgrade is unstable"),
            ("test_downgrade_observe", "RwLockWriteGuard::downgrade is unstable"),
            ("test_get_cloned", VALUE_ACCESSORS),
            ("test_get_cloned_poison", VALUE_ACCESSORS),
            ("test_replace", VALUE_ACCESSORS),
            ("test_replace_poison", VALUE_ACCESSORS),
            ("test_set", VALUE_ACCESSORS),
            ("test_set_poison", VALUE_ACCESSORS),
            ("test_mapped_read_guard_covariance", MAPPED),
            ("test_mapping_mapped_guard", MAPPED),
        ],
        edits: &[
            ("std::sync::poison::", "std::sync::"),
            ("MappedRwLockReadGuard, MappedRwLockWriteGuard, ", ""),
        ],
    },
];

fn main() {
    let out = std::env::var("OUT_DIR").unwrap();
    let mut registry = String::from("pub fn tests() -> Vec<Test> {\n    let mut v = Vec::new();\n");
    let mut skipped = String::from("pub const SKIPPED: &[(&str, &str)] = &[\n");

    for suite in SUITES {
        let src_path = Path::new("upstream").join(suite.file);
        println!("cargo:rerun-if-changed={}", src_path.display());
        let mut src = std::fs::read_to_string(&src_path).unwrap();
        for (from, to) in suite.edits {
            assert!(src.contains(from), "{}: edit {from:?} does not apply", suite.file);
            src = src.replace(from, to);
        }
        let (code, tests) = transform(&src, suite);
        let out_name = suite.module.replace("::", "__") + ".rs";
        std::fs::write(Path::new(&out).join(&out_name), code).unwrap();
        for t in tests {
            for c in &t.cfgs {
                writeln!(registry, "    {c}").unwrap();
            }
            writeln!(
                registry,
                "    v.push(Test {{ name: \"{m}::{n}\", f: || crate::report(crate::{m}::{n}()), should_panic: {p}, ignore: {i} }});",
                m = suite.module,
                n = t.name,
                p = match &t.should_panic {
                    None => "None".to_string(),
                    Some(msg) => format!("Some({msg:?})"),
                },
                i = t.ignore,
            )
            .unwrap();
        }
        for (name, why) in suite.skip {
            writeln!(skipped, "    (\"{}::{}\", {:?}),", suite.module, name, why).unwrap();
        }
    }
    registry.push_str("    v\n}\n");
    skipped.push_str("];\n");
    std::fs::write(Path::new(&out).join("registry.rs"), registry + &skipped).unwrap();

    // static, non-PIE, no crt files, no libc (as for rust_std's examples)
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let args: &[&str] = if arch == "aarch64" {
        &["-static", "--no-pie"]
    } else {
        &["-nostartfiles", "-nostdlib", "-static", "-no-pie"]
    };
    for a in args {
        println!("cargo:rustc-link-arg-bins={a}");
    }
    println!("cargo:rerun-if-changed=build.rs");
}

struct TestFn {
    name: String,
    cfgs: Vec<String>,
    should_panic: Option<String>,
    ignore: bool,
}

/// The identifier after `fn ` on a line, if any.
fn fn_name(line: &str) -> Option<&str> {
    let i = line.find("fn ")?;
    // reject e.g. "Fn " or a call like "foo_fn ("
    if i > 0 && line.as_bytes()[i - 1].is_ascii_alphanumeric() {
        return None;
    }
    let rest = &line[i + 3..];
    let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Index of the first line of the attribute block that ends just above `idx`.
fn attr_block_start(lines: &[String], mut idx: usize) -> usize {
    while idx > 0 {
        let prev = lines[idx - 1].trim();
        if prev.starts_with("#[") || prev.starts_with("///") {
            idx -= 1;
        } else if prev.ends_with(']') {
            // last line of a multi-line attribute: find its `#[` line
            let mut j = idx - 1;
            while j > 0 && !lines[j].trim().starts_with("#[") {
                j -= 1;
            }
            let joined: String = lines[j..idx].iter().map(|l| l.trim()).collect();
            if lines[j].trim().starts_with("#[") && balanced(&joined) {
                idx = j;
            } else {
                break;
            }
        } else {
            break;
        }
    }
    idx
}

/// For the item whose attributes end just above line `i`: whether it was
/// skipped, and its `#[cfg(..)]` attributes.
fn attrs_above(lines: &[String], i: usize) -> (bool, Vec<String>) {
    let block = &lines[attr_block_start(lines, i)..i];
    let removed = block.iter().any(|l| l.contains("#[cfg(any())] // skipped"));
    let cfgs = block
        .iter()
        .map(|l| l.trim())
        .filter(|l| l.starts_with("#[cfg("))
        .map(|l| l.split(" //").next().unwrap().to_string())
        .collect();
    (removed, cfgs)
}

fn transform(src: &str, suite: &Suite) -> (String, Vec<TestFn>) {
    let mut lines: Vec<String> = src
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("#![") || t.starts_with("//!"))
        })
        .map(|l| {
            let t = l.trim();
            if t == "mod common;" {
                "use crate::common;".to_string()
            } else {
                l.replace("use rand::", "use crate::rand::")
            }
        })
        .collect();

    // 1. skip items
    let skip: BTreeSet<&str> = suite.skip.iter().map(|s| s.0).collect();
    let mut found = BTreeSet::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim().to_string();
        let name = fn_name(&line)
            .map(str::to_string)
            .or_else(|| macro_test_name(&line).map(str::to_string))
            .or_else(|| item_name(&line).map(str::to_string));
        if let Some(name) = name.filter(|n| skip.contains(n.as_str())) {
            // macro tests: the item starts at the `..._test!(` line above
            let mut at = i;
            if macro_test_name(&line).is_some() {
                while at > 0 && !lines[at].contains("_test!(") {
                    at -= 1;
                }
            }
            let at = attr_block_start(&lines, at);
            lines.insert(at, "#[cfg(any())] // skipped by stdtests".to_string());
            found.insert(name);
            i += 2;
            continue;
        }
        i += 1;
    }
    for s in &skip {
        assert!(found.contains(*s), "{}: skip entry {s:?} matches no item", suite.file);
    }

    // 2. collect and rewrite #[test] functions
    let mut tests = Vec::new();
    let mut test_macros: Vec<(String, Vec<String>, Option<String>, bool)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        // tests generated by `nonpoison_and_poison_unwrap_test!(name: foo, ...)`
        if lines[i].trim().ends_with("_test!(") {
            let (removed, cfgs) = attrs_above(&lines, i);
            if let Some(name) = macro_test_name(lines[i + 1].trim()).filter(|_| !removed) {
                tests.push(TestFn { name: name.to_string(), cfgs, should_panic: None, ignore: false });
            }
            i += 1;
            continue;
        }
        if lines[i].trim() != "#[test]" {
            i += 1;
            continue;
        }
        let (removed, mut cfgs) = attrs_above(&lines, i);
        lines[i] = String::new();
        // walk to the fn line, classifying the attributes on the way
        let mut should_panic = None;
        let mut ignore = false;
        let mut j = i + 1;
        loop {
            let t = lines[j].trim().to_string();
            if t.is_empty() || t.starts_with("//") {
                j += 1;
                continue;
            }
            if t.starts_with("#[") {
                let start = j;
                let mut attr = t;
                while !balanced(&attr) {
                    j += 1;
                    attr.push_str(lines[j].trim());
                }
                let drop_attr = if attr.starts_with("#[cfg(") {
                    cfgs.push(attr.clone());
                    false
                } else if attr.starts_with("#[should_panic") {
                    should_panic = Some(expected_msg(&attr));
                    true
                } else if attr == "#[ignore]" || attr.starts_with("#[ignore =") {
                    ignore = true;
                    true
                } else {
                    false
                };
                if drop_attr {
                    for l in &mut lines[start..=j] {
                        l.clear();
                    }
                }
                j += 1;
                continue;
            }
            if t.contains("fn $") {
                // a test-generating macro: register its invocations below
                lines[j] = lines[j].replacen("fn ", "pub fn ", 1);
                let def = lines[..j].iter().rev().find_map(|l| l.trim().strip_prefix("macro_rules! ").map(|r| r.trim_end_matches(" {").to_string()));
                test_macros.push((def.expect("macro_rules! around fn $name"), cfgs, should_panic, ignore));
                break;
            }
            let name = fn_name(&t).unwrap_or_else(|| panic!("{}: no fn after #[test]: {t}", suite.file)).to_string();
            if !removed {
                if !t.starts_with("pub ") {
                    lines[j] = lines[j].replacen("fn ", "pub fn ", 1);
                }
                tests.push(TestFn { name, cfgs, should_panic, ignore });
            }
            break;
        }
        i = j + 1;
    }

    // tests generated by local macros: `test_foo! { name, ... }`
    for (mac, cfgs, should_panic, ignore) in &test_macros {
        for (k, l) in lines.iter().enumerate() {
            let t = l.trim();
            let Some(rest) = t.strip_prefix(mac.as_str()).and_then(|r| r.strip_prefix('!')) else { continue };
            let rest = rest.trim_start_matches([' ', '{', '(']);
            let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(rest.len());
            let skipped = k > 0 && lines[k - 1].contains("#[cfg(any())] // skipped");
            if end > 0 && !skipped {
                tests.push(TestFn { name: rest[..end].to_string(), cfgs: cfgs.clone(), should_panic: should_panic.clone(), ignore: *ignore });
            }
        }
    }

    let mut code = String::new();
    code.push_str(&format!("// generated by stdtests/build.rs from upstream/{}\n", suite.file));
    for l in &lines {
        code.push_str(l);
        code.push('\n');
    }
    (code, tests)
}

fn balanced(s: &str) -> bool {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut prev = ' ';
    for c in s.chars() {
        match c {
            '"' if prev != '\\' => in_str = !in_str,
            '(' | '[' if !in_str => depth += 1,
            ')' | ']' if !in_str => depth -= 1,
            _ => {}
        }
        prev = c;
    }
    depth == 0
}

/// The `expected` message of a `#[should_panic]` attribute ("" if none).
fn expected_msg(attr: &str) -> String {
    match attr.find('"') {
        Some(a) => {
            let rest = &attr[a + 1..];
            let b = rest.rfind('"').unwrap();
            rest[..b].replace("\\\"", "\"")
        }
        None => String::new(),
    }
}

/// `name: foo,` inside a `nonpoison_and_poison_unwrap_test!` invocation.
fn macro_test_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("name: ")?;
    Some(rest.trim_end_matches(','))
}

/// `struct Foo`, `impl Foo`, `macro_rules! foo`, ... for skipping helpers.
fn item_name(line: &str) -> Option<&str> {
    for kw in ["struct ", "enum ", "trait ", "macro_rules! ", "static ", "const ", "type "] {
        if let Some(rest) = line.strip_prefix(kw).or_else(|| line.strip_prefix("pub ").and_then(|l| l.strip_prefix(kw))) {
            let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(rest.len());
            return Some(&rest[..end]);
        }
    }
    None
}
