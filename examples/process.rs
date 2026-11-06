#![no_std]
#![no_main]
extern crate rust_std as std;
use std::io::{Read, Write};
use std::prelude::*;
use std::process::{Command, ExitCode, Stdio};

std::main!(main);

fn main() -> ExitCode {
    let out = Command::new("echo").args(["hello", "from", "echo"]).output().unwrap();
    println!("echo: status={} stdout={:?}", out.status, String::from_utf8_lossy(&out.stdout));

    let out = Command::new("sh").arg("-c").arg("echo out; echo err >&2; exit 3").output().unwrap();
    println!("sh: code={:?} stdout={:?} stderr={:?}", out.status.code(), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));

    let out = Command::new("sh").arg("-c").arg("echo $FOO; pwd").env("FOO", "bar").current_dir("/").output().unwrap();
    println!("env+cwd: {:?}", String::from_utf8_lossy(&out.stdout));

    let mut child = Command::new("cat").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"piped through cat").unwrap();
    let mut s = String::new();
    child.stdout.take().unwrap().read_to_string(&mut s).unwrap();
    println!("cat: {s:?} {}", child.wait().unwrap());

    match Command::new("/nonexistent/program").status() {
        Err(e) => println!("spawn error: {:?}", e.kind()),
        Ok(s) => println!("unexpected: {s}"),
    }

    let status = Command::new("sh").arg("-c").arg("kill -9 $$").status().unwrap();
    println!("killed: success={} code={:?}", status.success(), status.code());

    ExitCode::from(7)
}
