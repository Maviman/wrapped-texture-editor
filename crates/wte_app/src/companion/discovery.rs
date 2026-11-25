//! UDP discovery (protocol 4.7), Wi-Fi only: the tablet broadcasts `"WTEC?", u8 1`; the desktop answers the sender with
//! `"WTEC!", u8 1, tcp_port u16, desktop_id [16], desktop_name str`. The desktop never broadcasts and no secret is in
//! either packet. The packet code is pure; `Responder` is the socket loop.

use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use super::server::Hub;

pub const QUERY_MAGIC: &[u8; 5] = b"WTEC?";
pub const REPLY_MAGIC: &[u8; 5] = b"WTEC!";
/// The discovery packet version.
pub const VERSION: u8 = 1;

/// The tablet's query.
#[cfg_attr(not(test), allow(dead_code, reason = "used by the tests (and the tablet side of the protocol)"))]
pub fn query_packet() -> Vec<u8> {
    let mut p = QUERY_MAGIC.to_vec();
    p.push(VERSION);
    p
}

/// A query of any version (a later tablet still gets the version-1 answer).
pub fn is_query(p: &[u8]) -> bool {
    p.len() >= 6 && &p[..5] == QUERY_MAGIC && p[5] >= 1
}

/// The desktop's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub tcp_port: u16,
    pub desktop_id: [u8; 16],
    pub desktop_name: String,
}

pub fn reply_packet(r: &Reply) -> Vec<u8> {
    let mut p = REPLY_MAGIC.to_vec();
    p.push(VERSION);
    p.extend_from_slice(&r.tcp_port.to_le_bytes());
    p.extend_from_slice(&r.desktop_id);
    let name = &r.desktop_name.as_bytes()[..r.desktop_name.len().min(255)];
    let name = std::str::from_utf8(name).map_or(&b""[..], |s| s.as_bytes());
    p.extend_from_slice(&(name.len() as u16).to_le_bytes());
    p.extend_from_slice(name);
    p
}

/// Parses an answer; `None` for anything else (trailing bytes are ignored).
#[cfg_attr(not(test), allow(dead_code, reason = "used by the tests (and the tablet side of the protocol)"))]
pub fn parse_reply(p: &[u8]) -> Option<Reply> {
    if p.len() < 5 + 1 + 2 + 16 + 2 || &p[..5] != REPLY_MAGIC || p[5] < 1 {
        return None;
    }
    let tcp_port = u16::from_le_bytes([p[6], p[7]]);
    let desktop_id: [u8; 16] = p[8..24].try_into().ok()?;
    let n = u16::from_le_bytes([p[24], p[25]]) as usize;
    let name = p.get(26..26 + n)?;
    Some(Reply { tcp_port, desktop_id, desktop_name: String::from_utf8(name.to_vec()).ok()? })
}

/// The UDP responder of a running server.
pub struct Responder {
    sock: UdpSocket,
}

impl Responder {
    pub fn bind(addr: SocketAddr) -> io::Result<Responder> {
        let sock = UdpSocket::bind(addr)?;
        sock.set_read_timeout(Some(Duration::from_millis(100)))?;
        Ok(Responder { sock })
    }

    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.sock.local_addr().ok()
    }

    /// Answers queries until the hub stops.
    pub fn run(self, hub: &Hub, tcp_port: u16) {
        let mut buf = [0u8; 512];
        let desktop_id = hub.auth.lock().map(|a| a.desktop_id).unwrap_or_default();
        let reply = reply_packet(&Reply { tcp_port, desktop_id, desktop_name: hub.cfg.desktop_name.clone() });
        while !hub.is_stopped() {
            match self.sock.recv_from(&mut buf) {
                Ok((n, from)) if is_query(&buf[..n]) => {
                    let _ = self.sock.send_to(&reply, from);
                }
                Ok(_) => {}
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
                Err(e) if e.kind() == io::ErrorKind::ConnectionReset => {} // Windows: an earlier send was refused
                Err(e) => {
                    log::warn!("companion: discovery socket: {e}");
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets_round_trip_and_garbage_is_ignored() {
        assert_eq!(query_packet(), b"WTEC?\x01");
        assert!(is_query(&query_packet()));
        assert!(is_query(b"WTEC?\x02later"), "a later version still gets an answer");
        for junk in [&b""[..], b"WTEC?", b"WTEC?\x00", b"WTEC!\x01", b"HELLO?\x01", b"\xff\xff\xff\xff\xff\xff"] {
            assert!(!is_query(junk), "{junk:?}");
            assert_eq!(parse_reply(junk), None);
        }
        let r = Reply { tcp_port: 47800, desktop_id: [3; 16], desktop_name: "STUDIO-PC".into() };
        let p = reply_packet(&r);
        assert_eq!(parse_reply(&p), Some(r.clone()));
        let mut longer = p.clone();
        longer.extend_from_slice(b"future");
        assert_eq!(parse_reply(&longer), Some(r));
        assert_eq!(parse_reply(&p[..p.len() - 1]), None, "a cut name");
    }

    /// The responder answers on 127.0.0.1 (never a LAN address in a test).
    #[test]
    fn the_responder_answers_on_loopback() {
        let cfg = super::super::server::ServerConfig { desktop_name: "TEST-PC".into(), ..Default::default() };
        let (hub, _rx) = super::super::server::Hub::new(cfg, [9; 16], Default::default(), Vec::new(), Box::new(|| {}));
        let server = super::super::server::Server::start(
            std::sync::Arc::clone(&hub),
            &[
                super::super::server::Bind::Tcp("127.0.0.1:0".parse().unwrap()),
                super::super::server::Bind::Udp("127.0.0.1:0".parse().unwrap()),
            ],
        );
        let udp = server.udp_addr.expect("bound");
        assert!(udp.ip().is_loopback());
        let tcp_port = server.tcp_addrs[0].port();
        let s = UdpSocket::bind("127.0.0.1:0").unwrap();
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        s.send_to(b"noise", udp).unwrap();
        s.send_to(&query_packet(), udp).unwrap();
        let mut buf = [0u8; 512];
        let (n, from) = s.recv_from(&mut buf).unwrap();
        assert_eq!(from, udp);
        assert_eq!(parse_reply(&buf[..n]), Some(Reply { tcp_port, desktop_id: [9; 16], desktop_name: "TEST-PC".into() }));
        server.stop();
    }
}
