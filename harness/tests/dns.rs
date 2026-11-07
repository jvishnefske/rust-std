//! The runtime's DNS stub resolver (`linux_none_rt::dns`, which std's
//! `getaddrinfo` uses on linux-none) run on the host, against a fake server
//! on 127.0.0.1 that gives canned answers over UDP and TCP. The resolver
//! uses only raw syscalls, so it behaves here as it does under std. (That
//! std reaches it through `/etc/resolv.conf` is checked by the `resolve`
//! example, against the real system resolver.)

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use linux_none_rt::dns::{self, Config, Error, Family};

const A: u16 = 1;
const CNAME: u16 = 5;
const AAAA: u16 = 28;

/// A query as the server saw it.
#[derive(Clone, Debug)]
struct Query {
    id: u16,
    name: String,
    qtype: u16,
    tcp: bool,
    /// the question section, to echo back
    question: Vec<u8>,
}

fn parse_query(msg: &[u8], tcp: bool) -> Query {
    let mut pos = 12;
    let mut labels = Vec::new();
    while msg[pos] != 0 {
        let len = msg[pos] as usize;
        labels.push(String::from_utf8_lossy(&msg[pos + 1..pos + 1 + len]).into_owned());
        pos += 1 + len;
    }
    let qtype = u16::from_be_bytes([msg[pos + 1], msg[pos + 2]]);
    Query {
        id: u16::from_be_bytes([msg[0], msg[1]]),
        name: labels.join("."),
        qtype,
        tcp,
        question: msg[12..pos + 5].to_vec(),
    }
}

