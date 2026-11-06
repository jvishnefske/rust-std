//! TCP and UDP networking over raw socket syscalls.
//!
//! Name resolution is limited to IP literals and `localhost`: there is no
//! DNS resolver without libc.

use alloc_crate::vec::Vec;
use core::fmt;
use core::time::Duration;

pub use core::net::{
    AddrParseError, IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6,
};

use crate::io::{self, Read, Write};
use crate::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use crate::sys::fd::{self as sfd, cvt, cvt_r};
use crate::sys::linux;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Shutdown {
    Read,
    Write,
    Both,
}

// ---- sockaddr encoding ----

#[repr(C)]
#[derive(Copy, Clone)]
struct SockaddrStorage {
    bytes: [u8; 128],
}

fn encode(addr: &SocketAddr) -> (SockaddrStorage, u32) {
    let mut s = SockaddrStorage { bytes: [0; 128] };
    match addr {
        SocketAddr::V4(a) => {
            s.bytes[0..2].copy_from_slice(&linux::AF_INET.to_ne_bytes());
            s.bytes[2..4].copy_from_slice(&a.port().to_be_bytes());
            s.bytes[4..8].copy_from_slice(&a.ip().octets());
            (s, 16)
        }
        SocketAddr::V6(a) => {
            s.bytes[0..2].copy_from_slice(&linux::AF_INET6.to_ne_bytes());
            s.bytes[2..4].copy_from_slice(&a.port().to_be_bytes());
            s.bytes[4..8].copy_from_slice(&a.flowinfo().to_be_bytes());
            s.bytes[8..24].copy_from_slice(&a.ip().octets());
            s.bytes[24..28].copy_from_slice(&a.scope_id().to_ne_bytes());
            (s, 28)
        }
    }
}

fn decode(s: &SockaddrStorage) -> io::Result<SocketAddr> {
    let b = &s.bytes;
    let family = u16::from_ne_bytes([b[0], b[1]]);
    let port = u16::from_be_bytes([b[2], b[3]]);
    match family {
        linux::AF_INET => Ok(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(b[4], b[5], b[6], b[7]), port))),
        linux::AF_INET6 => {
            let flow = u32::from_be_bytes(b[4..8].try_into().unwrap());
            let ip: [u8; 16] = b[8..24].try_into().unwrap();
            let scope = u32::from_ne_bytes(b[24..28].try_into().unwrap());
            Ok(SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::from(ip), port, flow, scope)))
        }
        _ => Err(io::const_error!(io::ErrorKind::InvalidInput, "invalid socket address family")),
    }
}

// ---- generic socket ----

struct Socket {
    fd: OwnedFd,
}

impl Socket {
    fn new(addr: &SocketAddr, ty: u32) -> io::Result<Socket> {
        let family = if addr.is_ipv4() { linux::AF_INET } else { linux::AF_INET6 };
        let fd = cvt(linux::socket(family, ty | linux::SOCK_CLOEXEC, 0))?;
        Ok(Socket { fd: unsafe { OwnedFd::from_raw_fd(fd) } })
    }

    fn raw(&self) -> i32 {
        self.fd.as_raw_fd()
    }

    fn connect(&self, addr: &SocketAddr) -> io::Result<()> {
        let (s, len) = encode(addr);
        cvt_r(|| unsafe { linux::connect(self.raw(), s.bytes.as_ptr(), len) }).map(drop)
    }

    fn bind(&self, addr: &SocketAddr) -> io::Result<()> {
        let (s, len) = encode(addr);
        cvt(unsafe { linux::bind(self.raw(), s.bytes.as_ptr(), len) }).map(drop)
    }

    fn setsockopt<T: Copy>(&self, level: u32, name: u32, val: T) -> io::Result<()> {
        cvt(unsafe {
            linux::setsockopt(self.raw(), level, name, &val as *const T as *const u8, core::mem::size_of::<T>() as u32)
        })
        .map(drop)
    }

