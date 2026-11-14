use crate::process::BoundedCommand;
use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::cache::Cached;
use crate::handlers::AppState;
use crate::ubus;
use crate::util::MutexExt;

/// Associated stations change as clients join, roam and leave. 5 s keeps the
/// client list and the Wi-Fi client counts current while collapsing the
/// clients poll (15 s per open tab) and `/api/wifi/status` onto one `iw` pair.
const STATION_TTL: Duration = Duration::from_secs(5);
/// The assembled client list (DHCP leases, ARP, stations, bridge FDB).
const CLIENTS_TTL: Duration = Duration::from_secs(5);
/// Live channel and width from `iw <iface> info`.
const IW_INFO_TTL: Duration = Duration::from_secs(5);
/// A wired link's negotiated speed only changes on re-plug or re-enumeration.
/// Applies to the `ethtool` fallback only; sysfs is free and read every time.
const LINK_SPEED_TTL: Duration = Duration::from_secs(60);

/// Live channel and width per Wi-Fi interface, as `wifi::iw_info` reads them.
pub type IwInfo = [(String, String); 2];

/// Short-lived copies of the subprocess-backed reads behind the client list
/// and Wi-Fi status. Mutations that change them invalidate them
/// (`invalidate_wifi`, `invalidate_lan`), so a read-back after a change is fresh.
#[derive(Default)]
pub struct NetworkCache {
    stations: Cached<Arc<Stations>>,
    clients: Cached<Value>,
    iw_info: Cached<IwInfo>,
    link_speed: Mutex<HashMap<String, (Instant, Option<u64>)>>,
}

impl NetworkCache {
    /// Associated stations on both bands, shared by `/api/network/clients`
    /// and `/api/wifi/status`.
    pub fn stations(&self) -> Arc<Stations> {
        self.stations_with(Stations::read)
    }

    fn stations_with(&self, read: impl FnOnce() -> Stations) -> Arc<Stations> {
        self.stations
            .get_or_refresh(STATION_TTL, || Arc::new(read()))
    }

    pub fn iw_info(&self, read: impl FnOnce() -> IwInfo) -> IwInfo {
        self.iw_info.get_or_refresh(IW_INFO_TTL, read)
    }

    /// After a Wi-Fi change: `zwrt_wlan reload` restarts the radios, so the
    /// live channel, the stations and the client list are all wrong.
    pub fn invalidate_wifi(&self) {
        self.stations.invalidate();
        self.iw_info.invalidate();
        self.clients.invalidate();
    }

    /// After a LAN change or rollback: addresses and leases move.
    pub fn invalidate_lan(&self) {
        self.clients.invalidate();
    }

    fn link_speed_mbps(&self, iface: &str) -> Option<u64> {
        if let Some(speed) = sysfs_link_speed(iface) {
            return Some(speed);
        }
        if let Some((at, speed)) = self.link_speed.safe_lock().get(iface) {
            if at.elapsed() < LINK_SPEED_TTL {
                return *speed;
            }
        }
        // Runs inside the single-flight client refresh, so two callers never
        // fork `ethtool` for the same interface at once.
        let speed = ethtool_link_speed(iface);
        let mut memo = self.link_speed.safe_lock();
        memo.retain(|_, (at, _)| at.elapsed() < LINK_SPEED_TTL);
        memo.insert(iface.to_string(), (Instant::now(), speed));
        speed
    }
}

fn parse_first_float(input: &str) -> Option<f64> {
    let mut buf = String::new();
    let mut seen_digit = false;
    for ch in input.chars() {
        if ch.is_ascii_digit() || ch == '.' || (!seen_digit && ch == '-') {
            buf.push(ch);
            if ch.is_ascii_digit() {
                seen_digit = true;
            }
        } else if seen_digit {
            break;
        }
    }
    (!buf.is_empty()).then(|| buf.parse().ok()).flatten()
}

