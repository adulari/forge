// Nearby hosts: browse `_forge._tcp` (advertised by `forge serve --lan`) so the pairing screen can
// list daemons on the local network without a URL being typed. The TXT record carries only a
// host id and the certificate fingerprint, but anyone on the network can publish that record, so
// discovery suggests an address only: the fingerprint is deliberately not read, and a pin must
// come from the pairing link or from a fingerprint the user compares by eye. The pairing token is
// never advertised, so choosing a host still needs a link, a scan, or a one-time code.
use serde::Serialize;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const SERVICE: &str = "_forge._tcp.local.";

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct NearbyHost {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
}

/// Home and office LANs are 192.168/16 or 10/8; 172.16/12 is mostly container bridges.
fn route_rank(address: &std::net::Ipv4Addr) -> u8 {
    match address.octets() {
        [192, 168, ..] => 0,
        [10, ..] => 1,
        _ => 2,
    }
}

pub fn browse(window: Duration) -> Result<Vec<NearbyHost>, String> {
    let daemon = mdns_sd::ServiceDaemon::new().map_err(|e| e.to_string())?;
    let receiver = daemon.browse(SERVICE).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + window;
    let mut found: BTreeMap<String, NearbyHost> = BTreeMap::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match receiver.recv_timeout(left) {
            Ok(mdns_sd::ServiceEvent::ServiceResolved(info)) => {
                let addresses = info.get_addresses_v4();
                let Some(address) = addresses
                    .iter()
                    .copied()
                    .filter(|a| !a.is_loopback() && !a.is_link_local())
                    .min_by_key(|a| (route_rank(a), *a))
                    .or_else(|| addresses.iter().copied().min())
                else {
                    continue;
                };
                let id = info
                    .get_property_val_str("id")
                    .unwrap_or_default()
                    .to_string();
                let name = if id.is_empty() {
                    info.get_hostname().trim_end_matches('.').to_string()
                } else {
                    id.clone()
                };
                found.insert(
                    info.get_fullname().to_string(),
                    NearbyHost {
                        id,
                        name,
                        host: address.to_string(),
                        port: info.get_port(),
                    },
                );
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = daemon.shutdown();
    Ok(found.into_values().collect())
}

#[tauri::command]
pub async fn nearby_hosts(window_ms: Option<u64>) -> Result<Vec<NearbyHost>, String> {
    let window = Duration::from_millis(window_ms.unwrap_or(1500).clamp(300, 5000));
    tauri::async_runtime::spawn_blocking(move || browse(window))
        .await
        .map_err(|e| e.to_string())?
}