    fn getsockopt<T: Copy + Default>(&self, level: u32, name: u32) -> io::Result<T> {
        let mut val = T::default();
        let mut len = core::mem::size_of::<T>() as u32;
        cvt(unsafe { linux::getsockopt(self.raw(), level, name, &mut val as *mut T as *mut u8, &mut len) })?;
        Ok(val)
    }

    fn name(&self, peer: bool) -> io::Result<SocketAddr> {
        let mut s = SockaddrStorage { bytes: [0; 128] };
        let mut len = s.bytes.len() as u32;
        cvt(unsafe {
            if peer {
                linux::getpeername(self.raw(), s.bytes.as_mut_ptr(), &mut len)
            } else {
                linux::getsockname(self.raw(), s.bytes.as_mut_ptr(), &mut len)
            }
        })?;
        decode(&s)
    }

    fn recv_with_flags(&self, buf: &mut [u8], flags: u32) -> io::Result<usize> {
        cvt_r(|| unsafe {
            linux::recvfrom(self.raw(), buf.as_mut_ptr(), buf.len(), flags, core::ptr::null_mut(), core::ptr::null_mut())
        })
    }

    fn recv_from(&self, buf: &mut [u8], flags: u32) -> io::Result<(usize, SocketAddr)> {
        let mut s = SockaddrStorage { bytes: [0; 128] };
        let mut len = s.bytes.len() as u32;
        let n = cvt_r(|| unsafe {
            linux::recvfrom(self.raw(), buf.as_mut_ptr(), buf.len(), flags, s.bytes.as_mut_ptr(), &mut len)
        })?;
        Ok((n, decode(&s)?))
    }

    fn send(&self, buf: &[u8]) -> io::Result<usize> {
        cvt_r(|| unsafe { linux::sendto(self.raw(), buf.as_ptr(), buf.len(), linux::MSG_NOSIGNAL, core::ptr::null(), 0) })
    }

    fn send_to(&self, buf: &[u8], addr: &SocketAddr) -> io::Result<usize> {
        let (s, len) = encode(addr);
        cvt_r(|| unsafe { linux::sendto(self.raw(), buf.as_ptr(), buf.len(), linux::MSG_NOSIGNAL, s.bytes.as_ptr(), len) })
    }

    fn set_timeout(&self, dur: Option<Duration>, name: u32) -> io::Result<()> {
        let tv = match dur {
            Some(d) if d.is_zero() => {
                return Err(io::const_error!(io::ErrorKind::InvalidInput, "cannot set a 0 duration timeout"));
            }
            Some(d) => [d.as_secs().min(i64::MAX as u64) as i64, d.subsec_micros() as i64],
            None => [0i64, 0],
        };
        self.setsockopt(linux::SOL_SOCKET, name, tv)
    }

    fn timeout(&self, name: u32) -> io::Result<Option<Duration>> {
        let tv: [i64; 2] = self.getsockopt(linux::SOL_SOCKET, name)?;
        if tv == [0, 0] { Ok(None) } else { Ok(Some(Duration::new(tv[0] as u64, tv[1] as u32 * 1000))) }
    }

    fn take_error(&self) -> io::Result<Option<io::Error>> {
        let e: i32 = self.getsockopt(linux::SOL_SOCKET, linux::SO_ERROR)?;
        Ok(if e == 0 { None } else { Some(io::Error::from_raw_os_error(e)) })
    }

    fn try_clone(&self) -> io::Result<Socket> {
        Ok(Socket { fd: self.fd.try_clone()? })
    }