fn wire_name(name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for label in name.split('.').filter(|l| !l.is_empty()) {
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    out
}

/// One answer record. `owner: None` is a compression pointer to the
/// question's name.
struct Rr {
    owner: Option<&'static str>,
    rtype: u16,
    data: Vec<u8>,
}

fn a(owner: Option<&'static str>, ip: [u8; 4]) -> Rr {
    Rr { owner, rtype: A, data: ip.to_vec() }
}

fn aaaa(owner: Option<&'static str>, ip: Ipv6Addr) -> Rr {
    Rr { owner, rtype: AAAA, data: ip.octets().to_vec() }
}

fn cname(owner: Option<&'static str>, to: &str) -> Rr {
    Rr { owner, rtype: CNAME, data: wire_name(to) }
}

/// A reply to `q` with `rcode`, the TC bit if `truncated`, and `answers`.
fn reply(q: &Query, rcode: u8, truncated: bool, answers: &[Rr]) -> Vec<u8> {
    let flags: u16 = 0x8180 | if truncated { 0x0200 } else { 0 } | rcode as u16;
    let mut m = Vec::new();
    m.extend_from_slice(&q.id.to_be_bytes());
    m.extend_from_slice(&flags.to_be_bytes());
    m.extend_from_slice(&[0, 1]);
    m.extend_from_slice(&(answers.len() as u16).to_be_bytes());
    m.extend_from_slice(&[0, 0, 0, 0]);
    m.extend_from_slice(&q.question);
    for rr in answers {
        match rr.owner {
            None => m.extend_from_slice(&[0xc0, 12]),
            Some(name) => m.extend_from_slice(&wire_name(name)),
        }
        m.extend_from_slice(&rr.rtype.to_be_bytes());
        m.extend_from_slice(&[0, 1, 0, 0, 0, 60]);
        m.extend_from_slice(&(rr.data.len() as u16).to_be_bytes());
        m.extend_from_slice(&rr.data);
    }
    m
}

/// What the server does with a query.
enum Action {
    Send(Vec<u8>),
    /// send from another socket (a spoofed source)
    SendFromElsewhere(Vec<u8>),
}

type Handler = dyn Fn(&Query) -> Vec<Action> + Send + Sync;

/// A DNS server on 127.0.0.1 (UDP and TCP on one port) that answers with
/// `handler` and records the queries it gets.
struct Server {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<Query>>>,
}

impl Server {
    fn start(handler: impl Fn(&Query) -> Vec<Action> + Send + Sync + 'static) -> Server {
        let handler: Arc<Handler> = Arc::new(handler);
        let seen = Arc::new(Mutex::new(Vec::new()));
        // a port free for both protocols
        let (udp, tcp) = loop {
            let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
            if let Ok(tcp) = TcpListener::bind(udp.local_addr().unwrap()) {
                break (udp, tcp);
            }
        };
        let addr = udp.local_addr().unwrap();
        let (h, s) = (handler.clone(), seen.clone());
        std::thread::spawn(move || {
            let other = UdpSocket::bind("127.0.0.1:0").unwrap();
            let mut buf = [0u8; 512];
            loop {
                let (n, from) = udp.recv_from(&mut buf).unwrap();
                let q = parse_query(&buf[..n], false);
                s.lock().unwrap().push(q.clone());
                for action in h(&q) {
                    match action {
                        Action::Send(m) => udp.send_to(&m, from).unwrap(),
                        Action::SendFromElsewhere(m) => other.send_to(&m, from).unwrap(),
                    };
                }
            }
        });
        let s = seen.clone();
        std::thread::spawn(move || {
            for stream in tcp.incoming() {
                let mut stream = stream.unwrap();
                let mut len = [0u8; 2];
                stream.read_exact(&mut len).unwrap();
                let mut msg = vec![0; u16::from_be_bytes(len) as usize];
                stream.read_exact(&mut msg).unwrap();
                let q = parse_query(&msg, true);
                s.lock().unwrap().push(q.clone());
                for action in handler(&q) {
                    if let Action::Send(m) = action {
                        stream.write_all(&(m.len() as u16).to_be_bytes()).unwrap();
                        stream.write_all(&m).unwrap();
                    }
                }
            }
        });
        Server { addr, seen }
    }

    fn seen(&self) -> Vec<(String, u16, bool)> {
        self.seen.lock().unwrap().iter().map(|q| (q.name.clone(), q.qtype, q.tcp)).collect()
    }
}

/// A configuration with these servers and a short timeout.
fn config(servers: &[SocketAddr], timeout_ms: u32, attempts: u32) -> Config {
    let mut conf = Config::new();
    for s in servers {
        assert!(conf.add_nameserver(*s));
    }
    conf.timeout_ms = timeout_ms;
    conf.attempts = attempts;
    conf
}

fn random() -> u16 {
    let mut b = [0u8; 2];
    std::fs::File::open("/dev/urandom").unwrap().read_exact(&mut b).unwrap();
    u16::from_ne_bytes(b)
}

fn lookup_with(conf: &Config, name: &str, family: Family, random: &mut dyn FnMut() -> u16) -> Result<Vec<IpAddr>, Error> {
    let mut buf = vec![0u8; 65535];
    let mut out = [IpAddr::V4(Ipv4Addr::UNSPECIFIED); 48];
    let n = dns::lookup(conf, name.as_bytes(), family, random, &mut buf, &mut out)?;
    Ok(out[..n].to_vec())
}

fn lookup(conf: &Config, name: &str) -> Result<Vec<IpAddr>, Error> {
    lookup_with(conf, name, Family::Any, &mut random)
}

fn v4(s: &str) -> IpAddr {
    s.parse().unwrap()
}

#[test]
fn a_and_aaaa() {
    let srv = Server::start(|q| {
        let answers = match q.qtype {
            A => vec![a(None, [192, 0, 2, 1]), a(None, [192, 0, 2, 2])],
            _ => vec![aaaa(None, "2001:db8::1".parse().unwrap())],
        };
        vec![Action::Send(reply(q, 0, false, &answers))]
    });
    let conf = config(&[srv.addr], 2000, 1);
    // IPv4 first, in the server's order
    assert_eq!(lookup(&conf, "www.example.test").unwrap(), [v4("192.0.2.1"), v4("192.0.2.2"), v4("2001:db8::1")]);
    assert_eq!(lookup_with(&conf, "www.example.test", Family::V6, &mut random).unwrap(), [v4("2001:db8::1")]);
    // names compare without regard to case
    assert_eq!(lookup_with(&conf, "WWW.Example.TEST", Family::V4, &mut random).unwrap().len(), 2);
    let seen = srv.seen();
    assert!(seen.contains(&("www.example.test".into(), A, false)), "{seen:?}");
    assert!(seen.contains(&("www.example.test".into(), AAAA, false)), "{seen:?}");
    assert_eq!(seen.len(), 4, "{seen:?}");
}

#[test]
fn query_ids_come_from_the_random_source() {
    let srv = Server::start(|q| vec![Action::Send(reply(q, 0, false, &[a(None, [192, 0, 2, 7])]))]);
    let conf = config(&[srv.addr], 2000, 1);
    let mut ids = [0x1234u16, 0xbeef].into_iter();
    let mut next = || ids.next().unwrap();
    assert_eq!(lookup_with(&conf, "x.test", Family::Any, &mut next).unwrap(), [v4("192.0.2.7")]);
    let mut got: Vec<u16> = srv.seen.lock().unwrap().iter().map(|q| q.id).collect();
    got.sort();
    assert_eq!(got, [0x1234, 0xbeef]);
}

#[test]
fn cname_chain() {
    // the records out of order, with others that are not on the chain
    let srv = Server::start(|q| {
        let answers = match q.qtype {
            A => vec![
                a(Some("final.example.test"), [198, 51, 100, 9]),
                a(Some("unrelated.example.test"), [203, 0, 113, 1]),
                cname(Some("alias.example.test"), "final.example.test"),
                cname(None, "alias.example.test"),
            ],
            _ => vec![cname(None, "alias.example.test")],
        };
        vec![Action::Send(reply(q, 0, false, &answers))]
    });
    let conf = config(&[srv.addr], 2000, 1);
    assert_eq!(lookup(&conf, "www.example.test").unwrap(), [v4("198.51.100.9")]);
}

#[test]
fn cname_loop_has_no_addresses() {
    let srv = Server::start(|q| {
        let answers =
            [cname(None, "b.test"), cname(Some("b.test"), "a.test"), a(Some("unrelated.test"), [203, 0, 113, 1])];
        vec![Action::Send(reply(q, 0, false, &answers))]
    });
    let conf = config(&[srv.addr], 2000, 1);
    assert_eq!(lookup(&conf, "a.test"), Err(Error::NoName));
}

#[test]
fn truncated_reply_is_asked_again_over_tcp() {
    // 40 addresses do not fit in 512 bytes
    let srv = Server::start(|q| {
        let all: Vec<Rr> = (0..40).map(|i| a(None, [10, 0, 0, i])).collect();
        let answers = if q.tcp { &all[..] } else { &all[..3] };
        vec![Action::Send(reply(q, 0, !q.tcp, answers))]
    });
    let conf = config(&[srv.addr], 2000, 1);
    let got = lookup_with(&conf, "big.test", Family::V4, &mut random).unwrap();
    assert_eq!(got, (0..40).map(|i| IpAddr::V4(Ipv4Addr::new(10, 0, 0, i))).collect::<Vec<_>>());
    assert_eq!(srv.seen(), [("big.test".into(), A, false), ("big.test".into(), A, true)]);
}

#[test]
fn forged_replies_are_ignored() {
    let srv = Server::start(|q| {
        let mut wrong_id = q.clone();
        wrong_id.id ^= 0x8000;
        let mut wrong_question = q.clone();
        wrong_question.question = [&wire_name("evil.test")[..], &q.question[q.question.len() - 4..]].concat();
        let mut wrong_type = q.clone();
        let n = wrong_type.question.len();
        wrong_type.question[n - 3] = if q.qtype == A { AAAA as u8 } else { A as u8 };
        let evil = |q: &Query| reply(q, 0, false, &[a(None, [6, 6, 6, 6])]);
        let mut query_not_reply = evil(q);
        query_not_reply[2] &= 0x7f;
        vec![
            Action::Send(evil(&wrong_id)),
            Action::Send(evil(&wrong_question)),
            Action::Send(evil(&wrong_type)),
            Action::Send(query_not_reply),
            Action::Send(vec![0; 5]),
            Action::SendFromElsewhere(evil(q)),
            Action::Send(reply(q, 0, false, &[a(None, [192, 0, 2, 53])])),
        ]
    });
    let conf = config(&[srv.addr], 2000, 1);
    assert_eq!(lookup_with(&conf, "good.test", Family::V4, &mut random).unwrap(), [v4("192.0.2.53")]);
}

#[test]
fn malformed_answer_fails() {
    let srv = Server::start(|q| {
        let mut m = reply(q, 0, false, &[a(None, [192, 0, 2, 1])]);
        // the record's rdata runs past the end
        m.truncate(m.len() - 2);
        vec![Action::Send(m)]
    });
    let conf = config(&[srv.addr], 2000, 1);
    assert_eq!(lookup_with(&conf, "bad.test", Family::V4, &mut random), Err(Error::Fail));
}

#[test]
fn silent_server_times_out_then_next_server_answers() {
    let silent = Server::start(|_| vec![]);
    let good = Server::start(|q| vec![Action::Send(reply(q, 0, false, &[a(None, [192, 0, 2, 80])]))]);
    let conf = config(&[silent.addr, good.addr], 300, 1);
    let t = Instant::now();
    assert_eq!(lookup_with(&conf, "x.test", Family::V4, &mut random).unwrap(), [v4("192.0.2.80")]);
    let took = t.elapsed();
    assert!(took >= Duration::from_millis(300) && took < Duration::from_secs(3), "{took:?}");
}

#[test]
fn timeout_and_attempts() {
    let silent = Server::start(|_| vec![]);
    let conf = config(&[silent.addr], 200, 3);
    let t = Instant::now();
    assert_eq!(lookup_with(&conf, "x.test", Family::V4, &mut random), Err(Error::Again));
    let took = t.elapsed();
    assert!(took >= Duration::from_millis(600) && took < Duration::from_secs(4), "{took:?}");
    assert_eq!(silent.seen().len(), 3);
}

#[test]
fn servfail_tries_next_server() {
    let failing = Server::start(|q| vec![Action::Send(reply(q, 2, false, &[]))]);
    let good = Server::start(|q| vec![Action::Send(reply(q, 0, false, &[a(None, [192, 0, 2, 2])]))]);
    let conf = config(&[failing.addr, good.addr], 2000, 1);
    assert_eq!(lookup_with(&conf, "x.test", Family::V4, &mut random).unwrap(), [v4("192.0.2.2")]);
    let conf = config(&[failing.addr], 2000, 1);
    assert_eq!(lookup_with(&conf, "x.test", Family::V4, &mut random), Err(Error::Again));
    // one family's SERVFAIL does not lose the other's addresses
    let half = Server::start(|q| {
        let r = if q.qtype == A { reply(q, 0, false, &[a(None, [192, 0, 2, 3])]) } else { reply(q, 2, false, &[]) };
        vec![Action::Send(r)]
    });
    let conf = config(&[half.addr], 2000, 1);
    assert_eq!(lookup(&conf, "x.test").unwrap(), [v4("192.0.2.3")]);
    // REFUSED
    let refusing = Server::start(|q| vec![Action::Send(reply(q, 5, false, &[]))]);
    let conf = config(&[refusing.addr], 2000, 1);
    assert_eq!(lookup_with(&conf, "x.test", Family::V4, &mut random), Err(Error::Fail));
}

#[test]
fn closed_port_fails_fast() {
    // nothing listens there: the ICMP error ends the wait
    let port = UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let conf = config(&[port], 5000, 1);
    let t = Instant::now();
    assert_eq!(lookup_with(&conf, "x.test", Family::V4, &mut random), Err(Error::Again));
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
}

#[test]
fn nxdomain_nodata_and_search_domains() {
    // only host.lab.test exists, with an A record
    let srv = Server::start(|q| {
        let r = match (q.name.as_str(), q.qtype) {
            ("host.lab.test", A) => reply(q, 0, false, &[a(None, [192, 0, 2, 4])]),
            ("host.lab.test", _) => reply(q, 0, false, &[]),
            _ => reply(q, 3, false, &[]),
        };
        vec![Action::Send(r)]
    });
    let mut conf = Config::parse(b"search corp.test lab.test\noptions ndots:1 timeout:2 attempts:1\n");
    conf.nameservers = [Some(srv.addr), None, None];

    // no dots: the search domains first, then the name as-is
    assert_eq!(lookup_with(&conf, "host", Family::V4, &mut random).unwrap(), [v4("192.0.2.4")]);
    assert_eq!(srv.seen(), [("host.corp.test".into(), A, false), ("host.lab.test".into(), A, false)]);
    srv.seen.lock().unwrap().clear();

    // NODATA for AAAA is not an error when A has addresses
    assert_eq!(lookup(&conf, "host").unwrap(), [v4("192.0.2.4")]);
    srv.seen.lock().unwrap().clear();

    // enough dots: as-is first
    assert_eq!(lookup_with(&conf, "a.b", Family::V4, &mut random), Err(Error::NoName));
    let names: Vec<String> = srv.seen().into_iter().map(|s| s.0).collect();
    assert_eq!(names, ["a.b", "a.b.corp.test", "a.b.lab.test"]);
    srv.seen.lock().unwrap().clear();

    // absolute: no search
    assert_eq!(lookup_with(&conf, "nothing.", Family::V4, &mut random), Err(Error::NoName));
    assert_eq!(srv.seen(), [("nothing".into(), A, false)]);

    // NODATA alone is no name
    assert_eq!(lookup_with(&conf, "host.lab.test", Family::V6, &mut random), Err(Error::NoName));

    // invalid names are not sent
    srv.seen.lock().unwrap().clear();
    for bad in ["", ".", "a..b", &"x".repeat(64), &["abc"; 70].join(".")] {
        assert_eq!(lookup(&conf, bad), Err(Error::NoName), "{bad:?}");
    }
    assert_eq!(srv.seen(), []);
}

#[test]
fn resolv_conf() {
    let conf = Config::parse(
        b"# comment\n\
          nameserver 192.0.2.1 ; trailing\n\
          nameserver fe80::1%3\n\
          nameserver not-an-address\n\
          domain old.test\n\
          search a.test b.test.\n\
          options rotate ndots:3 timeout:1 attempts:9 edns0\n\
          nameserver 192.0.2.3\n\
          nameserver 192.0.2.4\n",
    );
    assert_eq!(
        conf.nameservers,
        [
            Some("192.0.2.1:53".parse().unwrap()),
            Some("[fe80::1%3]:53".parse().unwrap()),
            Some("192.0.2.3:53".parse().unwrap()),
        ]
    );
    assert_eq!(conf.search().collect::<Vec<_>>(), [&b"a.test"[..], b"b.test"]);
    assert_eq!((conf.ndots, conf.timeout_ms, conf.attempts), (3, 1000, 5));

    // defaults
    let conf = Config::parse(b"");
    assert_eq!(conf.nameservers, [Some("127.0.0.1:53".parse().unwrap()), None, None]);
    assert_eq!((conf.ndots, conf.timeout_ms, conf.attempts), (1, 5000, 2));
    assert_eq!(conf.search().count(), 0);
}

#[test]
fn passwd_home_dir() {
    use linux_none_rt::passwd::home_dir;
    let file = b"root:x:0:0:root:/root:/bin/sh\n\
                 +nis\n\
                 broken:x:1000\n\
                 j:x:1000:1000:J,,,:/home/j:/bin/bash\n\
                 dup:x:1000:1000::/elsewhere:/bin/sh\n\
                 nohome:x:7:7:::\n\
                 big:x:4294967296:0::/overflow:/bin/sh\n\
                 last:x:9:9::/no/newline:/bin/sh";
    assert_eq!(home_dir(file, 0), Some(&b"/root"[..]));
    assert_eq!(home_dir(file, 1000), Some(&b"/home/j"[..]));
    assert_eq!(home_dir(file, 7), Some(&b""[..]));
    assert_eq!(home_dir(file, 0u32.wrapping_sub(1)), None);
    assert_eq!(home_dir(file, 9), Some(&b"/no/newline"[..]));
    assert_eq!(home_dir(b"", 0), None);
}
