//! Bluetooth RFCOMM listener for Windows (COMPANION_PLAN 6.3): Winsock with `AF_BTH`, imported through `raw-dylib`
//! (`ws2_32`; no crate). The struct definitions follow `windows-sys` 0.61.2 (reference only).
//!
//! Decision C14: this code is compiled and its layouts are unit-tested, but agents never execute it (no advertising, no
//! pairing on the build machine). It runs only when the user turns the Bluetooth switch on (never in a self-test run:
//! `bind_plan` gives no `Bind::Rfcomm` there), and the adapter check runs only when the user opens the dialog.

#![cfg(windows)]

use std::io;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use super::server::{Hub, Link};

/// The service UUID both sides use: `7c1f0d6a-5b0e-4e7a-9c56-57544543a001`.
pub const SERVICE_UUID: &str = "7c1f0d6a-5b0e-4e7a-9c56-57544543a001";
/// The service name registered with the SDP server.
pub const SERVICE_NAME: &str = "Wrapped Texture Editor companion";

const AF_BTH: i32 = 32;
const SOCK_STREAM: i32 = 1;
const BTHPROTO_RFCOMM: i32 = 3;
const BT_PORT_ANY: u32 = 0xFFFF_FFFF;
const NS_BTH: u32 = 16;
const RNRSERVICE_REGISTER: i32 = 0;
const RNRSERVICE_DELETE: i32 = 2;
const INVALID_SOCKET: usize = usize::MAX;
const SOCKET_ERROR: i32 = -1;
const FIONBIO: i32 = 0x8004_667Eu32 as i32;
const SOL_SOCKET: i32 = 0xFFFF;
const SO_RCVTIMEO: i32 = 0x1006;
const SD_BOTH: i32 = 2;
const WSAEWOULDBLOCK: i32 = 10035;
const WSAETIMEDOUT: i32 = 10060;

/// `GUID`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

/// `SOCKADDR_BTH` (`#pragma pack(1)`, 30 bytes).
#[repr(C, packed(1))]
#[derive(Copy, Clone, Default)]
pub struct SockaddrBth {
    pub address_family: u16,
    pub bt_addr: u64,
    pub service_class_id: Guid,
    pub port: u32,
}

/// `SOCKET_ADDRESS`.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct SocketAddress {
    pub lp_sockaddr: *mut u8,
    pub i_sockaddr_length: i32,
}

/// `CSADDR_INFO`.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct CsaddrInfo {
    pub local_addr: SocketAddress,
    pub remote_addr: SocketAddress,
    pub i_socket_type: i32,
    pub i_protocol: i32,
}

/// `WSAQUERYSETW`.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct WsaQuerySetW {
    pub dw_size: u32,
    pub lpsz_service_instance_name: *mut u16,
    pub lp_service_class_id: *mut Guid,
    pub lp_version: *mut u8,
    pub lpsz_comment: *mut u16,
    pub dw_name_space: u32,
    pub lp_ns_provider_id: *mut Guid,
    pub lpsz_context: *mut u16,
    pub dw_number_of_protocols: u32,
    pub lpafp_protocols: *mut u8,
    pub lpsz_query_string: *mut u16,
    pub dw_number_of_cs_addrs: u32,
    pub lpcsa_buffer: *mut CsaddrInfo,
    pub dw_output_flags: u32,
    pub lp_blob: *mut u8,
}

