//! Fini's iroh address lookup over mDNS, on the `mdns-sd` crate.
//!
//! It announces this endpoint as `<endpoint id>._fini._udp.local.` and
//! looks for the other Fini endpoints. `mdns-sd` sends and receives on
//! every IPv4 interface, and gives on each interface the addresses of that
//! interface. `iroh-mdns-address-lookup` (swarm-discovery) used only the
//! interface of the default route, so a Windows host with WSL announced on
//! `vEthernet (WSL)` and not on Wi-Fi (n0-computer/iroh#3533).
//!
//! The record: the instance name is the endpoint id in lower-case base32,
//! the SRV port is the endpoint's UDP port, and the TXT record has
//! `user-data=<the iroh user data>`.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::stream::{self, BoxStream};
use iroh::address_lookup::{AddressLookup, EndpointData, EndpointInfo, Error, Item};
use iroh::endpoint_info::UserData;
use iroh::EndpointId;
use mdns_sd::{IfKind, ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::sync::mpsc;

/// The mDNS service type of Fini's Network channel. Only Fini endpoints
/// use it, so other iroh apps on the network are not heard.
pub const SERVICE_TYPE: &str = "_fini._udp.local.";

/// The TXT key of the iroh user data.
const USER_DATA_KEY: &str = "user-data";

/// The name of this lookup in the iroh `Item`s it gives.
const PROVENANCE: &str = "fini-mdns";

/// How often the lookup sends again the endpoints that it still has.
/// A record stays until its SRV record expires (`mdns-sd` asks for it
/// again before that), so this keeps the Network channel's "last heard"
/// time current.
const REPEAT_INTERVAL: Duration = Duration::from_secs(5);

/// What the lookup heard.
#[derive(Debug, Clone)]
pub enum MdnsEvent {
    /// An endpoint is on the network, or its record changed.
    Discovered(EndpointInfo),
    /// The record of the endpoint expired, or the endpoint removed its record.
    Expired(EndpointId),
}

/// The record this endpoint announces.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Record {
    port: u16,
    user_data: Option<String>,
}

pub struct MdnsLookup {
    daemon: ServiceDaemon,
    /// The instance name: the endpoint id in lower-case base32.
    instance: String,
    /// The record that is registered now.
    record: Mutex<Record>,
    /// The endpoints heard and not expired, for `resolve`.
    peers: Arc<Mutex<HashMap<EndpointId, EndpointInfo>>>,
    browse_task: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for MdnsLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MdnsLookup").field("instance", &self.instance).finish_non_exhaustive()
    }
}

impl MdnsLookup {
    /// Starts the mDNS daemon, announces this endpoint on `port` with
    /// `user_data`, and looks for the other Fini endpoints. Call it in
    /// a tokio runtime. The receiver gets what the lookup hears.
    pub fn new(
        endpoint_id: EndpointId,
        port: u16,
        user_data: Option<UserData>,
    ) -> Result<(Self, mpsc::Receiver<MdnsEvent>), String> {
        let daemon = ServiceDaemon::new().map_err(|err| format!("the mDNS daemon did not start: {err}"))?;
        // The endpoint binds IPv4 only, so IPv6 records are of no use.
        daemon.disable_interface(IfKind::IPv6).map_err(|err| err.to_string())?;
        let browse = daemon.browse(SERVICE_TYPE).map_err(|err| format!("mDNS browse failed: {err}"))?;
        let (sender, events) = mpsc::channel(64);
        let peers = Arc::new(Mutex::new(HashMap::new()));
        let browse_task = tokio::spawn(run_browse(browse, endpoint_id, peers.clone(), sender));
        let record = Record { port, user_data: user_data.map(|data| data.to_string()) };
        let lookup = Self {
            daemon,
            instance: instance_name(&endpoint_id),
            record: Mutex::new(record.clone()),
            peers,
            browse_task,
        };
        lookup.register(&record)?;
        Ok((lookup, events))
    }

    /// Announces the new user data, if it changed.
    pub fn set_user_data(&self, user_data: Option<UserData>) {
        self.update(|record| record.user_data = user_data.map(|data| data.to_string()));
    }

    /// Changes the record and registers it again, if it changed.
    fn update(&self, change: impl FnOnce(&mut Record)) {
        let mut record = self.record.lock().unwrap();
        let mut wanted = record.clone();
        change(&mut wanted);
        if wanted == *record {
            return;
        }
        match self.register(&wanted) {
            Ok(()) => *record = wanted,
            Err(err) => log::warn!("[transport][network] {err}"),
        }
    }

    /// Registers the record. A second registration of the same instance
    /// replaces the first one and announces it again.
    fn register(&self, record: &Record) -> Result<(), String> {
        let mut properties = HashMap::new();
        if let Some(user_data) = &record.user_data {
            properties.insert(USER_DATA_KEY.to_string(), user_data.clone());
        }
        let host = format!("{}.local.", self.instance);
        let info = ServiceInfo::new(SERVICE_TYPE, &self.instance, &host, (), record.port, properties)
            .map_err(|err| format!("the mDNS record is not valid: {err}"))?
            // The daemon gives on each interface the address of that
            // interface, and follows address changes.
            .enable_addr_auto();
        self.daemon.register(info).map_err(|err| format!("mDNS register failed: {err}"))
    }

