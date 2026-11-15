//! libproc listener lookup and Security guest-by-pid validation (E-1).
//! Residual: PID reuse between lookup and validation is a TOCTOU; no claim of channel binding.
#![allow(unsafe_code)]
use crate::peer_process::PeerProcessProbe;
use security_framework::os::macos::code_signing::{
    Flags, GuestAttributes, SecCode, SecRequirement,
};
use std::str::FromStr;

pub(crate) struct MacPeerProcessProbe;
// Darwin's public 64-bit socket_fdinfo ABI, <sys/proc_info.h>, arm64 and x86_64.
// Read individual fields as bytes so no Rust union/layout/transmute is needed. Values and size
// checked against the SDK with sizeof/offsetof; a short or changed kernel reply fails closed.
const SOCKET_BYTES: usize = 792;
fn integer(bytes: &[u8], offset: usize) -> Option<i32> {
    Some(i32::from_ne_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}
fn is_listener(bytes: &[u8], port: u16) -> bool {
    bytes.len() == SOCKET_BYTES
        && integer(bytes, 184) == Some(libc::AF_INET)
        && integer(bytes, 256) == Some(2) // SOCKINFO_TCP
        && integer(bytes, 344) == Some(1) // TSI_S_LISTEN
        && integer(bytes, 268) == Some(i32::from(port.to_be()))
        // in4in6_addr has twelve padding bytes followed by the IPv4 address.
        && bytes.get(324..328) == Some(&[127, 0, 0, 1])
}
impl PeerProcessProbe for MacPeerProcessProbe {
    fn listener_pid(&self, port: u16) -> Option<i32> {
        let mut pids = vec![0i32; 65536];
        // SAFETY: buffer is writable for the exact byte size passed; libproc returns a count.
        let count =
            unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), (pids.len() * 4) as i32) };
        if count <= 0 || count as usize >= pids.len() {
            return None;
        }
        let mut found = None;
        for pid in pids.into_iter().take(count as usize).filter(|p| *p > 0) {
            // proc_fdinfo = { int32_t proc_fd; uint32_t proc_fdtype; }.
            let mut fds = vec![0i32; 32768];
            // SAFETY: buffer is writable for its declared size, pid/flavor are scalar inputs.
            let len = unsafe {
                libc::proc_pidinfo(
                    pid,
                    libc::PROC_PIDLISTFDS,
                    0,
                    fds.as_mut_ptr().cast(),
                    (fds.len() * 4) as i32,
                )
            };
            if len <= 0 || len as usize >= fds.len() * 4 || len % 8 != 0 {
                continue;
            }
            for pair in fds.as_chunks::<2>().0.iter().take(len as usize / 8) {
                let (Some(fd), Some(kind)) = (pair.first(), pair.get(1)) else {
                    continue;
                };
                if *kind != 2 {
                    continue;
                } // PROX_FDTYPE_SOCKET
                let mut info = [0u8; SOCKET_BYTES];
                // SAFETY: the kernel writes at most SOCKET_BYTES into this byte buffer; only
                // an exact-size success is parsed. No pointer escapes this synchronous call.
                let read = unsafe {
                    libc::proc_pidfdinfo(pid, *fd, 3, info.as_mut_ptr().cast(), SOCKET_BYTES as i32)
                };
                if read == SOCKET_BYTES as i32 && is_listener(&info, port) {
                    if found.is_some_and(|old| old != pid) {
                        return None;
                    }
                    found = Some(pid);
                }
            }
        }
        found
    }
    fn process_satisfies(&self, pid: i32, requirement: &str) -> bool {
        let mut attrs = GuestAttributes::new();
        attrs.set_pid(pid);
        let Ok(code) = SecCode::copy_guest_with_attribues(None, &attrs, Flags::NONE) else {
            return false;
        };
        let Ok(requirement) = SecRequirement::from_str(requirement) else {
            return false;
        };
        code.check_validity(Flags::NONE, &requirement).is_ok()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_only_an_ipv4_loopback_tcp_listener() {
        let mut b = [0u8; SOCKET_BYTES];
        for (at, val) in [
            (184, libc::AF_INET),
            (256, 2),
            (344, 1),
            (268, i32::from(4321u16.to_be())),
        ] {
            b[at..at + 4].copy_from_slice(&val.to_ne_bytes());
        }
        b[324..328].copy_from_slice(&[127, 0, 0, 1]);
        assert!(is_listener(&b, 4321));
        assert!(!is_listener(&b, 4322));
        b[324] = 0;
        assert!(!is_listener(&b, 4321));
    }
}
