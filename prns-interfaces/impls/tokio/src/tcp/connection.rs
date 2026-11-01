#[cfg(feature = "tcp")]
use crate::reconnect::ReconnectPolicy;
#[cfg(feature = "tcp")]
use std::io;
#[cfg(feature = "tcp")]
use std::net::SocketAddr;
use std::time::Duration;

use socket2::{SockRef, TcpKeepalive};
use tokio::net::TcpStream;

#[cfg(feature = "tcp")]
use crate::byte_stream::framing::WriteProgressTimeout;

#[cfg(feature = "tcp")]
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Match the reference keepalive discipline: probe after 5 seconds idle and every 2 seconds
/// thereafter. The byte-stream driver independently enforces the same 24-second write-progress
/// deadline on every host.
#[cfg(feature = "tcp")]
const TCP_PROBE_AFTER: Duration = Duration::from_secs(5);
#[cfg(feature = "tcp")]
const TCP_PROBE_INTERVAL: Duration = Duration::from_secs(2);
#[cfg(feature = "tcp")]
const TCP_PROBES: u32 = 12;
#[cfg(feature = "tcp")]
const TCP_USER_TIMEOUT: Duration = Duration::from_secs(24);
const I2P_PROBE_AFTER: Duration = Duration::from_secs(10);
const I2P_PROBE_INTERVAL: Duration = Duration::from_secs(9);
const I2P_PROBES: u32 = 5;
const I2P_USER_TIMEOUT: Duration = Duration::from_secs(45);

#[cfg(feature = "tcp")]
pub(crate) const fn write_progress_timeout(tunnel: TcpTunnelMode) -> WriteProgressTimeout {
    let timeout = match tunnel {
        #[cfg(feature = "tcp")]
        TcpTunnelMode::Direct => TCP_USER_TIMEOUT,
        TcpTunnelMode::I2p => I2P_USER_TIMEOUT,
    };
    WriteProgressTimeout::after(timeout)
}

#[cfg(feature = "tcp")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressFamilyPreference {
    System,
    Ipv4,
    Ipv6,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpTunnelMode {
    #[cfg(feature = "tcp")]
    Direct,
    I2p,
}

#[cfg(feature = "tcp")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectLimit {
    Unlimited,
    Attempts(u32),
}

#[cfg(feature = "tcp")]
impl ReconnectLimit {
    pub const fn exhausted(self, completed_attempts: u32) -> bool {
        match self {
            Self::Unlimited => false,
            Self::Attempts(limit) => completed_attempts >= limit,
        }
    }
}

#[cfg(feature = "tcp")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TcpConnectionSettings {
    pub connect_timeout: Duration,
    pub reconnect_policy: ReconnectPolicy,
    pub reconnect_limit: ReconnectLimit,
    pub address_family: AddressFamilyPreference,
    pub tunnel: TcpTunnelMode,
}

#[cfg(feature = "tcp")]
impl TcpConnectionSettings {
    pub const STOCK: Self = Self {
        connect_timeout: CONNECT_TIMEOUT,
        reconnect_policy: ReconnectPolicy::STANDARD,
        reconnect_limit: ReconnectLimit::Unlimited,
        address_family: AddressFamilyPreference::System,
        tunnel: TcpTunnelMode::Direct,
    };
}

#[cfg(feature = "tcp")]
pub(crate) async fn connect(
    target: &str,
    settings: TcpConnectionSettings,
) -> io::Result<TcpStream> {
    // Every address the name has, the preferred one first: a host with an
    // IPv6 address is still reached over IPv4 where this machine has no IPv6
    // route (the resolver may list IPv6 first regardless).
    let mut last = None;
    for address in resolve(target, settings.address_family).await? {
        match tokio::time::timeout(settings.connect_timeout, TcpStream::connect(address)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) => last = Some(error),
            Err(_) => {
                last = Some(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "TCP connect timed out",
                ))
            }
        }
    }
    Err(last.unwrap_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "TCP target resolved to nothing")
    }))
}

#[cfg(feature = "tcp")]
async fn resolve(
    target: &str,
    preference: AddressFamilyPreference,
) -> io::Result<std::vec::Vec<SocketAddr>> {
    let addresses = tokio::net::lookup_host(target)
        .await?
        .collect::<std::vec::Vec<_>>();
    Ok(connect_order(&addresses, preference))
}

/// The addresses to try, in order: the one `select_address` picks first,
/// then the rest as the resolver listed them.
#[cfg(feature = "tcp")]
fn connect_order(
    addresses: &[SocketAddr],
    preference: AddressFamilyPreference,
) -> std::vec::Vec<SocketAddr> {
    let mut order = std::vec::Vec::with_capacity(addresses.len());
    if let Some(first) = select_address(addresses, preference) {
        order.push(first);
    }
    for address in addresses {
        if !order.contains(address) {
            order.push(*address);
        }
    }
    order
}

