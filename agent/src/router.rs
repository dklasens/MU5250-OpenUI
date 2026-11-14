use std::collections::HashMap;
use std::net::Ipv4Addr;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::handlers::AppState;
use crate::lan::LanSettings;
use crate::ubus;
use crate::validate::validate_ubus_input;

pub fn router_dns_get(_state: &AppState) -> (u16, Value) {
    match ubus::call("zwrt_router.api", "router_get_dns_para", Some("{}")) {
        Ok(data) => {
            // Firmware returns keys with "wan_" prefix (e.g. wan_dns_mode);
            // strip it so iOS client can use clean names (dns_mode, prefer_dns_manual, etc.)
            let mut cleaned = serde_json::Map::new();
            if let Some(obj) = data.as_object() {
                for (k, v) in obj {
                    let key = k.strip_prefix("wan_").unwrap_or(k).to_string();
                    cleaned.insert(key, v.clone());
                }
            }
            // Firmware bug: sometimes returns empty manual DNS values; fill from UCI
            if cleaned.get("dns_mode").and_then(|v| v.as_str()) == Some("manual")
                && cleaned
                    .get("prefer_dns_manual")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .is_empty()
            {
                if let Ok(v) = ubus::uci_get("network.wan.dns") {
                    let mut parts = v.split_whitespace();
                    if let Some(primary) = parts.next() {
                        cleaned.insert(
                            "prefer_dns_manual".into(),
                            Value::String(primary.to_string()),
                        );
                    }
                    if let Some(secondary) = parts.next() {
                        cleaned.insert(
                            "standby_dns_manual".into(),
                            Value::String(secondary.to_string()),
                        );
                    }
                }
            }
            (200, json!({"ok": true, "data": Value::Object(cleaned)}))
        }
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

pub fn router_dns_set(_state: &AppState, body: &[u8]) -> (u16, Value) {
    let parsed: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({"ok": false, "error": "invalid JSON"})),
    };
    if let Err(e) = validate_ubus_input(&parsed) {
        return (400, json!({"ok": false, "error": e}));
    }
    match ubus::call(
        "zwrt_router.api",
        "router_set_wan_dns",
        Some(&parsed.to_string()),
    ) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

/// The LAN settings from one `uci show zwrt_router` dump. Every key must be
/// present: a missing one is an error, exactly as the per-key `uci get` was.
fn lan_settings_from_uci(uci: &HashMap<String, String>) -> Result<LanSettings, String> {
    let read = |key: &str| {
        uci.get(key)
            .cloned()
            .ok_or_else(|| format!("cannot read zwrt_router.{key}"))
    };
    Ok(LanSettings {
        ipaddr: read("network.lan_ipaddr")?,
        netmask: read("dhcp.lan_netmask")?,
        dhcp_enabled: read("dhcp.ignore")? != "1",
        dhcp_start: read("dhcp.zte_start")?,
        dhcp_end: read("dhcp.zte_end")?,
        lease_seconds: read("dhcp.leasetime")?
            .parse::<u32>()
            .map_err(|_| "invalid zwrt_router DHCP lease time".to_string())?,
    })
}

/// Uncached on purpose: the dashboard reads this back after a LAN change.
pub fn router_lan_get(state: &AppState) -> (u16, Value) {
    let result = ubus::try_uci_show("zwrt_router")
        .map_err(|e| format!("cannot read zwrt_router: {e}"))
        .and_then(|uci| lan_settings_from_uci(&uci));

    match result {
        Ok(lan) => (
            200,
            json!({"ok": true, "data": {
                "transition": state.lan.status(),
                "ipaddr": lan.ipaddr,
                "netmask": lan.netmask,
                "dhcp_enabled": lan.dhcp_enabled,
                "dhcp_start": lan.dhcp_start,
                "dhcp_end": lan.dhcp_end,
                "lease_seconds": lan.lease_seconds,
            }}),
        ),
        Err(error) => (503, json!({"ok": false, "error": error})),
    }
}

pub(crate) fn validate_lan_settings(settings: &LanSettings) -> Result<(), String> {
    let ip: Ipv4Addr = settings
        .ipaddr
        .parse()
        .map_err(|_| "invalid LAN IPv4 address")?;
    let mask: Ipv4Addr = settings
        .netmask
        .parse()
        .map_err(|_| "invalid IPv4 netmask")?;
    let start: Ipv4Addr = settings
        .dhcp_start
        .parse()
        .map_err(|_| "invalid DHCP start address")?;
    let end: Ipv4Addr = settings
        .dhcp_end
        .parse()
        .map_err(|_| "invalid DHCP end address")?;

    if !ip.is_private() {
        return Err("LAN address must be a private IPv4 address".into());
    }
    let ip = u32::from(ip);
    let mask = u32::from(mask);
    let start = u32::from(start);
    let end = u32::from(end);
    let inverse = !mask;
    if inverse & inverse.wrapping_add(1) != 0 || mask.count_ones() < 8 || mask.count_ones() > 30 {
        return Err("netmask must be contiguous and between /8 and /30".to_string());
    }
    let network = ip & mask;
    let broadcast = network | inverse;
    if ip == network || ip == broadcast {
        return Err("LAN address must be a usable host address".into());
    }
    if start & mask != network || end & mask != network {
        return Err("DHCP range must be in the LAN subnet".to_string());
    }
    if start >= end || start == network || end == broadcast {
        return Err("DHCP range must contain usable addresses in ascending order".to_string());
    }
    if start <= ip && ip <= end {
        return Err("DHCP range must not include the router address".to_string());
    }
    if !(60..=604_800).contains(&settings.lease_seconds) {
        return Err("lease_seconds must be between 60 and 604800".to_string());
    }
    Ok(())
}

pub fn router_lan_set(state: &AppState, body: &[u8]) -> (u16, Value) {
    let parsed: LanSettings = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => {
            return (
                400,
                json!({"ok": false, "error": format!("invalid LAN settings: {e}")}),
            )
        }
    };
    if let Err(e) = validate_lan_settings(&parsed) {
        return (400, json!({"ok": false, "error": e}));
    }
    match state.lan.begin(parsed) {
        Ok(data) => (202, json!({"ok": true, "data": data})),
        Err(error) => (409, json!({"ok": false, "error": error})),
    }
}

