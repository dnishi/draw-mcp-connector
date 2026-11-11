//! Small helpers for networking and random numbers.

use std::io;
use std::net::{IpAddr, Ipv4Addr, UdpSocket};

/// This PC's LAN IP address, as reached from the canvas page.
/// A UDP connect only picks a route and sends no packets, so it reveals the source address used on the default route
pub fn lan_ip() -> io::Result<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    // 192.0.2.1 is a documentation address (RFC 5737). No traffic is actually sent
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9))?;
    Ok(socket.local_addr()?.ip())
}

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Generates `bytes` cryptographically secure random bytes and returns them as a hex string
pub fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0; bytes];
    getrandom::fill(&mut buffer).expect("OS の乱数を取得できない");
    to_hex(&buffer)
}

/// String comparison whose duration does not depend on the contents (used to check tokens)
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0, |diff, (x, y)| diff | (x ^ y))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_and_random() {
        assert_eq!(to_hex(&[0x00, 0xab, 0x0f]), "00ab0f");
        let token = random_hex(16);
        assert_eq!(token.len(), 32);
        assert_ne!(token, random_hex(16));
    }

    #[test]
    fn compares_tokens() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "ab"));
    }
}
