use std::collections::{HashMap, HashSet};
use std::process::Command;

use serde_json::{json, Value};

use crate::handlers::AppState;
use crate::ubus;

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

#[derive(Clone, Default)]
struct WifiStationInfo {
    band: String,
    signal_dbm: Option<i64>,
    tx_bitrate_mbps: Option<f64>,
    rx_bitrate_mbps: Option<f64>,
    expected_throughput_mbps: Option<f64>,
    connected_secs: Option<u64>,
}

fn parse_station_dump(iface: &str, band: &str) -> HashMap<String, WifiStationInfo> {
    let output = match Command::new("iw").args([iface, "station", "dump"]).output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => return HashMap::new(),
    };

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
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => return HashMap::new(),
    };

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
    let mut entries = Vec::new();
    let arp = match std::fs::read_to_string("/proc/net/arp") {
        Ok(v) => v,
        Err(_) => return entries,
    };

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

fn link_speed_mbps(iface: &str) -> Option<u64> {
    let path = format!("/sys/class/net/{iface}/speed");
    if let Ok(raw) = std::fs::read_to_string(path) {
        if let Ok(speed) = raw.trim().parse::<i64>() {
            if speed > 0 {
                return Some(speed as u64);
            }
        }
    }

    let output = Command::new("ethtool").arg(iface).output().ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().find_map(|line| {
        let trimmed = line.trim();
        let value = trimmed.strip_prefix("Speed:")?.trim();
        value.strip_suffix("Mb/s")?.trim().parse::<u64>().ok()
    })
}

pub fn network_wan(_state: &AppState) -> (u16, Value) {
    match ubus::call("network.interface.zte_wan", "status", Some("{}")) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

pub fn network_wan6(_state: &AppState) -> (u16, Value) {
    match ubus::call("network.interface.zte_wan6", "status", Some("{}")) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

pub fn network_lan_status(_state: &AppState) -> (u16, Value) {
    match ubus::call("network.interface.lan", "status", Some("{}")) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

pub fn network_clients(_state: &AppState) -> (u16, Value) {
    // Build hostname lookup from DHCP leases
    let mut hostname_by_mac: HashMap<String, String> = HashMap::new();
    if let Ok(dhcp) = ubus::call("luci-rpc", "getDHCPLeases", Some(r#"{"family":4}"#)) {
        if let Some(leases) = dhcp.get("dhcp_leases").and_then(|v| v.as_array()) {
            for l in leases {
                if let (Some(mac), Some(host)) = (l["macaddr"].as_str(), l["hostname"].as_str()) {
                    if !host.is_empty() {
                        hostname_by_mac.insert(mac.to_lowercase(), host.to_string());
                    }
                }
            }
        }
    }

    let arp = arp_entries();
    let ip_by_mac: HashMap<String, String> = arp.iter().cloned().collect();
    let wifi_2g = parse_station_dump("wlan0", "2.4 GHz");
    let wifi_5g = parse_station_dump("wlan2", "5 GHz");
    let bridge_ports = bridge_port_by_mac();

    let mut all_macs: HashSet<String> = HashSet::new();
    all_macs.extend(ip_by_mac.keys().cloned());
    all_macs.extend(wifi_2g.keys().cloned());
    all_macs.extend(wifi_5g.keys().cloned());
    all_macs.extend(bridge_ports.keys().cloned());

    let mut clients = Vec::new();
    let mut sorted_macs: Vec<String> = all_macs.into_iter().collect();
    sorted_macs.sort();

    for mac in sorted_macs {
        let wifi_info = wifi_2g.get(&mac).or_else(|| wifi_5g.get(&mac));
        let port = bridge_ports.get(&mac).map(String::as_str);
        let (medium, medium_detail) = medium_for_port(port, wifi_info);
        let wired_link_mbps = match port {
            Some(iface) if medium != "wifi" => link_speed_mbps(iface),
            _ => None,
        };

        clients.push(json!({
            "mac": mac,
            "ip": ip_by_mac.get(&mac).cloned(),
            "hostname": hostname_by_mac.get(&mac).cloned(),
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
        }));
    }

    (200, json!({"ok": true, "data": { "clients": clients }}))
}

pub fn network_speeds(_state: &AppState) -> (u16, Value) {
    match ubus::call("zwrt_data", "get_wwandst", Some(r#"{"cid":1,"type":1}"#)) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

pub fn network_rmnet(_state: &AppState) -> (u16, Value) {
    match ubus::call(
        "network.device",
        "status",
        Some(r#"{"name":"rmnet_data0"}"#),
    ) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

pub fn network_battery_ubus(_state: &AppState) -> (u16, Value) {
    match ubus::call("zwrt_bsp.battery", "list", Some("{}")) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}
