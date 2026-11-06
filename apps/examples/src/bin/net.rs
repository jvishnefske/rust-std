use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::thread;

fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let server = thread::spawn(move || -> std::io::Result<()> {
        for _ in 0..2 {
            let (stream, _) = listener.accept()?;
            let mut reader = BufReader::new(stream.try_clone()?);
            let mut line = String::new();
            reader.read_line(&mut line)?;
            (&stream).write_all(line.to_uppercase().as_bytes())?;
        }
        Ok(())
    });
    for msg in ["hello tcp\n", "second\n"] {
        let mut s = TcpStream::connect(addr)?;
        s.set_nodelay(true)?;
        s.write_all(msg.as_bytes())?;
        let mut line = String::new();
        BufReader::new(&s).read_line(&mut line)?;
        println!("tcp echo: {:?}", line);
    }
    server.join().unwrap()?;

    let a = UdpSocket::bind("127.0.0.1:0")?;
    let b = UdpSocket::bind(("localhost", 0))?;
    a.send_to(b"ping", b.local_addr()?)?;
    let mut buf = [0u8; 16];
    let (n, from) = b.recv_from(&mut buf)?;
    println!("udp: {:?} from a: {}", core::str::from_utf8(&buf[..n]).unwrap(), from == a.local_addr()?);

    match TcpStream::connect("127.0.0.1:1") {
        Err(e) => println!("refused: {:?}", e.kind()),
        Ok(_) => println!("unexpected connect"),
    }
    match TcpStream::connect("example.com:80") {
        Err(e) => println!("dns: {:?}", e.kind()),
        Ok(_) => println!("unexpected dns"),
    }
    Ok(())
}
