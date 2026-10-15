//! The UDP socket: `[::]:<port>` with IPV6_V6ONLY off, so IPv4 players arrive on the same socket
//! as `::ffff:a.b.c.d` and are answered there.
//!
//! std cannot set IPV6_V6ONLY, and the option only works before bind (Linux returns EINVAL after,
//! Windows ignores it), while std's UdpSocket::bind does socket+bind in one call. So the socket is
//! made here with the system's own socket/setsockopt/bind (declared below; std already links libc
//! and ws2_32) and handed to std. Windows' sockets on `[::]` are v6-only by default, so without
//! this IPv4 players could not connect there. If that fails (no IPv6 on the machine), the server
//! falls back to an IPv4-only `0.0.0.0` socket and says so.

use std::{
    io,
    net::{Ipv4Addr, SocketAddr, UdpSocket},
};

/// Binds the server's socket. The bool is true for the dual-stack socket, false for the IPv4-only
/// fallback.
pub fn bind_dual(port: u16) -> io::Result<(UdpSocket, bool)> {
    match sys::bind_dual_stack(port) {
        Ok(s) => Ok((s, true)),
        // another program has the port: the fallback would fail the same way, and the message is
        // clearer from here
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => Err(e),
        Err(_) => Ok((UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port))?, false)),
    }
}

/// An address as people write it: IPv4 players without the `::ffff:` prefix.
pub fn show(addr: &SocketAddr) -> String {
    match addr {
        SocketAddr::V6(v6) => match v6.ip().to_ipv4_mapped() {
            Some(v4) => format!("{v4}:{}", v6.port()),
            None => addr.to_string(),
        },
        SocketAddr::V4(_) => addr.to_string(),
    }
}

/// sockaddr_in6 (the same layout on Linux and Windows): family, port (big-endian), flow info,
/// address, scope id. 28 bytes.
#[allow(dead_code)]
#[repr(C)]
struct SockaddrIn6 {
    family: u16,
    port: u16,
    flowinfo: u32,
    addr: [u8; 16],
    scope_id: u32,
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod sys {
    use super::SockaddrIn6;
    use std::{
        ffi::{c_int, c_void},
        io,
        net::UdpSocket,
        os::fd::FromRawFd,
    };

    const AF_INET6: c_int = 10;
    const SOCK_DGRAM: c_int = 2;
    const SOCK_CLOEXEC: c_int = 0o2000000;
    const IPPROTO_IPV6: c_int = 41;
    const IPV6_V6ONLY: c_int = 26;

    unsafe extern "C" {
        fn socket(domain: c_int, ty: c_int, protocol: c_int) -> c_int;
        fn setsockopt(fd: c_int, level: c_int, name: c_int, value: *const c_void, len: u32) -> c_int;
        fn bind(fd: c_int, addr: *const c_void, len: u32) -> c_int;
        fn close(fd: c_int) -> c_int;
    }

    pub fn bind_dual_stack(port: u16) -> io::Result<UdpSocket> {
        unsafe {
            let fd = socket(AF_INET6, SOCK_DGRAM | SOCK_CLOEXEC, 0);
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let off: c_int = 0;
            let sa = SockaddrIn6 { family: AF_INET6 as u16, port: port.to_be(), flowinfo: 0, addr: [0; 16], scope_id: 0 };
            if setsockopt(fd, IPPROTO_IPV6, IPV6_V6ONLY, &off as *const c_int as *const c_void, 4) != 0
                || bind(fd, &sa as *const SockaddrIn6 as *const c_void, size_of::<SockaddrIn6>() as u32) != 0
            {
                let e = io::Error::last_os_error();
                close(fd);
                return Err(e);
            }
            Ok(UdpSocket::from_raw_fd(fd))
        }
    }
}

#[cfg(windows)]
mod sys {
    use super::SockaddrIn6;
    use std::{
        io,
        net::{Ipv4Addr, UdpSocket},
        os::windows::io::FromRawSocket,
    };

    const AF_INET6: i32 = 23;
    const SOCK_DGRAM: i32 = 2;
    const IPPROTO_IPV6: i32 = 41;
    const IPV6_V6ONLY: i32 = 27;
    const INVALID_SOCKET: usize = !0;

    #[link(name = "ws2_32")]
    unsafe extern "system" {
        fn socket(af: i32, ty: i32, protocol: i32) -> usize;
        fn setsockopt(s: usize, level: i32, name: i32, value: *const u8, len: i32) -> i32;
        fn bind(s: usize, name: *const u8, len: i32) -> i32;
        fn closesocket(s: usize) -> i32;
        fn WSAGetLastError() -> i32;
    }

    pub fn bind_dual_stack(port: u16) -> io::Result<UdpSocket> {
        // std calls WSAStartup the first time it makes a socket; this throwaway one does that
        drop(UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)));
        unsafe {
            let s = socket(AF_INET6, SOCK_DGRAM, 0);
            if s == INVALID_SOCKET {
                return Err(io::Error::from_raw_os_error(WSAGetLastError()));
            }
            let off: u32 = 0;
            let sa = SockaddrIn6 { family: AF_INET6 as u16, port: port.to_be(), flowinfo: 0, addr: [0; 16], scope_id: 0 };
            if setsockopt(s, IPPROTO_IPV6, IPV6_V6ONLY, &off as *const u32 as *const u8, 4) != 0
                || bind(s, &sa as *const SockaddrIn6 as *const u8, size_of::<SockaddrIn6>() as i32) != 0
            {
                let e = io::Error::from_raw_os_error(WSAGetLastError());
                closesocket(s);
                return Err(e);
            }
            Ok(UdpSocket::from_raw_socket(s as u64))
        }
    }
}

/// Elsewhere (macOS, BSDs): a plain `[::]` bind, dual-stack or not as the system defaults.
#[cfg(not(any(target_os = "linux", target_os = "android", windows)))]
mod sys {
    use std::{
        io,
        net::{Ipv6Addr, UdpSocket},
    };

    pub fn bind_dual_stack(port: u16) -> io::Result<UdpSocket> {
        UdpSocket::bind((Ipv6Addr::UNSPECIFIED, port))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn dual_stack_takes_ipv4() {
        let (sock, dual) = match bind_dual(0) {
            Ok(x) => x,
            Err(e) => {
                eprintln!("skipped: cannot bind: {e}");
                return;
            }
        };
        if !dual {
            eprintln!("skipped: no IPv6 here, IPv4-only fallback");
            return;
        }
        sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let port = sock.local_addr().unwrap().port();
        let client = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        client.send_to(b"hi", (Ipv4Addr::LOCALHOST, port)).unwrap();
        let mut buf = [0u8; 16];
        let (n, from) = sock.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"hi");
        assert_eq!(show(&from), client.local_addr().unwrap().to_string());
        // and the answer goes back over IPv4
        sock.send_to(b"ok", from).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let (n, _) = client.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"ok");
    }

    #[test]
    fn port_in_use_is_an_error() {
        let Ok((a, _)) = bind_dual(0) else { return };
        let port = a.local_addr().unwrap().port();
        assert!(bind_dual(port).is_err());
    }
}