fn is_unicast_mac(mac: &str) -> bool {
    let parts: Vec<&str> = mac.split(':').collect();
    if parts.len() != 6 {
        return false;
    }
    let first = match u8::from_str_radix(parts[0], 16) {
        Ok(v) => v,
        Err(_) => return false,
    };
    first & 1 == 0
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WifiStationInfo {
    band: String,
    signal_dbm: Option<i64>,
    tx_bitrate_mbps: Option<f64>,
    rx_bitrate_mbps: Option<f64>,
    expected_throughput_mbps: Option<f64>,
    connected_secs: Option<u64>,
}

/// Stations associated on each band, keyed by lower-case MAC.
#[derive(Default)]
pub struct Stations {
    pub band_2g: HashMap<String, WifiStationInfo>,
    pub band_5g: HashMap<String, WifiStationInfo>,
}

impl Stations {
    /// One `iw <iface> station dump` per band. An unreadable band is empty.
    fn read() -> Self {
        let dump = |iface: &str| {
            Command::new("iw")
                .args([iface, "station", "dump"])
                .bounded_output()
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default()
        };
        Self {
            band_2g: parse_station_dump(&dump("wlan0"), "2.4 GHz"),
            band_5g: parse_station_dump(&dump("wlan2"), "5 GHz"),
        }
    }

    fn get(&self, mac: &str) -> Option<&WifiStationInfo> {
        self.band_2g.get(mac).or_else(|| self.band_5g.get(mac))
    }
}

fn parse_station_dump(output: &str, band: &str) -> HashMap<String, WifiStationInfo> {
    let mut stations = HashMap::new();
    let mut current_mac = String::new();
    let mut current = WifiStationInfo {
        band: band.to_string(),
        ..WifiStationInfo::default()
    };

    let flush = |stations: &mut HashMap<String, WifiStationInfo>,
                 current_mac: &mut String,
                 current: &mut WifiStationInfo| {
        if !current_mac.is_empty() {
            stations.insert(current_mac.clone(), current.clone());
        }
    };

    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Station ") {
            flush(&mut stations, &mut current_mac, &mut current);
            current_mac = rest
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_lowercase();
            current = WifiStationInfo {
                band: band.to_string(),
                ..WifiStationInfo::default()
            };
            continue;
        }

        if let Some(value) = trimmed.strip_prefix("signal:") {
            current.signal_dbm = parse_first_float(value).map(|v| v.round() as i64);
        } else if let Some(value) = trimmed.strip_prefix("tx bitrate:") {
            current.tx_bitrate_mbps = parse_first_float(value);
        } else if let Some(value) = trimmed.strip_prefix("rx bitrate:") {
            current.rx_bitrate_mbps = parse_first_float(value);
        } else if let Some(value) = trimmed.strip_prefix("expected throughput:") {
            current.expected_throughput_mbps = parse_first_float(value);
        } else if let Some(value) = trimmed.strip_prefix("connected time:") {
            current.connected_secs = parse_first_float(value).map(|v| v.max(0.0) as u64);
        }
    }

    flush(&mut stations, &mut current_mac, &mut current);
    stations
}

fn bridge_port_by_mac() -> HashMap<String, String> {
    let output = match Command::new("bridge")
        .args(["fdb", "show", "br", "br-lan"])
        .bounded_output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => return HashMap::new(),
    };
    parse_bridge_fdb(&output)
}

fn parse_bridge_fdb(output: &str) -> HashMap<String, String> {
    let mut ports = HashMap::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.contains(" self ") || trimmed.contains(" permanent") {
            continue;
        }
        let tokens: Vec<&str> = trimmed.split_whitespace().collect();
        if tokens.len() < 4 {
            continue;
        }
        let mac = tokens[0].to_lowercase();
        if !is_unicast_mac(&mac) {
            continue;
        }
        let dev = tokens
            .windows(2)
            .find_map(|w| (w[0] == "dev").then_some(w[1]))
            .unwrap_or_default();
        if !dev.is_empty() {
            ports.insert(mac, dev.to_string());
        }
    }
    ports
}

fn arp_entries() -> Vec<(String, String)> {
    std::fs::read_to_string("/proc/net/arp")
        .map(|arp| parse_arp(&arp))
        .unwrap_or_default()
}

fn parse_arp(arp: &str) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    for line in arp.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 6 {
            continue;
        }
        let ip = cols[0];
        let flags = cols[2];
        let mac = cols[3].to_lowercase();
        if flags == "0x0" || mac == "00:00:00:00:00:00" || !is_unicast_mac(&mac) {
            continue;
        }
        entries.push((mac, ip.to_string()));
    }
    entries
}

fn medium_for_port(
    port: Option<&str>,
    wifi: Option<&WifiStationInfo>,
) -> (&'static str, Option<&'static str>) {
    if let Some(wifi) = wifi {
        return (
            "wifi",
            Some(if wifi.band == "5 GHz" {
                "wifi_5ghz"
            } else {
                "wifi_2ghz"
            }),
        );
    }
    match port.unwrap_or_default() {
        p if p.starts_with("ecm") || p.starts_with("ncm") || p.starts_with("usb") => {
            ("usb-c", Some("usb_c"))
        }
        p if p.starts_with("eth") => ("ethernet", Some("ethernet")),
        _ => ("wired", None),
    }
}