    fn connect_timeout(&self, addr: &SocketAddr, timeout: Duration) -> io::Result<()> {
        if timeout.is_zero() {
            return Err(io::const_error!(io::ErrorKind::InvalidInput, "cannot set a 0 duration timeout"));
        }
        sfd::set_nonblocking(self.raw(), true)?;
        let (s, len) = encode(addr);
        let r = unsafe { linux::connect(self.raw(), s.bytes.as_ptr(), len) };
        match r {
            Ok(_) => {}
            Err(linux::EINPROGRESS) => {
                let start = crate::time::Instant::now();
                loop {
                    let left = timeout.checked_sub(start.elapsed()).ok_or_else(|| {
                        io::const_error!(io::ErrorKind::TimedOut, "connection timed out")
                    })?;
                    let ts = linux::Timespec { sec: left.as_secs() as i64, nsec: left.subsec_nanos() as i64 };
                    let mut pfd = linux::PollFd { fd: self.raw(), events: linux::POLLOUT, revents: 0 };
                    match unsafe { linux::ppoll(&mut pfd, 1, &ts) } {
                        Ok(0) => {}
                        Ok(_) => {
                            if let Some(e) = self.take_error()? {
                                return Err(e);
                            }
                            break;
                        }
                        Err(linux::EINTR) => {}
                        Err(e) => return Err(io::Error::from_raw_os_error(e)),
                    }
                }
            }
            Err(e) => return Err(io::Error::from_raw_os_error(e)),
        }
        sfd::set_nonblocking(self.raw(), false)
    }
}

// ---- ToSocketAddrs ----

pub trait ToSocketAddrs {
    type Iter: Iterator<Item = SocketAddr>;
    fn to_socket_addrs(&self) -> io::Result<Self::Iter>;
}

impl ToSocketAddrs for SocketAddr {
    type Iter = core::option::IntoIter<SocketAddr>;
    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        Ok(Some(*self).into_iter())
    }
}

impl ToSocketAddrs for SocketAddrV4 {
    type Iter = core::option::IntoIter<SocketAddr>;
    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        SocketAddr::V4(*self).to_socket_addrs()
    }
}

impl ToSocketAddrs for SocketAddrV6 {
    type Iter = core::option::IntoIter<SocketAddr>;
    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        SocketAddr::V6(*self).to_socket_addrs()
    }
}

impl ToSocketAddrs for (IpAddr, u16) {
    type Iter = core::option::IntoIter<SocketAddr>;
    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        SocketAddr::new(self.0, self.1).to_socket_addrs()
    }
}

impl ToSocketAddrs for (Ipv4Addr, u16) {
    type Iter = core::option::IntoIter<SocketAddr>;
    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        SocketAddr::new(IpAddr::V4(self.0), self.1).to_socket_addrs()
    }
}

impl ToSocketAddrs for (Ipv6Addr, u16) {
    type Iter = core::option::IntoIter<SocketAddr>;
    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        SocketAddr::new(IpAddr::V6(self.0), self.1).to_socket_addrs()
    }
}

fn resolve_host(host: &str, port: u16) -> io::Result<alloc_crate::vec::IntoIter<SocketAddr>> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(alloc_crate::vec![SocketAddr::new(ip, port)].into_iter());
    }
    if host.eq_ignore_ascii_case("localhost") {
        return Ok(alloc_crate::vec![
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
            SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port),
        ]
        .into_iter());
    }
    Err(io::const_error!(io::ErrorKind::Unsupported, "hostname resolution is not supported (no DNS resolver)"))
}

impl ToSocketAddrs for (&str, u16) {
    type Iter = alloc_crate::vec::IntoIter<SocketAddr>;
    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        resolve_host(self.0, self.1)
    }
}

impl ToSocketAddrs for (alloc_crate::string::String, u16) {
    type Iter = alloc_crate::vec::IntoIter<SocketAddr>;
    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        resolve_host(&self.0, self.1)
    }
}

impl ToSocketAddrs for str {
    type Iter = alloc_crate::vec::IntoIter<SocketAddr>;
    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        if let Ok(addr) = self.parse::<SocketAddr>() {
            return Ok(alloc_crate::vec![addr].into_iter());
        }
        let (host, port) = self
            .rsplit_once(':')
            .ok_or_else(|| io::const_error!(io::ErrorKind::InvalidInput, "invalid socket address"))?;
        let port: u16 = port.parse().map_err(|_| io::const_error!(io::ErrorKind::InvalidInput, "invalid port value"))?;
        resolve_host(host, port)
    }
}

impl ToSocketAddrs for alloc_crate::string::String {
    type Iter = alloc_crate::vec::IntoIter<SocketAddr>;
    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        (**self).to_socket_addrs()
    }
}