#[link(name = "ws2_32", kind = "raw-dylib")]
unsafe extern "system" {
    fn WSAStartup(version: u16, data: *mut u8) -> i32;
    fn WSAGetLastError() -> i32;
    fn socket(af: i32, ty: i32, protocol: i32) -> usize;
    fn bind(s: usize, name: *const u8, namelen: i32) -> i32;
    fn listen(s: usize, backlog: i32) -> i32;
    fn getsockname(s: usize, name: *mut u8, namelen: *mut i32) -> i32;
    fn accept(s: usize, addr: *mut u8, addrlen: *mut i32) -> usize;
    fn ioctlsocket(s: usize, cmd: i32, argp: *mut u32) -> i32;
    fn recv(s: usize, buf: *mut u8, len: i32, flags: i32) -> i32;
    fn send(s: usize, buf: *const u8, len: i32, flags: i32) -> i32;
    fn setsockopt(s: usize, level: i32, optname: i32, optval: *const u8, optlen: i32) -> i32;
    fn shutdown(s: usize, how: i32) -> i32;
    fn closesocket(s: usize) -> i32;
    fn WSASetServiceW(info: *const WsaQuerySetW, op: i32, flags: u32) -> i32;
}

/// Parses a UUID string into a GUID (the first three fields little-endian in memory, as Windows stores them).
pub fn guid_from_uuid(uuid: &str) -> Option<Guid> {
    let hex: String = uuid.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok();
    let mut b = [0u8; 16];
    for (i, v) in b.iter_mut().enumerate() {
        *v = byte(i)?;
    }
    Some(Guid {
        data1: u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        data2: u16::from_be_bytes([b[4], b[5]]),
        data3: u16::from_be_bytes([b[6], b[7]]),
        data4: [b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]],
    })
}

/// The GUID's bytes as they lie in memory.
#[cfg(test)]
pub fn guid_bytes(g: &Guid) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[..4].copy_from_slice(&g.data1.to_le_bytes());
    out[4..6].copy_from_slice(&g.data2.to_le_bytes());
    out[6..8].copy_from_slice(&g.data3.to_le_bytes());
    out[8..].copy_from_slice(&g.data4);
    out
}

fn last_error() -> io::Error {
    // SAFETY: no arguments; reads the calling thread's Winsock error.
    io::Error::from_raw_os_error(unsafe { WSAGetLastError() })
}

fn startup() -> io::Result<()> {
    let mut data = [0u8; 512];
    // SAFETY: `data` is larger than WSADATA (408 bytes on x64).
    let r = unsafe { WSAStartup(0x0202, data.as_mut_ptr()) };
    if r == 0 { Ok(()) } else { Err(io::Error::from_raw_os_error(r)) }
}

/// An owned RFCOMM socket; closed on drop.
struct Sock(usize);

impl Drop for Sock {
    fn drop(&mut self) {
        // SAFETY: the socket is owned and closed once.
        unsafe { closesocket(self.0) };
    }
}

/// True when a Bluetooth adapter can make an RFCOMM socket (read-only check for the dialog; never in a self-test run).
pub fn adapter_present() -> bool {
    if startup().is_err() {
        return false;
    }
    // SAFETY: plain socket creation; closed by `Sock`.
    let s = unsafe { socket(AF_BTH, SOCK_STREAM, BTHPROTO_RFCOMM) };
    if s == INVALID_SOCKET {
        return false;
    }
    drop(Sock(s));
    true
}

/// An accepted RFCOMM connection as a `Link`.
pub struct RfcommLink(Arc<Sock>);

impl io::Read for RfcommLink {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = buf.len().min(i32::MAX as usize) as i32;
        // SAFETY: `buf` is valid for `len` bytes.
        let n = unsafe { recv(self.0.0, buf.as_mut_ptr(), len, 0) };
        if n >= 0 {
            return Ok(n as usize);
        }
        let e = last_error();
        Err(match e.raw_os_error() {
            Some(WSAETIMEDOUT) => io::Error::new(io::ErrorKind::TimedOut, e),
            Some(WSAEWOULDBLOCK) => io::Error::new(io::ErrorKind::WouldBlock, e),
            _ => e,
        })
    }
}