fn sysfs_link_speed(iface: &str) -> Option<u64> {
    let raw = std::fs::read_to_string(format!("/sys/class/net/{iface}/speed")).ok()?;
    let speed = raw.trim().parse::<i64>().ok()?;
    (speed > 0).then_some(speed as u64)
}

fn ethtool_link_speed(iface: &str) -> Option<u64> {
    let output = Command::new("ethtool").arg(iface).bounded_output().ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().find_map(|line| {
        let trimmed = line.trim();
        let value = trimmed.strip_prefix("Speed:")?.trim();
        value.strip_suffix("Mb/s")?.trim().parse::<u64>().ok()
    })
}

fn hostnames_from_leases(dhcp: &Value) -> HashMap<String, String> {
    let mut hostname_by_mac = HashMap::new();
    if let Some(leases) = dhcp.get("dhcp_leases").and_then(|v| v.as_array()) {
        for l in leases {
            if let (Some(mac), Some(host)) = (l["macaddr"].as_str(), l["hostname"].as_str()) {
                if !host.is_empty() {
                    hostname_by_mac.insert(mac.to_lowercase(), host.to_string());
                }
            }
        }
    }
    hostname_by_mac
}

/// Join the per-source readings into the client list, one entry per MAC.
fn assemble_clients(
    hostname_by_mac: &HashMap<String, String>,
    arp: Vec<(String, String)>,
    stations: &Stations,
    bridge_ports: &HashMap<String, String>,
    mut link_speed: impl FnMut(&str) -> Option<u64>,
) -> Vec<Value> {
    let ip_by_mac: HashMap<String, String> = arp.into_iter().collect();

    let mut all_macs: HashSet<&String> = HashSet::new();
    all_macs.extend(ip_by_mac.keys());
    all_macs.extend(stations.band_2g.keys());
    all_macs.extend(stations.band_5g.keys());
    all_macs.extend(bridge_ports.keys());
    let mut sorted_macs: Vec<&String> = all_macs.into_iter().collect();
    sorted_macs.sort();

    sorted_macs
        .into_iter()
        .map(|mac| {
            let wifi_info = stations.get(mac);
            let port = bridge_ports.get(mac).map(String::as_str);
            let (medium, medium_detail) = medium_for_port(port, wifi_info);
            let wired_link_mbps = match port {
                Some(iface) if medium != "wifi" => link_speed(iface),
                _ => None,
            };
            json!({
                "mac": mac,
                "ip": ip_by_mac.get(mac).cloned(),
                "hostname": hostname_by_mac.get(mac).cloned(),
                "medium": medium,
                "medium_detail": medium_detail,
                "interface": port,
                "wifi_band": wifi_info.map(|w| w.band.clone()),
                "signal_dbm": wifi_info.and_then(|w| w.signal_dbm),
                "tx_bitrate_mbps": wifi_info.and_then(|w| w.tx_bitrate_mbps),
                "rx_bitrate_mbps": wifi_info.and_then(|w| w.rx_bitrate_mbps),
                "expected_throughput_mbps": wifi_info.and_then(|w| w.expected_throughput_mbps),
                "connected_secs": wifi_info.and_then(|w| w.connected_secs),
                "wired_link_mbps": wired_link_mbps,
            })
        })
        .collect()
}