impl<'a> ToSocketAddrs for &'a [SocketAddr] {
    type Iter = core::iter::Cloned<core::slice::Iter<'a, SocketAddr>>;
    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        Ok(self.iter().cloned())
    }
}

impl<T: ToSocketAddrs + ?Sized> ToSocketAddrs for &T {
    type Iter = T::Iter;
    fn to_socket_addrs(&self) -> io::Result<T::Iter> {
        (**self).to_socket_addrs()
    }
}

/// Try `f` on each resolved address, returning the first success.
fn each_addr<A: ToSocketAddrs, T>(addr: A, mut f: impl FnMut(&SocketAddr) -> io::Result<T>) -> io::Result<T> {
    let mut last_err = None;
    for a in addr.to_socket_addrs()? {
        match f(&a) {
            Ok(v) => return Ok(v),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| io::const_error!(io::ErrorKind::InvalidInput, "could not resolve to any addresses")))
}

// ---- TCP ----

pub struct TcpStream {
    sock: Socket,
}

pub struct TcpListener {
    sock: Socket,
}

impl TcpStream {
    pub fn connect<A: ToSocketAddrs>(addr: A) -> io::Result<TcpStream> {
        each_addr(addr, |a| {
            let sock = Socket::new(a, linux::SOCK_STREAM)?;
            sock.connect(a)?;
            Ok(TcpStream { sock })
        })
    }

    pub fn connect_timeout(addr: &SocketAddr, timeout: Duration) -> io::Result<TcpStream> {
        let sock = Socket::new(addr, linux::SOCK_STREAM)?;
        sock.connect_timeout(addr, timeout)?;
        Ok(TcpStream { sock })
    }

    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.sock.name(true)
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.sock.name(false)
    }

    pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        let how = match how {
            Shutdown::Read => linux::SHUT_RD,
            Shutdown::Write => linux::SHUT_WR,
            Shutdown::Both => linux::SHUT_RDWR,
        };
        cvt(linux::shutdown(self.sock.raw(), how)).map(drop)
    }

    pub fn try_clone(&self) -> io::Result<TcpStream> {
        Ok(TcpStream { sock: self.sock.try_clone()? })
    }

    pub fn set_read_timeout(&self, dur: Option<Duration>) -> io::Result<()> {
        self.sock.set_timeout(dur, linux::SO_RCVTIMEO)
    }

    pub fn set_write_timeout(&self, dur: Option<Duration>) -> io::Result<()> {
        self.sock.set_timeout(dur, linux::SO_SNDTIMEO)
    }

    pub fn read_timeout(&self) -> io::Result<Option<Duration>> {
        self.sock.timeout(linux::SO_RCVTIMEO)
    }

    pub fn write_timeout(&self) -> io::Result<Option<Duration>> {
        self.sock.timeout(linux::SO_SNDTIMEO)
    }

    pub fn peek(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.sock.recv_with_flags(buf, linux::MSG_PEEK)
    }

    pub fn set_nodelay(&self, nodelay: bool) -> io::Result<()> {
        self.sock.setsockopt(linux::IPPROTO_TCP, linux::TCP_NODELAY, nodelay as i32)
    }

    pub fn nodelay(&self) -> io::Result<bool> {
        Ok(self.sock.getsockopt::<i32>(linux::IPPROTO_TCP, linux::TCP_NODELAY)? != 0)
    }

    pub fn set_ttl(&self, ttl: u32) -> io::Result<()> {
        self.sock.setsockopt(linux::IPPROTO_IP, linux::IP_TTL, ttl as i32)
    }

    pub fn ttl(&self) -> io::Result<u32> {
        Ok(self.sock.getsockopt::<i32>(linux::IPPROTO_IP, linux::IP_TTL)? as u32)
    }

    pub fn take_error(&self) -> io::Result<Option<io::Error>> {
        self.sock.take_error()
    }

    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        sfd::set_nonblocking(self.sock.raw(), nonblocking)
    }
}

impl Read for TcpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.sock.recv_with_flags(buf, 0)
    }
}

impl Read for &TcpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.sock.recv_with_flags(buf, 0)
    }
}