pub fn router_lan_confirm(state: &AppState, body: &[u8]) -> (u16, Value) {
    let parsed: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(_) => return (400, json!({"ok": false, "error": "invalid confirmation"})),
    };
    match state
        .lan
        .confirm(parsed["token"].as_str().unwrap_or_default())
    {
        Ok(()) => (200, json!({"ok": true, "data": {"confirmed": true}})),
        Err(error) => (409, json!({"ok": false, "error": error})),
    }
}

pub fn router_apn_mode_get(_state: &AppState) -> (u16, Value) {
    match ubus::call("zwrt_apn_object", "get_apn_mode", Some("{}")) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

pub fn router_apn_mode_set(_state: &AppState, body: &[u8]) -> (u16, Value) {
    let parsed: ApnMode = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => {
            return (
                400,
                json!({"ok": false, "error": format!("invalid APN mode: {e}")}),
            )
        }
    };
    if parsed.apn_mode > 1 {
        return (
            400,
            json!({"ok": false, "error": "apn_mode must be 0 (automatic) or 1 (manual)"}),
        );
    }
    match ubus::call(
        "zwrt_apn_object",
        "set_apn_mode",
        Some(&json!({"apn_mode": parsed.apn_mode}).to_string()),
    ) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

pub fn router_apn_profiles_get(_state: &AppState) -> (u16, Value) {
    match ubus::call("zwrt_apn_object", "get_manu_apn_list", Some("{}")) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

pub fn router_apn_profiles_add(_state: &AppState, body: &[u8]) -> (u16, Value) {
    let parsed: ManualApn = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => {
            return (
                400,
                json!({"ok": false, "error": format!("invalid APN profile: {e}")}),
            )
        }
    };
    if let Err(e) = parsed.validate() {
        return (400, json!({"ok": false, "error": e}));
    }
    let payload = json!({
        "profilename": parsed.profilename,
        "wanapn": parsed.wanapn,
        "username": parsed.username,
        "password": parsed.password,
        "pdpType": parsed.pdp_type,
        "pppAuthMode": parsed.ppp_auth_mode,
    });
    match ubus::call(
        "zwrt_apn_object",
        "add_manu_apn",
        Some(&payload.to_string()),
    ) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

pub fn router_apn_profiles_delete(_state: &AppState, body: &[u8]) -> (u16, Value) {
    let parsed: ApnProfileId = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => {
            return (
                400,
                json!({"ok": false, "error": format!("invalid profile id: {e}")}),
            )
        }
    };
    if let Err(e) = parsed.validate() {
        return (400, json!({"ok": false, "error": e}));
    }
    match ubus::call("zwrt_apn_object", "get_manu_apn_list", Some("{}")) {
        Ok(data) if apn_profile_is_active(&data, &parsed.profile_id) => {
            return (
                409,
                json!({"ok": false, "error": "cannot delete the active APN profile"}),
            );
        }
        Ok(_) => {}
        Err(e) => {
            return (
                503,
                json!({"ok": false, "error": format!("cannot verify active APN profile: {e}")}),
            )
        }
    }
    let payload = json!({"profileId": parsed.profile_id});
    match ubus::call(
        "zwrt_apn_object",
        "delete_manu_apn",
        Some(&payload.to_string()),
    ) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

pub fn router_apn_profiles_activate(_state: &AppState, body: &[u8]) -> (u16, Value) {
    let parsed: ApnProfileId = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => {
            return (
                400,
                json!({"ok": false, "error": format!("invalid profile id: {e}")}),
            )
        }
    };
    if let Err(e) = parsed.validate() {
        return (400, json!({"ok": false, "error": e}));
    }
    let current_mode = ubus::call("zwrt_apn_object", "get_apn_mode", Some("{}"))
        .ok()
        .and_then(|v| v.get("apn_mode").and_then(Value::as_u64));
    if let Err(e) = ubus::call("zwrt_apn_object", "set_apn_mode", Some(r#"{"apn_mode":1}"#)) {
        return (
            503,
            json!({"ok": false, "error": format!("cannot enable manual APN mode: {e}")}),
        );
    }
    let payload = json!({"profileId": parsed.profile_id});
    match ubus::call(
        "zwrt_apn_object",
        "enable_manu_apn_id",
        Some(&payload.to_string()),
    ) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => {
            if current_mode == Some(0) {
                let _ = ubus::call("zwrt_apn_object", "set_apn_mode", Some(r#"{"apn_mode":0}"#));
            }
            (503, json!({"ok": false, "error": e}))
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApnMode {
    apn_mode: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ManualApn {
    profilename: String,
    wanapn: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
    pdp_type: u8,
    ppp_auth_mode: u8,
}

impl ManualApn {
    fn validate(&self) -> Result<(), String> {
        validate_apn_text("profilename", &self.profilename, 64, false)?;
        validate_apn_text("wanapn", &self.wanapn, 100, false)?;
        validate_apn_text("username", &self.username, 128, true)?;
        validate_apn_text("password", &self.password, 128, true)?;
        if !(1..=3).contains(&self.pdp_type) {
            return Err("pdpType must be 1 (IPv4), 2 (IPv6), or 3 (IPv4v6)".to_string());
        }
        if self.ppp_auth_mode > 3 {
            return Err("pppAuthMode must be between 0 and 3".to_string());
        }
        if self.ppp_auth_mode == 0 && (!self.username.is_empty() || !self.password.is_empty()) {
            return Err("credentials require PAP, CHAP, or PAP/CHAP authentication".to_string());
        }
        Ok(())
    }
}

fn validate_apn_text(
    field: &str,
    value: &str,
    max: usize,
    allow_empty: bool,
) -> Result<(), String> {
    if !allow_empty && value.trim().is_empty() {
        return Err(format!("{field} is required"));
    }
    if value.len() > max || value.chars().any(char::is_control) {
        return Err(format!(
            "{field} is invalid or longer than {max} characters"
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ApnProfileId {
    profile_id: String,
}

impl ApnProfileId {
    fn validate(&self) -> Result<(), String> {
        if self.profile_id.is_empty()
            || self.profile_id.len() > 32
            || !self
                .profile_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err("profileId must be a firmware profile identifier".to_string());
        }
        Ok(())
    }
}

fn apn_profile_is_active(data: &Value, profile_id: &str) -> bool {
    data.get("apnListArray")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|profile| {
            let id_matches = profile
                .get("profileId")
                .map(|id| match id {
                    Value::String(id) => id == profile_id,
                    Value::Number(id) => id.to_string() == profile_id,
                    _ => false,
                })
                .unwrap_or(false);
            let enabled = profile
                .get("isEnable")
                .is_some_and(|enabled| match enabled {
                    Value::Bool(enabled) => *enabled,
                    Value::Number(enabled) => enabled.as_u64() == Some(1),
                    Value::String(enabled) => {
                        enabled == "1" || enabled.eq_ignore_ascii_case("true")
                    }
                    _ => false,
                });
            id_matches && enabled
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_lan() -> LanSettings {
        LanSettings {
            ipaddr: "192.168.0.1".into(),
            netmask: "255.255.255.0".into(),
            dhcp_enabled: true,
            dhcp_start: "192.168.0.2".into(),
            dhcp_end: "192.168.0.253".into(),
            lease_seconds: 86_400,
        }
    }

    fn lan_uci() -> HashMap<String, String> {
        [
            ("network", "zte_router"),
            ("network.lan_ipaddr", "192.168.0.1"),
            ("dhcp", "zte_router"),
            ("dhcp.lan_netmask", "255.255.255.0"),
            ("dhcp.ignore", "0"),
            ("dhcp.zte_start", "192.168.0.2"),
            ("dhcp.zte_end", "192.168.0.253"),
            ("dhcp.leasetime", "86400"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    #[test]
    fn lan_settings_come_from_one_uci_dump() {
        assert_eq!(lan_settings_from_uci(&lan_uci()).unwrap(), valid_lan());
        let mut disabled = lan_uci();
        disabled.insert("dhcp.ignore".into(), "1".into());
        assert!(!lan_settings_from_uci(&disabled).unwrap().dhcp_enabled);
    }

    #[test]
    fn lan_read_fails_instead_of_reporting_empty_values() {
        // A failed `uci show` used to look like an empty config.
        assert!(lan_settings_from_uci(&HashMap::new()).is_err());
        let mut missing = lan_uci();
        missing.remove("dhcp.zte_end");
        assert!(lan_settings_from_uci(&missing)
            .unwrap_err()
            .contains("zte_end"));
        let mut bad_lease = lan_uci();
        bad_lease.insert("dhcp.leasetime".into(), "12h".into());
        assert!(lan_settings_from_uci(&bad_lease).is_err());
    }

    #[test]
    fn lan_validation_matches_stock_full_address_contract() {
        assert!(validate_lan_settings(&valid_lan()).is_ok());
        let mut invalid = valid_lan();
        invalid.dhcp_start = "192.168.1.2".into();
        assert!(validate_lan_settings(&invalid).is_err());
        let mut includes_router = valid_lan();
        includes_router.dhcp_start = "192.168.0.1".into();
        assert!(validate_lan_settings(&includes_router).is_err());
    }

    #[test]
    fn apn_validation_rejects_unknown_firmware_values() {
        let profile = ManualApn {
            profilename: "Test".into(),
            wanapn: "internet".into(),
            username: String::new(),
            password: String::new(),
            pdp_type: 3,
            ppp_auth_mode: 0,
        };
        assert!(profile.validate().is_ok());
        let invalid = ManualApn {
            pdp_type: 4,
            ..profile
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn active_apn_profiles_are_detected_across_firmware_types() {
        let data = json!({"apnListArray": [
            {"profileId": 4, "isEnable": "1"},
            {"profileId": "5", "isEnable": false}
        ]});
        assert!(apn_profile_is_active(&data, "4"));
        assert!(!apn_profile_is_active(&data, "5"));
        assert!(ApnProfileId {
            profile_id: "manu1".into()
        }
        .validate()
        .is_ok());
        assert!(ApnProfileId {
            profile_id: "../../etc".into()
        }
        .validate()
        .is_err());
    }
}
