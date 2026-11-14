use crate::process::BoundedCommand;
use std::collections::HashMap;
use std::process::Command;

use serde_json::{json, Value};

use crate::handlers::AppState;
use crate::ubus;
use crate::uci_transaction::{self, Change};
use crate::util::MutexExt;

const WIFI_ONOFF_KEY: &str = "wifi_onoff";
const WIFI6_SWITCH_KEY: &str = "wifi6_switch";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The whole `wireless` and `zte_mbb` configs, dumped once per request.
struct WifiConfig {
    wireless: HashMap<String, String>,
    mbb: HashMap<String, String>,
}

impl WifiConfig {
    fn load() -> Self {
        Self {
            wireless: ubus::uci_show("wireless"),
            mbb: ubus::uci_show("zte_mbb"),
        }
    }

    fn get(&self, key: &str) -> String {
        self.wireless.get(key).cloned().unwrap_or_default()
    }

    /// Newer firmware (CN B27+) keeps the global Wi-Fi switches in a separate
    /// `zte_mbb` UCI config instead of `wireless.zte_mbb`. Read both namespaces.
    fn feature(&self, key: &str) -> String {
        if let Some(v) = self.mbb.get(&format!("wifi.{key}")) {
            if !v.is_empty() {
                return v.clone();
            }
        }
        self.get(&format!("zte_mbb.{key}"))
    }
}