impl Write for TcpStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.sock.send(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Write for &TcpStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.sock.send(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl fmt::Debug for TcpStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("TcpStream");
        if let Ok(a) = self.local_addr() {
            d.field("addr", &a);
        }
        if let Ok(p) = self.peer_addr() {
            d.field("peer", &p);
        }
        d.field("fd", &self.sock.raw()).finish()
    }
}

impl TcpListener {
    pub fn bind<A: ToSocketAddrs>(addr: A) -> io::Result<TcpListener> {
        each_addr(addr, |a| {
            let sock = Socket::new(a, linux::SOCK_STREAM)?;
            sock.setsockopt(linux::SOL_SOCKET, linux::SO_REUSEADDR, 1i32)?;
            sock.bind(a)?;
            cvt(linux::listen(sock.raw(), 128))?;
            Ok(TcpListener { sock })
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.sock.name(false)
    }

    pub fn try_clone(&self) -> io::Result<TcpListener> {
        Ok(TcpListener { sock: self.sock.try_clone()? })
    }

    pub fn accept(&self) -> io::Result<(TcpStream, SocketAddr)> {
        let mut s = SockaddrStorage { bytes: [0; 128] };
        let mut len = s.bytes.len() as u32;
        let fd = cvt_r(|| unsafe { linux::accept4(self.sock.raw(), s.bytes.as_mut_ptr(), &mut len, linux::SOCK_CLOEXEC) })?;
        let stream = TcpStream { sock: Socket { fd: unsafe { OwnedFd::from_raw_fd(fd) } } };
        Ok((stream, decode(&s)?))
    }

    pub fn incoming(&self) -> Incoming<'_> {
        Incoming { listener: self }
    }

    pub fn into_incoming(self) -> IntoIncoming {
        IntoIncoming { listener: self }
    }

    pub fn set_ttl(&self, ttl: u32) -> io::Result<()> {
        self.sock.setsockopt(linux::IPPROTO_IP, linux::IP_TTL, ttl as i32)
    }

    pub fn ttl(&self) -> io::Result<u32> {
        Ok(self.sock.getsockopt::<i32>(linux::IPPROTO_IP, linux::IP_TTL)? as u32)
    }

    pub fn take_error(&self) -> io::Result<Option<io::Error>> {
        self.sock.take_error()
    }

    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        sfd::set_nonblocking(self.sock.raw(), nonblocking)
    }
}

impl fmt::Debug for TcpListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("TcpListener");
        if let Ok(a) = self.local_addr() {
            d.field("addr", &a);
        }
        d.field("fd", &self.sock.raw()).finish()
    }
}

#[derive(Debug)]
pub struct Incoming<'a> {
    listener: &'a TcpListener,
}

impl Iterator for Incoming<'_> {
    type Item = io::Result<TcpStream>;
    fn next(&mut self) -> Option<io::Result<TcpStream>> {
        Some(self.listener.accept().map(|p| p.0))
    }
}

#[derive(Debug)]
pub struct IntoIncoming {
    listener: TcpListener,
}

impl Iterator for IntoIncoming {
    type Item = io::Result<TcpStream>;
    fn next(&mut self) -> Option<io::Result<TcpStream>> {
        Some(self.listener.accept().map(|p| p.0))
    }
}

// ---- UDP ----

pub struct UdpSocket {
    sock: Socket,
}

impl UdpSocket {
    pub fn bind<A: ToSocketAddrs>(addr: A) -> io::Result<UdpSocket> {
        each_addr(addr, |a| {
            let sock = Socket::new(a, linux::SOCK_DGRAM)?;
            sock.bind(a)?;
            Ok(UdpSocket { sock })
        })
    }