    fn fullname(&self) -> String {
        format!("{}.{SERVICE_TYPE}", self.instance)
    }
}

impl Drop for MdnsLookup {
    /// Removes this record from the network and stops the daemon.
    fn drop(&mut self) {
        self.browse_task.abort();
        let _ = self.daemon.unregister(&self.fullname());
        let _ = self.daemon.shutdown();
    }
}

impl AddressLookup for MdnsLookup {
    /// iroh publishes the endpoint's data: only the user data goes into
    /// the record. The port is the bound port, and the daemon finds the
    /// addresses of each interface.
    fn publish(&self, data: &EndpointData) {
        let user_data = data.user_data().map(|data| data.to_string());
        self.update(|record| record.user_data = user_data);
    }

    fn resolve(&self, endpoint_id: EndpointId) -> Option<BoxStream<'static, Result<Item, Error>>> {
        let info = self.peers.lock().unwrap().get(&endpoint_id).cloned()?;
        Some(Box::pin(stream::once(async move { Ok(Item::new(info, PROVENANCE, None)) })))
    }
}

/// Reads the browse events of the daemon until the receiver of `sender`
/// closes.
async fn run_browse(
    browse: mdns_sd::Receiver<ServiceEvent>,
    own_id: EndpointId,
    peers: Arc<Mutex<HashMap<EndpointId, EndpointInfo>>>,
    sender: mpsc::Sender<MdnsEvent>,
) {
    let mut repeat = tokio::time::interval(REPEAT_INTERVAL);
    loop {
        let events = tokio::select! {
            event = browse.recv_async() => match event {
                Ok(ServiceEvent::ServiceResolved(service)) => {
                    let Some(info) = endpoint_info(&service).filter(|info| info.endpoint_id != own_id) else {
                        continue;
                    };
                    peers.lock().unwrap().insert(info.endpoint_id, info.clone());
                    vec![MdnsEvent::Discovered(info)]
                }
                Ok(ServiceEvent::ServiceRemoved(_, fullname)) => {
                    let Some(endpoint_id) = endpoint_id_of(&fullname) else {
                        continue;
                    };
                    if peers.lock().unwrap().remove(&endpoint_id).is_none() {
                        continue;
                    }
                    vec![MdnsEvent::Expired(endpoint_id)]
                }
                Ok(_) => continue,
                Err(_) => {
                    log::warn!("[transport][network] the mDNS daemon stopped");
                    return;
                }
            },
            _ = repeat.tick() => peers.lock().unwrap().values().cloned().map(MdnsEvent::Discovered).collect(),
        };
        for event in events {
            if sender.send(event).await.is_err() {
                return;
            }
        }
    }
}

/// The endpoint id in lower-case base32, as `iroh-mdns-address-lookup`
/// writes it.
fn instance_name(endpoint_id: &EndpointId) -> String {
    data_encoding::BASE32_NOPAD.encode(endpoint_id.as_bytes()).to_ascii_lowercase()
}

/// The endpoint id in a full instance name
/// (`<endpoint id>._fini._udp.local.`).
fn endpoint_id_of(fullname: &str) -> Option<EndpointId> {
    let instance = fullname.strip_suffix(SERVICE_TYPE)?.strip_suffix('.')?;
    instance.parse().ok()
}

/// The iroh endpoint info in a resolved record: the IPv4 addresses with
/// the SRV port, other than loopback first, and the user data.
fn endpoint_info(service: &ResolvedService) -> Option<EndpointInfo> {
    let endpoint_id = endpoint_id_of(service.get_fullname())?;
    let mut ips: Vec<Ipv4Addr> = service.get_addresses_v4().into_iter().collect();
    ips.sort_by_key(|ip| (ip.is_loopback(), *ip));
    let addrs = to_socket_addrs(ips, service.get_port());
    if addrs.is_empty() {
        return None;
    }
    let user_data = service.get_property_val_str(USER_DATA_KEY).and_then(|value| value.parse().ok());
    Some(EndpointInfo::new(endpoint_id).with_ip_addrs(addrs).with_user_data(user_data))
}

fn to_socket_addrs(ips: Vec<Ipv4Addr>, port: u16) -> Vec<SocketAddr> {
    ips.into_iter().map(|ip| SocketAddr::new(IpAddr::V4(ip), port)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_instance_name_gives_the_endpoint_id_back() {
        let endpoint_id = iroh::SecretKey::from_bytes(&[7; 32]).public();
        let fullname = format!("{}.{SERVICE_TYPE}", instance_name(&endpoint_id));
        assert_eq!(endpoint_id_of(&fullname), Some(endpoint_id));
        assert_eq!(endpoint_id_of("other._http._tcp.local."), None);
    }
}