fn report_value(report: Option<&Value>, key: &str) -> String {
    report
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

fn iw_info(iface: &str) -> (String, String) {
    let output = Command::new("iw")
        .args([iface, "info"])
        .bounded_output()
        .ok();
    let out = output
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let channel = out
        .lines()
        .find_map(|l| {
            let l = l.trim();
            if l.starts_with("channel ") {
                l.split_whitespace().nth(1).map(|s| s.to_string())
            } else {
                None
            }
        })
        .unwrap_or_default();
    let bw = out
        .lines()
        .find_map(|l| {
            let l = l.trim();
            if let Some(pos) = l.find("width:") {
                let rest = l[pos + 6..].trim();
                let end = rest.find("MHz").map(|i| i + 3).unwrap_or(rest.len());
                Some(rest[..end].trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_default();
    (channel, bw)
}

fn bandwidth_options(hwmode: &str, standards: &str, is_5g: bool) -> Vec<String> {
    let mode = hwmode.to_ascii_lowercase();
    let supported: Vec<String> = standards
        .split(',')
        .map(|value| value.trim().to_ascii_lowercase())
        .collect();
    let has = |standard: &str| supported.iter().any(|value| value == standard);
    let prefix = if mode.contains("be") || has("be") {
        "EHT"
    } else if mode.contains("ax") || has("ax") {
        "HE"
    } else if is_5g && (mode.contains("ac") || has("ac")) {
        "VHT"
    } else {
        "HT"
    };
    let widths: &[u16] = if is_5g { &[20, 40, 80, 160] } else { &[20, 40] };
    widths
        .iter()
        .map(|width| format!("{prefix}{width}"))
        .collect()
}

// ---------------------------------------------------------------------------
// GET /api/wifi/status
// ---------------------------------------------------------------------------

/// The configuration (`zwrt_wlan report`, `uci show`) is read fresh on every
/// call: the dashboard verifies a change by reading it back. The runtime parts
/// (`iw info`, the station dump shared with the client list) sit behind short
/// TTLs that `wifi_set` invalidates.
pub fn wifi_status(state: &AppState) -> (u16, Value) {
    let mut result = serde_json::Map::new();
    let report = ubus::call("zwrt_wlan", "report", Some("{}")).ok();
    let cfg = WifiConfig::load();

    // Global switches from wireless feature config (both UCI namespaces),
    // with report fallback when exposed there.
    let mut wifi_onoff = cfg.feature(WIFI_ONOFF_KEY);
    if wifi_onoff.is_empty() {
        wifi_onoff = report_value(report.as_ref(), WIFI_ONOFF_KEY);
    }
    let wifi_onoff_supported = !wifi_onoff.is_empty();
    if wifi_onoff_supported {
        result.insert("wifi_onoff".into(), json!(wifi_onoff));
    }
    result.insert("wifi_onoff_supported".into(), json!(wifi_onoff_supported));

    let mut wifi6_switch = cfg.feature(WIFI6_SWITCH_KEY);
    if wifi6_switch.is_empty() {
        wifi6_switch = report_value(report.as_ref(), WIFI6_SWITCH_KEY);
    }
    let wifi6_supported = !wifi6_switch.is_empty();
    if wifi6_supported {
        result.insert("wifi6_switch".into(), json!(wifi6_switch));
    }
    result.insert("wifi6_supported".into(), json!(wifi6_supported));

    // Radio config
    result.insert("radio2_disabled".into(), json!(cfg.get("wifi0.disabled")));
    result.insert("radio5_disabled".into(), json!(cfg.get("wifi1.disabled")));
    result.insert("channel_2g".into(), json!(cfg.get("wifi0.channel")));
    result.insert("channel_5g".into(), json!(cfg.get("wifi1.channel")));
    result.insert("txpower_2g".into(), json!(cfg.get("wifi0.txpowerpercent")));
    result.insert("txpower_5g".into(), json!(cfg.get("wifi1.txpowerpercent")));
    result.insert("htmode_2g".into(), json!(cfg.get("wifi0.htmode")));
    result.insert("htmode_5g".into(), json!(cfg.get("wifi1.htmode")));
    let hwmode_2g = cfg.get("wifi0.hwmode");
    let hwmode_5g = cfg.get("wifi1.hwmode");
    let standards_2g = cfg.get("wifi0.SupportedStandards");
    let standards_5g = cfg.get("wifi1.SupportedStandards");
    result.insert("hwmode_2g".into(), json!(hwmode_2g));
    result.insert("hwmode_5g".into(), json!(hwmode_5g));
    result.insert("supported_standards_2g".into(), json!(standards_2g));
    result.insert("supported_standards_5g".into(), json!(standards_5g));
    result.insert(
        "bandwidth_options_2g".into(),
        json!(bandwidth_options(&hwmode_2g, &standards_2g, false)),
    );
    result.insert(
        "bandwidth_options_5g".into(),
        json!(bandwidth_options(&hwmode_5g, &standards_5g, true)),
    );
    result.insert(
        "wifi7_supported".into(),
        json!(
            standards_2g.split(',').any(|s| s.trim() == "be")
                || standards_5g.split(',').any(|s| s.trim() == "be")
        ),
    );
    result.insert("country_code".into(), json!(cfg.get("wifi0.country")));

    // Interface config
    result.insert("ssid_2g".into(), json!(cfg.get("main_2g.ssid")));
    result.insert("ssid_5g".into(), json!(cfg.get("main_5g.ssid")));
    result.insert("key_2g".into(), json!(cfg.get("main_2g.key")));
    result.insert("key_5g".into(), json!(cfg.get("main_5g.key")));
    result.insert(
        "has_key_2g".into(),
        json!(!cfg.get("main_2g.key").is_empty()),
    );
    result.insert(
        "has_key_5g".into(),
        json!(!cfg.get("main_5g.key").is_empty()),
    );
    result.insert("encryption_2g".into(), json!(cfg.get("main_2g.encryption")));
    result.insert("encryption_5g".into(), json!(cfg.get("main_5g.encryption")));
    result.insert("hidden_2g".into(), json!(cfg.get("main_2g.hidden")));
    result.insert("hidden_5g".into(), json!(cfg.get("main_5g.hidden")));

    // Runtime info from iw
    let [(ch2, bw2), (ch5, bw5)] = state.net.iw_info(|| [iw_info("wlan0"), iw_info("wlan2")]);
    result.insert("actual_channel_2g".into(), json!(ch2));
    result.insert("actual_bw_2g".into(), json!(bw2));
    result.insert("actual_channel_5g".into(), json!(ch5));
    result.insert("actual_bw_5g".into(), json!(bw5));

    // Client counts, from the station dump the client list also uses.
    let stations = state.net.stations();
    let c2g = stations.band_2g.len() as u64;
    let c5g = stations.band_5g.len() as u64;
    result.insert("clients_2g".into(), json!(c2g));
    result.insert("clients_5g".into(), json!(c5g));
    result.insert("clients_total".into(), json!(c2g + c5g));

    // Guest WiFi summary
    result.insert(
        "guest_disabled_2g".into(),
        json!(cfg.get("guest_2g.disabled")),
    );
    result.insert(
        "guest_disabled_5g".into(),
        json!(cfg.get("guest_5g.disabled")),
    );
    result.insert("guest_ssid".into(), json!(cfg.get("guest_2g.ssid")));

    (200, json!({"ok": true, "data": result}))
}

// ---------------------------------------------------------------------------
// PUT /api/wifi/settings
// ---------------------------------------------------------------------------

const WIFI_FIELDS: &[(&str, &str)] = &[
    ("ssid_2g", "wireless.main_2g.ssid"),
    ("ssid_5g", "wireless.main_5g.ssid"),
    ("key_2g", "wireless.main_2g.key"), // gitleaks:allow -- UCI option path, not a credential.
    ("key_5g", "wireless.main_5g.key"), // gitleaks:allow -- UCI option path, not a credential.
    ("encryption_2g", "wireless.main_2g.encryption"),
    ("encryption_5g", "wireless.main_5g.encryption"),
    ("hidden_2g", "wireless.main_2g.hidden"),
    ("hidden_5g", "wireless.main_5g.hidden"),
    ("channel_2g", "wireless.wifi0.channel"),
    ("channel_5g", "wireless.wifi1.channel"),
    ("txpower_2g", "wireless.wifi0.txpowerpercent"),
    ("txpower_5g", "wireless.wifi1.txpowerpercent"),
    ("htmode_2g", "wireless.wifi0.htmode"),
    ("htmode_5g", "wireless.wifi1.htmode"),
    ("radio2_disabled", "wireless.wifi0.disabled"),
    ("radio5_disabled", "wireless.wifi1.disabled"),
];

fn wifi_value(key: &str, value: &Value, cfg: &WifiConfig) -> Result<String, String> {
    let text = match value {
        Value::String(s) => s.clone(),
        Value::Bool(v) => if *v { "1" } else { "0" }.into(),
        Value::Number(n) if n.as_u64().is_some() => n.to_string(),
        _ => return Err(format!("{key} has an invalid type")),
    };
    if text.chars().any(char::is_control) {
        return Err(format!("{key} contains a control character"));
    }
    let valid = if key.starts_with("ssid_") {
        // Reject unsupported shell metacharacters explicitly instead of silently
        // renaming the SSID. Firmware downstream handling is not shell-audited.
        (1..=32).contains(&text.len())
            && !text.chars().any(|c| {
                matches!(
                    c,
                    '\'' | '"' | ';' | '$' | '`' | '\\' | '|' | '<' | '>' | '&'
                )
            })
    } else if key.starts_with("key_") {
        (8..=63).contains(&text.len())
            || (text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit()))
    } else if key.starts_with("encryption_") {
        [
            "none",
            "psk2",
            "psk2+ccmp",
            "psk2+aes",
            "psk3",
            "psk3-mixed",
            "sae",
            "sae-mixed",
        ]
        .contains(&text.as_str())
    } else if key.starts_with("txpower_") {
        text.parse::<u8>().is_ok_and(|n| (1..=100).contains(&n))
    } else if key.starts_with("channel_") {
        text == "auto"
            || text == "0"
            || text.parse::<u16>().is_ok_and(|n| {
                if key.ends_with("2g") {
                    (1..=13).contains(&n)
                } else {
                    [
                        36, 40, 44, 48, 52, 56, 60, 64, 100, 104, 108, 112, 116, 120, 124, 128,
                        132, 136, 140, 144, 149, 153, 157, 161, 165,
                    ]
                    .contains(&n)
                }
            })
    } else if key.starts_with("htmode_") {
        let radio = if key.ends_with("2g") {
            "wifi0"
        } else {
            "wifi1"
        };
        bandwidth_options(
            &cfg.get(&format!("{radio}.hwmode")),
            &cfg.get(&format!("{radio}.SupportedStandards")),
            key.ends_with("5g"),
        )
        .contains(&text)
    } else {
        text == "0" || text == "1"
    };
    if valid {
        Ok(text)
    } else {
        Err(format!("{key} is outside the supported values"))
    }
}

fn plan_wifi(parsed: &Value, cfg: &WifiConfig) -> Result<Vec<Change>, String> {
    let obj = parsed
        .as_object()
        .ok_or("expected a Wi-Fi settings object")?;
    if obj.is_empty() || obj.len() > WIFI_FIELDS.len() + 2 {
        return Err("empty or oversized Wi-Fi update".into());
    }
    let mut changes = Vec::new();
    for (key, value) in obj {
        let mut paths = Vec::new();
        if key == WIFI_ONOFF_KEY || key == WIFI6_SWITCH_KEY {
            for path in [
                format!("wireless.zte_mbb.{key}"),
                format!("zte_mbb.wifi.{key}"),
            ] {
                let (config, tail) = path.split_once('.').unwrap();
                if (if config == "wireless" {
                    &cfg.wireless
                } else {
                    &cfg.mbb
                })
                .contains_key(tail)
                {
                    paths.push(path);
                }
            }
            if key == WIFI_ONOFF_KEY && cfg.wireless.contains_key("zte_mbb.wifi_onoff_by_user") {
                paths.push("wireless.zte_mbb.wifi_onoff_by_user".into());
            }
        } else {
            paths.push(
                WIFI_FIELDS
                    .iter()
                    .find(|(name, _)| name == key)
                    .ok_or_else(|| format!("unknown Wi-Fi setting: {key}"))?
                    .1
                    .into(),
            );
        }
        if paths.is_empty() {
            return Err(format!("{key} is not supported by this firmware"));
        }
        if key.starts_with("key_") && value.as_str() == Some("••••••••") {
            continue;
        }
        let after = wifi_value(key, value, cfg)?;
        for path in paths {
            let (config, tail) = path.split_once('.').unwrap();
            let before = (if config == "wireless" {
                &cfg.wireless
            } else {
                &cfg.mbb
            })
            .get(tail)
            .ok_or_else(|| format!("{key} is unavailable on this firmware"))?;
            if before != &after {
                changes.push(Change {
                    key: path.clone(),
                    before: before.clone(),
                    after: after.clone(),
                });
            }
        }
    }
    for suffix in ["2g", "5g"] {
        if obj.contains_key(&format!("encryption_{suffix}"))
            || obj.contains_key(&format!("key_{suffix}"))
        {
            let effective = |field: &str| {
                let path = format!("wireless.main_{suffix}.{field}");
                changes
                    .iter()
                    .find(|c| c.key == path)
                    .map(|c| c.after.clone())
                    .unwrap_or_else(|| cfg.get(&format!("main_{suffix}.{field}")))
            };
            if effective("encryption") != "none" {
                wifi_value(
                    &format!("key_{suffix}"),
                    &Value::String(effective("key")),
                    cfg,
                )?;
            }
        }
    }
    Ok(changes)
}

pub fn wifi_set(state: &AppState, body: &[u8]) -> (u16, Value) {
    let parsed: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(error) => {
            return (
                400,
                json!({"ok": false, "error": format!("invalid Wi-Fi settings: {error}")}),
            )
        }
    };
    let _guard = uci_transaction::WIFI_CHANGE.safe_lock();
    let changes = match plan_wifi(&parsed, &WifiConfig::load()) {
        Ok(changes) => changes,
        Err(error) => return (400, json!({"ok": false, "error": error})),
    };
    let applied = uci_transaction::apply(&changes);
    // Even a failed apply may have reloaded the radios (or restored them), so
    // whatever was cached before is suspect either way.
    if !changes.is_empty() {
        state.net.invalidate_wifi();
    }
    match applied {
        Ok(()) => (
            200,
            json!({"ok": true, "data": {"status": "ok", "changed": !changes.is_empty()}}),
        ),
        Err(error) => (503, json!({"ok": false, "error": error})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> WifiConfig {
        let mut wireless = HashMap::new();
        for (_, path) in WIFI_FIELDS {
            wireless.insert(path.strip_prefix("wireless.").unwrap().into(), "0".into());
        }
        wireless.insert("wifi0.hwmode".into(), "11beg".into());
        wireless.insert("wifi0.SupportedStandards".into(), "b,g,n,ax,be".into());
        wireless.insert("main_2g.key".into(), "oldpassword".into());
        wireless.insert("main_2g.encryption".into(), "psk3-mixed".into());
        WifiConfig {
            wireless,
            mbb: HashMap::new(),
        }
    }
    #[test]
    fn validates_entire_request_and_rejects_unknown_fields() {
        let cfg = config();
        for body in [
            json!({"ssid_2g": "Valid", "txpower_2g": 101}),
            json!({"key_2g": "short"}),
            json!({"channel_2g": 36}),
            json!({"ssid_2g": "line\nbreak"}),
            json!({"ssid_2g": "a".repeat(33)}),
            json!({"radio2_disabled": 3}),
            json!({"encryption_2g": "invalid"}),
            json!({"unknown": true}),
            json!({"htmode_2g": "HT40"}),
        ] {
            assert!(plan_wifi(&body, &cfg).is_err(), "{body}");
        }
    }
    #[test]
    fn accepts_special_character_passwords_without_changing_them() {
        let key = r#"Pass$word'";\|<>&`!"#;
        let plan = plan_wifi(&json!({"key_2g": key, "htmode_2g": "EHT40"}), &config()).unwrap();
        assert_eq!(
            plan.iter().find(|c| c.key.ends_with(".key")).unwrap().after,
            key
        );
    }
    #[test]
    fn wifi7_radios_use_eht_bandwidth_names() {
        assert_eq!(
            bandwidth_options("11beg", "b,g,n,ax,be", false),
            ["EHT20", "EHT40"]
        );
        assert_eq!(
            bandwidth_options("11bea", "a,n,ac,ax,be", true),
            ["EHT20", "EHT40", "EHT80", "EHT160"]
        );
    }
}
