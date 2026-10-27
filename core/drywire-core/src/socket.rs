//! Waiting for a datagram on a non-blocking socket.
//!
//! A call's socket is non-blocking, so that the capture callback can send
//! on it without ever waiting (ADR 0016): when the send buffer is full, the
//! packet is dropped and counted instead. The network thread still needs to
//! sleep until a packet arrives, and a read timeout only works on a
//! blocking socket, so it asks the operating system directly: `poll` on
//! macOS and Linux, `WSAPoll` on Windows. Both wake up as soon as a datagram
//! is there, exactly as a blocking `recv` would.

use std::io;
use std::net::UdpSocket;
use std::time::Duration;

/// Waits until `socket` has a datagram to read, or `timeout` passes.
/// Returns whether it is readable. An error waiting on the socket (an ICMP
/// error from the relay, say) also counts as readable: the next `recv`
/// returns it, and clears it.
pub(crate) fn wait_readable(socket: &UdpSocket, timeout: Duration) -> io::Result<bool> {
    // Both APIs count in milliseconds; round up so a short wait still waits.
    let ms = timeout.as_micros().div_ceil(1_000).min(i32::MAX as u128) as i32;
    let ready = poll(socket, ms);
    if ready < 0 {
        let e = io::Error::last_os_error();
        // A signal interrupted the wait: the caller loops anyway.
        return if e.kind() == io::ErrorKind::Interrupted {
            Ok(false)
        } else {
            Err(e)
        };
    }
    Ok(ready > 0)
}

#[cfg(unix)]
fn poll(socket: &UdpSocket, ms: i32) -> i32 {
    use std::os::fd::AsRawFd;
    let mut fd = libc::pollfd {
        fd: socket.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one valid pollfd, and the count passed is one.
    unsafe { libc::poll(&mut fd, 1, ms) }
}

#[cfg(windows)]
fn poll(socket: &UdpSocket, ms: i32) -> i32 {
    use std::os::windows::io::AsRawSocket;
    use windows::Win32::Networking::WinSock::{POLLRDNORM, SOCKET, WSAPOLLFD, WSAPoll};
    let mut fd = WSAPOLLFD {
        fd: SOCKET(socket.as_raw_socket() as usize),
        events: POLLRDNORM,
        revents: Default::default(),
    };
    // SAFETY: one valid WSAPOLLFD, and the count passed is one.
    unsafe { WSAPoll(&mut fd, 1, ms) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn wakes_up_for_a_datagram_and_times_out_without_one() {
        let a = UdpSocket::bind("127.0.0.1:0").unwrap();
        let b = UdpSocket::bind("127.0.0.1:0").unwrap();
        a.set_nonblocking(true).unwrap();
        let started = Instant::now();
        assert!(!wait_readable(&a, Duration::from_millis(20)).unwrap());
        assert!(started.elapsed() >= Duration::from_millis(20), "it waited");

        b.send_to(b"hi", a.local_addr().unwrap()).unwrap();
        assert!(wait_readable(&a, Duration::from_secs(5)).unwrap());
        let mut buf = [0u8; 8];
        assert_eq!(a.recv(&mut buf).unwrap(), 2);
        let empty = a.recv(&mut buf).unwrap_err();
        assert_eq!(empty.kind(), io::ErrorKind::WouldBlock);
    }
}