    pub fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        self.sock.recv_from(buf, 0)
    }

    pub fn peek_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        self.sock.recv_from(buf, linux::MSG_PEEK)
    }

    pub fn send_to<A: ToSocketAddrs>(&self, buf: &[u8], addr: A) -> io::Result<usize> {
        let addrs: Vec<SocketAddr> = addr.to_socket_addrs()?.collect();
        match addrs.first() {
            Some(a) => self.sock.send_to(buf, a),
            None => Err(io::const_error!(io::ErrorKind::InvalidInput, "no addresses to send data to")),
        }
    }

    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.sock.name(true)
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.sock.name(false)
    }

    pub fn try_clone(&self) -> io::Result<UdpSocket> {
        Ok(UdpSocket { sock: self.sock.try_clone()? })
    }

    pub fn set_read_timeout(&self, dur: Option<Duration>) -> io::Result<()> {
        self.sock.set_timeout(dur, linux::SO_RCVTIMEO)
    }

    pub fn set_write_timeout(&self, dur: Option<Duration>) -> io::Result<()> {
        self.sock.set_timeout(dur, linux::SO_SNDTIMEO)
    }

    pub fn read_timeout(&self) -> io::Result<Option<Duration>> {
        self.sock.timeout(linux::SO_RCVTIMEO)
    }

    pub fn write_timeout(&self) -> io::Result<Option<Duration>> {
        self.sock.timeout(linux::SO_SNDTIMEO)
    }

    pub fn set_broadcast(&self, broadcast: bool) -> io::Result<()> {
        self.sock.setsockopt(linux::SOL_SOCKET, linux::SO_BROADCAST, broadcast as i32)
    }

    pub fn broadcast(&self) -> io::Result<bool> {
        Ok(self.sock.getsockopt::<i32>(linux::SOL_SOCKET, linux::SO_BROADCAST)? != 0)
    }

    pub fn set_ttl(&self, ttl: u32) -> io::Result<()> {
        self.sock.setsockopt(linux::IPPROTO_IP, linux::IP_TTL, ttl as i32)
    }

    pub fn ttl(&self) -> io::Result<u32> {
        Ok(self.sock.getsockopt::<i32>(linux::IPPROTO_IP, linux::IP_TTL)? as u32)
    }

    pub fn take_error(&self) -> io::Result<Option<io::Error>> {
        self.sock.take_error()
    }

    pub fn connect<A: ToSocketAddrs>(&self, addr: A) -> io::Result<()> {
        each_addr(addr, |a| self.sock.connect(a))
    }

    pub fn send(&self, buf: &[u8]) -> io::Result<usize> {
        self.sock.send(buf)
    }

    pub fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.sock.recv_with_flags(buf, 0)
    }

    pub fn peek(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.sock.recv_with_flags(buf, linux::MSG_PEEK)
    }

    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        sfd::set_nonblocking(self.sock.raw(), nonblocking)
    }
}

impl fmt::Debug for UdpSocket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("UdpSocket");
        if let Ok(a) = self.local_addr() {
            d.field("addr", &a);
        }
        d.field("fd", &self.sock.raw()).finish()
    }
}

macro_rules! sock_fd {
    ($t:ty) => {
        impl crate::os::fd::AsRawFd for $t {
            fn as_raw_fd(&self) -> crate::os::fd::RawFd {
                self.sock.raw()
            }
        }
        impl crate::os::fd::AsFd for $t {
            fn as_fd(&self) -> crate::os::fd::BorrowedFd<'_> {
                crate::os::fd::AsFd::as_fd(&self.sock.fd)
            }
        }
        impl crate::os::fd::IntoRawFd for $t {
            fn into_raw_fd(self) -> crate::os::fd::RawFd {
                crate::os::fd::IntoRawFd::into_raw_fd(self.sock.fd)
            }
        }
        impl crate::os::fd::FromRawFd for $t {
            unsafe fn from_raw_fd(fd: crate::os::fd::RawFd) -> Self {
                Self { sock: Socket { fd: unsafe { OwnedFd::from_raw_fd(fd) } } }
            }
        }
        impl From<$t> for OwnedFd {
            fn from(v: $t) -> OwnedFd {
                v.sock.fd
            }
        }
        impl From<OwnedFd> for $t {
            fn from(fd: OwnedFd) -> Self {
                Self { sock: Socket { fd } }
            }
        }
    };
}

sock_fd!(TcpStream);
sock_fd!(TcpListener);
sock_fd!(UdpSocket);