impl io::Write for RfcommLink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let len = buf.len().min(i32::MAX as usize) as i32;
        // SAFETY: `buf` is valid for `len` bytes.
        let n = unsafe { send(self.0.0, buf.as_ptr(), len, 0) };
        if n == SOCKET_ERROR { Err(last_error()) } else { Ok(n as usize) }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Link for RfcommLink {
    fn try_clone_link(&self) -> io::Result<Box<dyn Link>> {
        Ok(Box::new(RfcommLink(Arc::clone(&self.0))))
    }
    fn shutdown(&self) {
        // SAFETY: shutting down an open socket; harmless on a closed connection.
        unsafe { shutdown(self.0.0, SD_BOTH) };
    }
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        let ms: u32 = timeout.map_or(0, |t| t.as_millis().clamp(1, u128::from(u32::MAX)) as u32);
        // SAFETY: `ms` lives across the call; the option takes a DWORD.
        let r = unsafe { setsockopt(self.0.0, SOL_SOCKET, SO_RCVTIMEO, (&raw const ms).cast(), 4) };
        if r == SOCKET_ERROR { Err(last_error()) } else { Ok(()) }
    }
    fn peer_loopback(&self) -> bool {
        false
    }
    fn is_rfcomm(&self) -> bool {
        true
    }
}

fn set_nonblocking(s: usize, on: bool) -> io::Result<()> {
    let mut v: u32 = u32::from(on);
    // SAFETY: FIONBIO takes a pointer to a u_long.
    let r = unsafe { ioctlsocket(s, FIONBIO, &raw mut v) };
    if r == SOCKET_ERROR { Err(last_error()) } else { Ok(()) }
}

/// Registers (`RNRSERVICE_REGISTER`) or deletes the SDP record of the listener at `local`.
fn register(op: i32, local: &mut SockaddrBth, name: &mut [u16], class: &mut Guid) -> i32 {
    let mut csa = CsaddrInfo {
        local_addr: SocketAddress { lp_sockaddr: (local as *mut SockaddrBth).cast(), i_sockaddr_length: std::mem::size_of::<SockaddrBth>() as i32 },
        remote_addr: SocketAddress { lp_sockaddr: std::ptr::null_mut(), i_sockaddr_length: 0 },
        i_socket_type: SOCK_STREAM,
        i_protocol: BTHPROTO_RFCOMM,
    };
    let query = WsaQuerySetW {
        dw_size: std::mem::size_of::<WsaQuerySetW>() as u32,
        lpsz_service_instance_name: name.as_mut_ptr(),
        lp_service_class_id: class as *mut Guid,
        lp_version: std::ptr::null_mut(),
        lpsz_comment: std::ptr::null_mut(),
        dw_name_space: NS_BTH,
        lp_ns_provider_id: std::ptr::null_mut(),
        lpsz_context: std::ptr::null_mut(),
        dw_number_of_protocols: 0,
        lpafp_protocols: std::ptr::null_mut(),
        lpsz_query_string: std::ptr::null_mut(),
        dw_number_of_cs_addrs: 1,
        lpcsa_buffer: &raw mut csa,
        dw_output_flags: 0,
        lp_blob: std::ptr::null_mut(),
    };
    // SAFETY: every pointer in `query` points to data alive for the call.
    unsafe { WSASetServiceW(&raw const query, op, 0) }
}