#[cfg(feature = "tcp")]
fn select_address(
    addresses: &[SocketAddr],
    preference: AddressFamilyPreference,
) -> Option<SocketAddr> {
    let preferred = match preference {
        AddressFamilyPreference::System => return addresses.first().copied(),
        AddressFamilyPreference::Ipv4 => addresses.iter().find(|address| address.is_ipv4()),
        AddressFamilyPreference::Ipv6 => addresses.iter().find(|address| address.is_ipv6()),
    };
    preferred.copied().or_else(|| addresses.first().copied())
}

/// Socket tuning is best-effort because a socket that refuses an option can still carry frames.
#[cfg(feature = "tcp")]
pub fn tune(stream: &TcpStream) {
    tune_for_tunnel(stream, TcpTunnelMode::Direct);
}

#[cfg(feature = "i2p")]
pub(crate) fn tune_i2p(stream: &TcpStream) {
    tune_for_tunnel(stream, TcpTunnelMode::I2p);
}

pub(crate) fn tune_for_tunnel(stream: &TcpStream, tunnel: TcpTunnelMode) {
    let _ = stream.set_nodelay(true);
    let socket = SockRef::from(stream);
    let (probe_after, _probe_interval, _probes, _user_timeout) = match tunnel {
        #[cfg(feature = "tcp")]
        TcpTunnelMode::Direct => (
            TCP_PROBE_AFTER,
            TCP_PROBE_INTERVAL,
            TCP_PROBES,
            TCP_USER_TIMEOUT,
        ),
        TcpTunnelMode::I2p => (
            I2P_PROBE_AFTER,
            I2P_PROBE_INTERVAL,
            I2P_PROBES,
            I2P_USER_TIMEOUT,
        ),
    };
    let keepalive = TcpKeepalive::new().with_time(probe_after);
    // socket2 exposes no keepalive interval or probe count on Haiku, so Haiku keeps only the idle time.
    #[cfg(not(target_os = "haiku"))]
    let keepalive = keepalive
        .with_interval(_probe_interval)
        .with_retries(_probes);
    let _ = socket.set_tcp_keepalive(&keepalive);
    #[cfg(target_os = "linux")]
    let _ = socket.set_tcp_user_timeout(Some(_user_timeout));
}

#[cfg(all(test, feature = "tcp"))]
mod tests {
    use super::*;

    #[test]
    fn address_selection_honors_the_requested_family_with_a_system_fallback() {
        let v6 = "[::1]:4242".parse().expect("valid IPv6 address");
        let v4 = "127.0.0.1:4242".parse().expect("valid IPv4 address");
        let addresses = [v6, v4];

        assert_eq!(
            select_address(&addresses, AddressFamilyPreference::System),
            Some(v6)
        );
        assert_eq!(
            select_address(&addresses, AddressFamilyPreference::Ipv4),
            Some(v4)
        );
        assert_eq!(
            select_address(&[v4], AddressFamilyPreference::Ipv6),
            Some(v4)
        );
    }

    #[test]
    fn every_resolved_address_is_tried_the_preferred_first() {
        let v6 = "[2001:db8::1]:4242".parse().expect("valid IPv6 address");
        let v4 = "192.0.2.1:4242".parse().expect("valid IPv4 address");
        let addresses = [v6, v4];

        assert_eq!(
            connect_order(&addresses, AddressFamilyPreference::System),
            [v6, v4]
        );
        assert_eq!(
            connect_order(&addresses, AddressFamilyPreference::Ipv4),
            [v4, v6]
        );
        assert!(connect_order(&[], AddressFamilyPreference::System).is_empty());
    }

    #[tokio::test]
    async fn an_unreachable_first_address_falls_back_to_the_next() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a local listener");
        let port = listener.local_addr().expect("its address").port();
        // "localhost" is ::1 and 127.0.0.1 on most systems, ::1 often first;
        // only the IPv4 one listens, so the IPv6 one refuses and the next is
        // taken.
        let target = format!("localhost:{port}");
        assert!(connect(&target, TcpConnectionSettings::STOCK).await.is_ok());
    }

    #[test]
    fn reconnect_limits_count_only_reconnect_attempts() {
        assert!(!ReconnectLimit::Unlimited.exhausted(u32::MAX));
        assert!(ReconnectLimit::Attempts(0).exhausted(0));
        assert!(!ReconnectLimit::Attempts(2).exhausted(1));
        assert!(ReconnectLimit::Attempts(2).exhausted(2));
    }

    #[test]
    fn write_progress_deadlines_follow_the_tunnel_liveness_policy() {
        assert_eq!(
            write_progress_timeout(TcpTunnelMode::Direct),
            WriteProgressTimeout::after(Duration::from_secs(24))
        );
        assert_eq!(
            write_progress_timeout(TcpTunnelMode::I2p),
            WriteProgressTimeout::after(Duration::from_secs(45))
        );
    }
}
