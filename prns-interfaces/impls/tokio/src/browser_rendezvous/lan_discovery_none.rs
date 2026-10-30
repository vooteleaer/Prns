//! LAN discovery where mDNS isn't available (Haiku): `mdns-sd` needs the
//! receiving interface of each datagram (`IP_PKTINFO` through
//! `socket-pktinfo`), which Haiku's IPv4 stack doesn't report. Gateways are
//! then neither advertised nor found on the LAN; the rendezvous otherwise
//! works as elsewhere.

use prns_core::interfaces::browser_rendezvous::BrowserRendezvousId;

use crate::network_device::AutoWifiDevicePolicy;

use super::catalog::BrowserGatewayEndpoint;

pub(super) struct LanDiscovery;

impl LanDiscovery {
    pub(super) fn spawn(_id: BrowserRendezvousId, _devices: AutoWifiDevicePolicy) -> Self {
        Self
    }

    pub(super) fn set_core(&self, _is_core: bool) {}

    /// No snapshots: the caller stops polling at the first `None`.
    pub(super) async fn next_snapshot(&mut self) -> Option<Vec<BrowserGatewayEndpoint>> {
        None
    }
}