/// Opens the RFCOMM listener, registers the service and starts the accept loop (non-blocking accept with a 50 ms poll;
/// the service is deleted and the socket closed when the server stops). Returns the thread and the RFCOMM channel.
pub fn listen_rfcomm(hub: Arc<Hub>) -> io::Result<(JoinHandle<()>, u32)> {
    startup()?;
    // SAFETY: plain socket creation.
    let raw = unsafe { socket(AF_BTH, SOCK_STREAM, BTHPROTO_RFCOMM) };
    if raw == INVALID_SOCKET {
        return Err(last_error());
    }
    let sock = Sock(raw);
    let addr = SockaddrBth { address_family: AF_BTH as u16, bt_addr: 0, service_class_id: Guid::default(), port: BT_PORT_ANY };
    let len = std::mem::size_of::<SockaddrBth>() as i32;
    // SAFETY: `addr` is a valid SOCKADDR_BTH of `len` bytes.
    if unsafe { bind(sock.0, (&raw const addr).cast(), len) } == SOCKET_ERROR {
        return Err(last_error());
    }
    // SAFETY: a bound socket.
    if unsafe { listen(sock.0, 1) } == SOCKET_ERROR {
        return Err(last_error());
    }
    let mut local = SockaddrBth::default();
    let mut local_len = len;
    // SAFETY: `local` has room for a SOCKADDR_BTH.
    if unsafe { getsockname(sock.0, (&raw mut local).cast(), &raw mut local_len) } == SOCKET_ERROR {
        return Err(last_error());
    }
    let channel = local.port;
    set_nonblocking(sock.0, true)?;
    let mut name: Vec<u16> = SERVICE_NAME.encode_utf16().chain(std::iter::once(0)).collect();
    let mut class = guid_from_uuid(SERVICE_UUID).unwrap_or_default();
    if register(RNRSERVICE_REGISTER, &mut local, &mut name, &mut class) == SOCKET_ERROR {
        return Err(last_error());
    }
    log::info!("companion: Bluetooth RFCOMM listening on channel {channel}");
    let thread = super::server::spawn_named("wte-companion-rfcomm", move || {
        while !hub.is_stopped() {
            // SAFETY: accept on a listening non-blocking socket; no peer address wanted.
            let s = unsafe { accept(sock.0, std::ptr::null_mut(), std::ptr::null_mut()) };
            if s == INVALID_SOCKET {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            let _ = set_nonblocking(s, false);
            log::info!("companion: Bluetooth connection");
            let link = RfcommLink(Arc::new(Sock(s)));
            let hub = Arc::clone(&hub);
            super::server::spawn_named("wte-companion-session", move || super::server::run_session(Box::new(link), hub));
        }
        register(RNRSERVICE_DELETE, &mut local, &mut name, &mut class);
        drop(sock);
    });
    Ok((thread, channel))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn struct_layouts() {
        assert_eq!(size_of::<SockaddrBth>(), 30);
        assert_eq!(offset_of!(SockaddrBth, address_family), 0);
        assert_eq!(offset_of!(SockaddrBth, bt_addr), 2);
        assert_eq!(offset_of!(SockaddrBth, service_class_id), 10);
        assert_eq!(offset_of!(SockaddrBth, port), 26);
        assert_eq!(size_of::<Guid>(), 16);
        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(size_of::<SocketAddress>(), 16);
            assert_eq!(size_of::<CsaddrInfo>(), 40);
            assert_eq!(offset_of!(CsaddrInfo, i_socket_type), 32);
            assert_eq!(size_of::<WsaQuerySetW>(), 120);
            assert_eq!(offset_of!(WsaQuerySetW, dw_name_space), 40);
            assert_eq!(offset_of!(WsaQuerySetW, dw_number_of_cs_addrs), 88);
            assert_eq!(offset_of!(WsaQuerySetW, lpcsa_buffer), 96);
            assert_eq!(offset_of!(WsaQuerySetW, lp_blob), 112);
        }
    }

    #[test]
    fn uuid_to_guid_bytes() {
        let g = guid_from_uuid(SERVICE_UUID).unwrap();
        assert_eq!(g.data1, 0x7c1f_0d6a);
        assert_eq!(g.data2, 0x5b0e);
        assert_eq!(g.data3, 0x4e7a);
        assert_eq!(guid_bytes(&g), [0x6a, 0x0d, 0x1f, 0x7c, 0x0e, 0x5b, 0x7a, 0x4e, 0x9c, 0x56, 0x57, 0x54, 0x45, 0x43, 0xa0, 0x01]);
        assert_eq!(guid_from_uuid("7c1f0d6a-5b0e-4e7a-9c56"), None);
        assert_eq!(guid_from_uuid("zc1f0d6a-5b0e-4e7a-9c56-57544543a001"), None);
    }
}
