//! Resolves each name given on the command line, as `TcpStream::connect`
//! would, and prints the addresses (sorted) or the error.

use std::net::ToSocketAddrs;

fn main() {
    for name in std::env::args().skip(1) {
        match (name.as_str(), 0).to_socket_addrs() {
            Ok(addrs) => {
                let mut ips: Vec<_> = addrs.map(|a| a.ip()).collect();
                ips.sort();
                println!("{name}: Ok({ips:?})");
            }
            Err(e) => println!("{name}: Err({:?}) {e}", e.kind()),
        }
    }
}