fn read_clients(net: &NetworkCache) -> Value {
    // Hostnames from DHCP leases. A failed read just leaves them unnamed.
    let hostname_by_mac = ubus::call_poll("luci-rpc", "getDHCPLeases", Some(r#"{"family":4}"#))
        .map(|dhcp| hostnames_from_leases(&dhcp))
        .unwrap_or_default();
    let stations = net.stations();
    let clients = assemble_clients(
        &hostname_by_mac,
        arp_entries(),
        &stations,
        &bridge_port_by_mac(),
        |iface| net.link_speed_mbps(iface),
    );
    Value::Array(clients)
}

pub fn network_clients(state: &AppState) -> (u16, Value) {
    let clients = state
        .net
        .clients
        .get_or_refresh(CLIENTS_TTL, || read_clients(&state.net));
    (200, json!({"ok": true, "data": { "clients": clients }}))
}

pub fn network_battery_ubus(_state: &AppState) -> (u16, Value) {
    match ubus::call("zwrt_bsp.battery", "list", Some("{}")) {
        Ok(data) => {
            let number = |key: &str| -> Option<i64> {
                match data.get(key)? {
                    Value::Number(value) => value.as_i64(),
                    Value::String(value) => value.parse().ok(),
                    _ => None,
                }
                .filter(|value| *value >= 0)
            };
            let online = number("battery_online").map(|value| value == 1);
            let low_power = number("battery_low_power").map(|value| value == 1);
            let using_hw_fg_chip = number("battery_using_hw_fg_chip").map(|value| value == 1);
            let time_to_full_mins = number("battery_time_to_full");
            let time_to_empty_mins = number("battery_time_to_empty");
            let available = online.is_some()
                || low_power.is_some()
                || using_hw_fg_chip.is_some()
                || time_to_full_mins.is_some()
                || time_to_empty_mins.is_some();
            (
                200,
                json!({"ok": true, "data": {
                    "available": available,
                    "online": online,
                    "low_power": low_power,
                    "using_hw_fg_chip": using_hw_fg_chip,
                    "time_to_full_mins": time_to_full_mins,
                    "time_to_empty_mins": time_to_empty_mins,
                }}),
            )
        }
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    // Upstream `iw` (nl80211) layout: a `Station` header, tab-indented fields.
    const STATION_DUMP: &str = "\
Station 3C:22:FB:12:34:56 (on wlan2)
\tinactive time:\t120 ms
\trx bytes:\t1048576
\ttx bytes:\t2097152
\tsignal:  \t-52 [-54, -55] dBm
\tsignal avg:\t-53 dBm
\ttx bitrate:\t1200.9 MBit/s 80MHz HE-MCS 11 HE-NSS 2 HE-GI 0 HE-DCM 0
\trx bitrate:\t864.8 MBit/s 80MHz HE-MCS 8 HE-NSS 2 HE-GI 0 HE-DCM 0
\texpected throughput:\t550.781Mbps
\tauthorized:\tyes
\tconnected time:\t3605 seconds
Station a4:83:e7:aa:bb:cc (on wlan2)
\tsignal:  \t-71 dBm
\ttx bitrate:\t6.0 MBit/s
\tconnected time:\t12 seconds
";

    #[test]
    fn parses_station_dump_per_station() {
        let stations = parse_station_dump(STATION_DUMP, "5 GHz");
        assert_eq!(stations.len(), 2);
        let first = &stations["3c:22:fb:12:34:56"];
        assert_eq!(first.band, "5 GHz");
        assert_eq!(first.signal_dbm, Some(-52));
        assert_eq!(first.tx_bitrate_mbps, Some(1200.9));
        assert_eq!(first.rx_bitrate_mbps, Some(864.8));
        assert_eq!(first.expected_throughput_mbps, Some(550.781));
        assert_eq!(first.connected_secs, Some(3605));
        let second = &stations["a4:83:e7:aa:bb:cc"];
        assert_eq!(second.signal_dbm, Some(-71));
        assert_eq!(second.rx_bitrate_mbps, None);
        assert!(parse_station_dump("", "2.4 GHz").is_empty());
    }

    #[test]
    fn bridge_fdb_keeps_learned_unicast_entries() {
        // iproute2 `bridge fdb show br br-lan`.
        let fdb = "\
33:33:00:00:00:01 dev ecm0 self permanent
01:00:5e:00:00:01 dev wlan0 self permanent
f2:11:22:33:44:55 dev ecm0 master br-lan
3c:22:fb:12:34:56 dev wlan2 master br-lan
de:ad:be:ef:00:01 dev ecm0 vlan 1 master br-lan permanent
01:00:5e:7f:ff:fa dev wlan0 master br-lan
";
        let ports = parse_bridge_fdb(fdb);
        assert_eq!(ports.len(), 2, "{ports:?}");
        assert_eq!(ports["f2:11:22:33:44:55"], "ecm0");
        assert_eq!(ports["3c:22:fb:12:34:56"], "wlan2");
    }

    #[test]
    fn arp_skips_incomplete_entries() {
        let arp = "\
IP address       HW type     Flags       HW address            Mask     Device
192.168.0.23     0x1         0x2         F2:11:22:33:44:55     *        br-lan
192.168.0.40     0x1         0x0         00:00:00:00:00:00     *        br-lan
192.168.0.41     0x1         0x2         00:00:00:00:00:00     *        br-lan
192.168.0.42     0x1         0x2         3c:22:fb:12:34:56     *        br-lan
";
        assert_eq!(
            parse_arp(arp),
            [
                ("f2:11:22:33:44:55".to_string(), "192.168.0.23".to_string()),
                ("3c:22:fb:12:34:56".to_string(), "192.168.0.42".to_string()),
            ]
        );
    }

    #[test]
    fn clients_join_sources_and_keep_their_shape() {
        let stations = Stations {
            band_2g: HashMap::new(),
            band_5g: parse_station_dump(STATION_DUMP, "5 GHz"),
        };
        let hostnames = HashMap::from([("f2:11:22:33:44:55".to_string(), "laptop".to_string())]);
        let ports = HashMap::from([
            ("f2:11:22:33:44:55".to_string(), "ecm0".to_string()),
            ("3c:22:fb:12:34:56".to_string(), "wlan2".to_string()),
        ]);
        let arp = vec![("f2:11:22:33:44:55".to_string(), "192.168.0.23".to_string())];
        let mut speed_lookups = Vec::new();
        let clients = assemble_clients(&hostnames, arp, &stations, &ports, |iface| {
            speed_lookups.push(iface.to_string());
            Some(5000)
        });
        assert_eq!(clients.len(), 3);
        // Link speed is only looked up for wired clients.
        assert_eq!(speed_lookups, ["ecm0"]);
        let wifi = &clients[0];
        assert_eq!(wifi["mac"], "3c:22:fb:12:34:56");
        assert_eq!(wifi["medium"], "wifi");
        assert_eq!(wifi["medium_detail"], "wifi_5ghz");
        assert_eq!(wifi["wired_link_mbps"], Value::Null);
        let usb = &clients[2];
        assert_eq!(usb["medium_detail"], "usb_c");
        assert_eq!(usb["hostname"], "laptop");
        assert_eq!(usb["ip"], "192.168.0.23");
        assert_eq!(usb["wired_link_mbps"], 5000);
        let mut keys: Vec<&str> = usb
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "connected_secs",
                "expected_throughput_mbps",
                "hostname",
                "interface",
                "ip",
                "mac",
                "medium",
                "medium_detail",
                "rx_bitrate_mbps",
                "signal_dbm",
                "tx_bitrate_mbps",
                "wifi_band",
                "wired_link_mbps",
            ]
        );
    }

    #[test]
    fn station_dump_is_shared_until_a_wifi_change() {
        let net = NetworkCache::default();
        let reads = AtomicU32::new(0);
        let read = || {
            reads.fetch_add(1, Ordering::Relaxed);
            Stations {
                band_2g: parse_station_dump(STATION_DUMP, "2.4 GHz"),
                band_5g: HashMap::new(),
            }
        };
        // Clients list and Wi-Fi status within one TTL: one `iw` pair.
        assert_eq!(net.stations_with(read).band_2g.len(), 2);
        assert_eq!(net.stations_with(read).band_2g.len(), 2);
        assert_eq!(reads.load(Ordering::Relaxed), 1);
        net.invalidate_wifi();
        net.stations_with(read);
        assert_eq!(reads.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn mutations_invalidate_the_reads_they_change() {
        let net = NetworkCache::default();
        let ttl = Duration::from_secs(60);
        net.clients.get_or_refresh(ttl, || json!(["before"]));
        net.iw_info(|| [("1".into(), "20MHz".into()), ("36".into(), "80MHz".into())]);
        net.invalidate_lan();
        assert_eq!(
            net.clients.get_or_refresh(ttl, || json!(["after lan"])),
            json!(["after lan"])
        );
        net.invalidate_wifi();
        assert_eq!(
            net.clients.get_or_refresh(ttl, || json!(["after wifi"])),
            json!(["after wifi"])
        );
        assert_eq!(
            net.iw_info(|| [
                ("6".into(), "40MHz".into()),
                ("149".into(), "160MHz".into())
            ])[1]
                .0,
            "149"
        );
    }

    #[test]
    fn ethtool_fallback_is_memoised_per_interface() {
        let net = NetworkCache::default();
        net.link_speed
            .safe_lock()
            .insert("ecm-test0".into(), (Instant::now(), Some(480)));
        // No sysfs node for this name, and the memo answers without ethtool.
        assert_eq!(net.link_speed_mbps("ecm-test0"), Some(480));
    }
}
