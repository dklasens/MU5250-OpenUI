use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::handlers::AppState;
use crate::ubus;

const WIFI_CONFIG_PATH: &str = "/data/local/tmp/wifi_config.json";
const PERSIST_ON_BOOT_KEY: &str = "persist_on_boot";
const WIFI_ONOFF_KEY: &str = "wifi_onoff";
const WIFI_ONOFF_BY_USER_KEY: &str = "wifi_onoff_by_user";
const WIFI6_SWITCH_KEY: &str = "wifi6_switch";

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

pub fn enforce_wifi_state_on_boot() {
    let persisted = read_persisted_wifi_state();
    if !persist_on_boot_enabled(&persisted) {
        return;
    }
    // Stock /sbin/zte_start_wlan_at_boot.sh forces both radios enabled and
    // brings up Wi-Fi via zwrt_qcmap_cli after a 5-40s wait for QCMAP/topsw
    // services. Racing it from rc.local leaves the radios in an indeterminate
    // state — sometimes both off. Spawn off the main thread so the HTTP
    // server starts immediately, then wait for stock to finish before applying.
    std::thread::spawn(move || {
        wait_for_stock_wlan_boot(Duration::from_secs(90));
        wait_for_wlan_idle(Duration::from_secs(30));
        apply_persisted_state(&persisted, 2);
    });
}

fn wait_for_stock_wlan_boot(max: Duration) {
    let deadline = Instant::now() + max;
    // Brief grace period so an about-to-start stock script registers.
    std::thread::sleep(Duration::from_millis(500));
    while Instant::now() < deadline {
        let running = Command::new("sh")
            .args(["-c", "pgrep -f zte_start_wlan_at_boot >/dev/null"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !running {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn wait_for_wlan_idle(max: Duration) {
    let deadline = Instant::now() + max;
    let mut consecutive_idle = 0;
    while Instant::now() < deadline {
        let status = ubus::call("zwrt_wlan", "status", Some("{}"))
            .ok()
            .and_then(|v| v["app_status"].as_str().map(String::from))
            .unwrap_or_default();
        if status == "idle" {
            consecutive_idle += 1;
            if consecutive_idle >= 2 {
                return;
            }
        } else {
            consecutive_idle = 0;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn apply_persisted_state(persisted: &Value, max_attempts: u32) {
    for attempt in 1..=max_attempts {
        let mut changed = false;

        if let Some(global) = persisted[WIFI_ONOFF_KEY].as_str().filter(|s| !s.is_empty()) {
            let current_global = current_wifi_onoff_value();
            let current_user = uci_get_feature(WIFI_ONOFF_BY_USER_KEY);
            if current_global != global || (!current_user.is_empty() && current_user != global) {
                let _ = ubus::uci_set_no_commit("wireless.zte_mbb.wifi_onoff", global);
                if !current_user.is_empty() {
                    let _ = ubus::uci_set_no_commit("wireless.zte_mbb.wifi_onoff_by_user", global);
                }
                changed = true;
            }
        }
        if let Some(r2) = persisted["radio2_disabled"]
            .as_str()
            .filter(|s| !s.is_empty())
        {
            if ubus::uci_get("wireless.wifi0.disabled").unwrap_or_default() != r2 {
                let _ = ubus::uci_set_no_commit("wireless.wifi0.disabled", r2);
                changed = true;
            }
        }
        if let Some(r5) = persisted["radio5_disabled"]
            .as_str()
            .filter(|s| !s.is_empty())
        {
            if ubus::uci_get("wireless.wifi1.disabled").unwrap_or_default() != r5 {
                let _ = ubus::uci_set_no_commit("wireless.wifi1.disabled", r5);
                changed = true;
            }
        }

        if !changed {
            return;
        }

        if ubus::uci_commit("wireless").is_err() {
            if attempt < max_attempts {
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
            return;
        }
        let _ = reload_wireless();
        // Give the wlan daemon time to consume the reload then settle.
        wait_for_wlan_idle(Duration::from_secs(20));

        let report = ubus::call("zwrt_wlan", "report", Some("{}")).ok();
        let want_r2 = persisted["radio2_disabled"].as_str().unwrap_or("");
        let want_r5 = persisted["radio5_disabled"].as_str().unwrap_or("");
        let actual_r2 = report
            .as_ref()
            .and_then(|v| v["radio2_disabled"].as_str())
            .unwrap_or("");
        let actual_r5 = report
            .as_ref()
            .and_then(|v| v["radio5_disabled"].as_str())
            .unwrap_or("");
        let r2_ok = want_r2.is_empty() || actual_r2 == want_r2;
        let r5_ok = want_r5.is_empty() || actual_r5 == want_r5;
        if r2_ok && r5_ok {
            return;
        }
        if attempt < max_attempts {
            std::thread::sleep(Duration::from_secs(2));
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn uci_get_wireless(key: &str) -> String {
    ubus::uci_get(&format!("wireless.{key}")).unwrap_or_default()
}

fn uci_get_feature(key: &str) -> String {
    uci_get_wireless(&format!("zte_mbb.{key}"))
}

fn report_value(report: Option<&Value>, key: &str) -> String {
    report
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

fn current_wifi_onoff_value() -> String {
    let value = uci_get_feature(WIFI_ONOFF_KEY);
    if !value.is_empty() {
        return value;
    }
    let report = ubus::call("zwrt_wlan", "report", Some("{}")).ok();
    report_value(report.as_ref(), WIFI_ONOFF_KEY)
}

fn iw_info(iface: &str) -> (String, String) {
    let output = Command::new("iw").args([iface, "info"]).output().ok();
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

fn station_count(iface: &str) -> u64 {
    let output = Command::new("sh")
        .args([
            "-c",
            &format!("iw {iface} station dump 2>/dev/null | grep -c Station"),
        ])
        .output()
        .ok();
    output
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse::<u64>()
                .ok()
        })
        .unwrap_or(0)
}

fn sanitize_uci_value(v: &str) -> String {
    v.chars()
        .filter(|c| {
            !matches!(
                c,
                '\'' | '"' | ';' | '$' | '`' | '\\' | '|' | '<' | '>' | '&'
            )
        })
        .collect()
}

fn parse_bool_str(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "enabled" => Some(true),
        "0" | "false" | "no" | "off" | "disabled" => Some(false),
        _ => None,
    }
}

fn parse_bool_value(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_i64().map(|v| v != 0),
        Value::String(s) => parse_bool_str(s),
        _ => None,
    }
}

fn read_persisted_wifi_state() -> Value {
    std::fs::read_to_string(WIFI_CONFIG_PATH)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or_else(|| json!({}))
}

fn write_persisted_wifi_state(persisted: &Value) {
    let _ = std::fs::write(
        WIFI_CONFIG_PATH,
        serde_json::to_string(persisted).unwrap_or_default(),
    );
}

/// Refresh wifi_onoff and per-radio fields from live state, skipping empty
/// reads so we never persist invalid values that would brick boot enforcement.
/// Returns true if any field changed.
fn snapshot_persistable_state_into(persisted: &mut Value) -> bool {
    let mut changed = false;
    let wifi_onoff = current_wifi_onoff_value();
    if !wifi_onoff.is_empty() && persisted[WIFI_ONOFF_KEY].as_str() != Some(wifi_onoff.as_str()) {
        persisted[WIFI_ONOFF_KEY] = json!(wifi_onoff);
        changed = true;
    }
    let r2 = ubus::uci_get("wireless.wifi0.disabled").unwrap_or_default();
    if !r2.is_empty() && persisted["radio2_disabled"].as_str() != Some(r2.as_str()) {
        persisted["radio2_disabled"] = json!(r2);
        changed = true;
    }
    let r5 = ubus::uci_get("wireless.wifi1.disabled").unwrap_or_default();
    if !r5.is_empty() && persisted["radio5_disabled"].as_str() != Some(r5.as_str()) {
        persisted["radio5_disabled"] = json!(r5);
        changed = true;
    }
    changed
}

fn reload_wireless() -> Result<(), String> {
    ubus::call("zwrt_wlan", "reload", Some("{}")).map(|_| ())
}

fn persist_on_boot_enabled(persisted: &Value) -> bool {
    persisted
        .get(PERSIST_ON_BOOT_KEY)
        .and_then(parse_bool_value)
        .unwrap_or(true)
}

fn sanitize_wifi_key_value(v: &str) -> String {
    // Keep Wi-Fi passphrases intact except for control chars that can corrupt UCI entries.
    v.chars()
        .filter(|c| !matches!(c, '\u{0000}' | '\n' | '\r'))
        .collect()
}

fn sanitize_wifi_input_value(key: &str, v: &str) -> String {
    match key {
        "key_2g" | "key_5g" | "guest_key" => sanitize_wifi_key_value(v),
        _ => sanitize_uci_value(v),
    }
}

// ---------------------------------------------------------------------------
// GET /api/wifi/status
// ---------------------------------------------------------------------------

pub fn wifi_status(_state: &AppState) -> (u16, Value) {
    let mut result = serde_json::Map::new();
    let report = ubus::call("zwrt_wlan", "report", Some("{}")).ok();

    // Global switches from wireless feature config, with report fallback when exposed there.
    let mut wifi_onoff = uci_get_feature(WIFI_ONOFF_KEY);
    if wifi_onoff.is_empty() {
        wifi_onoff = report_value(report.as_ref(), WIFI_ONOFF_KEY);
    }
    let wifi_onoff_supported = !wifi_onoff.is_empty();
    if wifi_onoff_supported {
        result.insert("wifi_onoff".into(), json!(wifi_onoff));
    }
    result.insert("wifi_onoff_supported".into(), json!(wifi_onoff_supported));

    let mut wifi6_switch = uci_get_feature(WIFI6_SWITCH_KEY);
    if wifi6_switch.is_empty() {
        wifi6_switch = report_value(report.as_ref(), WIFI6_SWITCH_KEY);
    }
    let wifi6_supported = !wifi6_switch.is_empty();
    if wifi6_supported {
        result.insert("wifi6_switch".into(), json!(wifi6_switch));
    }
    result.insert("wifi6_supported".into(), json!(wifi6_supported));

    let mut persisted = read_persisted_wifi_state();
    let persist_enabled = persist_on_boot_enabled(&persisted);
    // Keep the snapshot in sync with reality whenever someone views the page.
    // Without this, touchscreen toggles or per-band changes leave wifi_onoff
    // stale and boot enforcement would force the old value.
    if persist_enabled && snapshot_persistable_state_into(&mut persisted) {
        write_persisted_wifi_state(&persisted);
    }
    result.insert(PERSIST_ON_BOOT_KEY.into(), json!(persist_enabled));

    // Radio config
    result.insert(
        "radio2_disabled".into(),
        json!(uci_get_wireless("wifi0.disabled")),
    );
    result.insert(
        "radio5_disabled".into(),
        json!(uci_get_wireless("wifi1.disabled")),
    );
    result.insert(
        "channel_2g".into(),
        json!(uci_get_wireless("wifi0.channel")),
    );
    result.insert(
        "channel_5g".into(),
        json!(uci_get_wireless("wifi1.channel")),
    );
    result.insert(
        "txpower_2g".into(),
        json!(uci_get_wireless("wifi0.txpowerpercent")),
    );
    result.insert(
        "txpower_5g".into(),
        json!(uci_get_wireless("wifi1.txpowerpercent")),
    );
    result.insert("htmode_2g".into(), json!(uci_get_wireless("wifi0.htmode")));
    result.insert("htmode_5g".into(), json!(uci_get_wireless("wifi1.htmode")));
    result.insert(
        "country_code".into(),
        json!(uci_get_wireless("wifi0.country")),
    );

    // Interface config
    result.insert("ssid_2g".into(), json!(uci_get_wireless("main_2g.ssid")));
    result.insert("ssid_5g".into(), json!(uci_get_wireless("main_5g.ssid")));
    result.insert("key_2g".into(), json!(uci_get_wireless("main_2g.key")));
    result.insert("key_5g".into(), json!(uci_get_wireless("main_5g.key")));
    result.insert(
        "has_key_2g".into(),
        json!(!uci_get_wireless("main_2g.key").is_empty()),
    );
    result.insert(
        "has_key_5g".into(),
        json!(!uci_get_wireless("main_5g.key").is_empty()),
    );
    result.insert(
        "encryption_2g".into(),
        json!(uci_get_wireless("main_2g.encryption")),
    );
    result.insert(
        "encryption_5g".into(),
        json!(uci_get_wireless("main_5g.encryption")),
    );
    result.insert(
        "hidden_2g".into(),
        json!(uci_get_wireless("main_2g.hidden")),
    );
    result.insert(
        "hidden_5g".into(),
        json!(uci_get_wireless("main_5g.hidden")),
    );

    // Runtime info from iw
    let (ch2, bw2) = iw_info("wlan0");
    let (ch5, bw5) = iw_info("wlan2");
    result.insert("actual_channel_2g".into(), json!(ch2));
    result.insert("actual_bw_2g".into(), json!(bw2));
    result.insert("actual_channel_5g".into(), json!(ch5));
    result.insert("actual_bw_5g".into(), json!(bw5));

    // Client counts
    let c2g = station_count("wlan0");
    let c5g = station_count("wlan2");
    result.insert("clients_2g".into(), json!(c2g));
    result.insert("clients_5g".into(), json!(c5g));
    result.insert("clients_total".into(), json!(c2g + c5g));

    // Guest WiFi summary
    result.insert(
        "guest_disabled_2g".into(),
        json!(uci_get_wireless("guest_2g.disabled")),
    );
    result.insert(
        "guest_disabled_5g".into(),
        json!(uci_get_wireless("guest_5g.disabled")),
    );
    result.insert(
        "guest_ssid".into(),
        json!(uci_get_wireless("guest_2g.ssid")),
    );

    (200, json!({"ok": true, "data": result}))
}

// ---------------------------------------------------------------------------
// PUT /api/wifi/settings
// ---------------------------------------------------------------------------

pub fn wifi_set(_state: &AppState, body: &[u8]) -> (u16, Value) {
    let parsed: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({"ok": false, "error": "invalid JSON"})),
    };
    let obj = match parsed.as_object() {
        Some(o) => o,
        None => return (400, json!({"ok": false, "error": "expected JSON object"})),
    };

    let uci_map: &[(&str, &str)] = &[
        ("ssid_2g", "wireless.main_2g.ssid"),
        ("ssid_5g", "wireless.main_5g.ssid"),
        ("key_2g", "wireless.main_2g.key"),
        ("key_5g", "wireless.main_5g.key"),
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
    let txpower_keys: &[&str] = &["txpower_2g", "txpower_5g"];

    let mut wireless_changed = false;
    let mut only_txpower = true;
    let mut txpower_2g_val: Option<u32> = None;
    let mut txpower_5g_val: Option<u32> = None;

    let mut persisted_state = read_persisted_wifi_state();
    let mut persisted_state_changed = false;

    for (key, value) in obj {
        if key == PERSIST_ON_BOOT_KEY {
            let enabled = match parse_bool_value(value) {
                Some(v) => v,
                None => {
                    return (
                        400,
                        json!({"ok": false, "error": "invalid persist_on_boot value"}),
                    )
                }
            };
            persisted_state[PERSIST_ON_BOOT_KEY] = json!(if enabled { "1" } else { "0" });
            persisted_state_changed = true;
            if enabled {
                // Snapshot live state, skipping empty reads so we never persist
                // a value that would brick boot enforcement.
                snapshot_persistable_state_into(&mut persisted_state);
            }
            continue;
        }

        let val_str = match value {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => if *b { "1" } else { "0" }.to_string(),
            _ => continue,
        };
        let val_str = sanitize_wifi_input_value(key, &val_str);

        // Prevent overwriting with masked placeholder
        if (key == "key_2g" || key == "key_5g") && val_str == "••••••••" {
            continue;
        }

        if key == "radio2_disabled" || key == "radio5_disabled" {
            persisted_state[key.as_str()] = json!(val_str);
            persisted_state_changed = true;
        }

        if key == WIFI_ONOFF_KEY {
            let mut changed_any = false;
            let current = ubus::uci_get("wireless.zte_mbb.wifi_onoff").unwrap_or_default();
            if current != val_str {
                if let Err(e) = ubus::uci_set_no_commit("wireless.zte_mbb.wifi_onoff", &val_str) {
                    return (500, json!({"ok": false, "error": e}));
                }
                changed_any = true;
            }

            let current_user =
                ubus::uci_get("wireless.zte_mbb.wifi_onoff_by_user").unwrap_or_default();
            if !current_user.is_empty() && current_user != val_str {
                if let Err(e) =
                    ubus::uci_set_no_commit("wireless.zte_mbb.wifi_onoff_by_user", &val_str)
                {
                    return (500, json!({"ok": false, "error": e}));
                }
                changed_any = true;
            }

            persisted_state[WIFI_ONOFF_KEY] = json!(val_str);
            persisted_state_changed = true;

            if changed_any {
                wireless_changed = true;
                only_txpower = false;
            }
            continue;
        }

        if key == WIFI6_SWITCH_KEY {
            let current = ubus::uci_get("wireless.zte_mbb.wifi6_switch").unwrap_or_default();
            if current.is_empty() {
                continue;
            }
            if current != val_str {
                if let Err(e) = ubus::uci_set_no_commit("wireless.zte_mbb.wifi6_switch", &val_str) {
                    return (500, json!({"ok": false, "error": e}));
                }
                wireless_changed = true;
                only_txpower = false;
            }
            continue;
        }

        // Check wireless UCI map
        if let Some(&(_, path)) = uci_map.iter().find(|&&(k, _)| k == key) {
            let current = ubus::uci_get(path).unwrap_or_default();
            if current != val_str {
                if let Err(e) = ubus::uci_set_no_commit(path, &val_str) {
                    return (500, json!({"ok": false, "error": e}));
                }
                wireless_changed = true;
                if !txpower_keys.contains(&key.as_str()) {
                    only_txpower = false;
                } else if key == "txpower_2g" {
                    txpower_2g_val = val_str.parse().ok();
                } else if key == "txpower_5g" {
                    txpower_5g_val = val_str.parse().ok();
                }
            }
            continue;
        }
    }

    // Commit batched changes
    if wireless_changed {
        if let Err(e) = ubus::uci_commit("wireless") {
            return (500, json!({"ok": false, "error": e}));
        }
    }

    // Only persist when the toggle is on (or the user just changed it), so the
    // file doesn't drift while persistence is disabled.
    let persist_toggled = obj.contains_key(PERSIST_ON_BOOT_KEY);
    let persist_currently_on = persist_on_boot_enabled(&persisted_state);
    if (wireless_changed || persisted_state_changed) && (persist_toggled || persist_currently_on) {
        write_persisted_wifi_state(&persisted_state);
    }

    if !wireless_changed && !persisted_state_changed {
        return (
            200,
            json!({"ok": true, "data": {"status": "ok", "note": "no changes"}}),
        );
    }

    // Hot-apply txpower if that's the only change
    if wireless_changed && only_txpower {
        if let Some(val) = txpower_2g_val {
            let _ = Command::new("iw")
                .args([
                    "dev",
                    "wlan0",
                    "set",
                    "txpower",
                    "limit",
                    &(val * 30).to_string(),
                ])
                .output();
        }
        if let Some(val) = txpower_5g_val {
            let _ = Command::new("iw")
                .args([
                    "dev",
                    "wlan2",
                    "set",
                    "txpower",
                    "limit",
                    &(val * 30).to_string(),
                ])
                .output();
        }
        return (
            200,
            json!({"ok": true, "data": {"status": "ok", "hot": true}}),
        );
    }

    // Full reload needed. Run synchronously so a reload failure surfaces to
    // the caller instead of leaving UCI committed but running config stale.
    if wireless_changed {
        if let Err(e) = reload_wireless() {
            return (
                500,
                json!({"ok": false, "error": format!("wireless reload failed: {e}")}),
            );
        }
    }

    (200, json!({"ok": true, "data": {"status": "ok"}}))
}

// ---------------------------------------------------------------------------
// GET /api/wifi/guest
// ---------------------------------------------------------------------------

pub fn guest_status(_state: &AppState) -> (u16, Value) {
    let mut result = serde_json::Map::new();

    result.insert("ssid".into(), json!(uci_get_wireless("guest_2g.ssid")));
    result.insert("key".into(), json!(uci_get_wireless("guest_2g.key")));
    result.insert(
        "encryption".into(),
        json!(uci_get_wireless("guest_2g.encryption")),
    );
    result.insert(
        "disabled_2g".into(),
        json!(uci_get_wireless("guest_2g.disabled")),
    );
    result.insert(
        "disabled_5g".into(),
        json!(uci_get_wireless("guest_5g.disabled")),
    );
    result.insert("hidden".into(), json!(uci_get_wireless("guest_2g.hidden")));
    result.insert(
        "isolate".into(),
        json!(uci_get_wireless("guest_2g.isolate")),
    );
    result.insert(
        "guest_active_time".into(),
        json!(uci_get_wireless("guest_2g.guest_active_time")),
    );

    // Runtime remaining time
    let remaining = ubus::call("zwrt_wlan", "wlan_get_guest_access_left_time", Some("{}"))
        .ok()
        .and_then(|v| {
            v["guest_left_time"]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
        })
        .unwrap_or(-1);
    result.insert("remaining_seconds".into(), json!(remaining));

    (200, json!({"ok": true, "data": result}))
}

// ---------------------------------------------------------------------------
// PUT /api/wifi/guest
// ---------------------------------------------------------------------------

pub fn guest_set(_state: &AppState, body: &[u8]) -> (u16, Value) {
    let parsed: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({"ok": false, "error": "invalid JSON"})),
    };
    let obj = match parsed.as_object() {
        Some(o) => o,
        None => return (400, json!({"ok": false, "error": "expected JSON object"})),
    };

    let guest_map: &[(&str, &[&str])] = &[
        (
            "guest_ssid",
            &["wireless.guest_2g.ssid", "wireless.guest_5g.ssid"],
        ),
        (
            "guest_key",
            &["wireless.guest_2g.key", "wireless.guest_5g.key"],
        ),
        (
            "guest_encryption",
            &[
                "wireless.guest_2g.encryption",
                "wireless.guest_5g.encryption",
            ],
        ),
        (
            "guest_disabled",
            &["wireless.guest_2g.disabled", "wireless.guest_5g.disabled"],
        ),
        ("guest_disabled_2g", &["wireless.guest_2g.disabled"]),
        ("guest_disabled_5g", &["wireless.guest_5g.disabled"]),
        (
            "guest_hidden",
            &["wireless.guest_2g.hidden", "wireless.guest_5g.hidden"],
        ),
        (
            "guest_isolate",
            &["wireless.guest_2g.isolate", "wireless.guest_5g.isolate"],
        ),
        (
            "guest_active_time",
            &[
                "wireless.guest_2g.guest_active_time",
                "wireless.guest_5g.guest_active_time",
            ],
        ),
    ];

    let mut changed = false;
    for (key, value) in obj {
        let val_str = match value {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => if *b { "1" } else { "0" }.to_string(),
            _ => continue,
        };
        let val_str = sanitize_wifi_input_value(key, &val_str);

        if let Some(&(_, paths)) = guest_map.iter().find(|&&(k, _)| k == key) {
            for &path in paths {
                // Guest interfaces may not exist; skip silently if missing
                if ubus::uci_set_no_commit(path, &val_str).is_ok() {
                    changed = true;
                }
            }
        }
    }

    if !changed {
        return (
            200,
            json!({"ok": true, "data": {"status": "ok", "note": "no changes"}}),
        );
    }

    // Commit batched changes
    if let Err(e) = ubus::uci_commit("wireless") {
        return (500, json!({"ok": false, "error": e}));
    }
    if let Err(e) = reload_wireless() {
        return (
            500,
            json!({"ok": false, "error": format!("wireless reload failed: {e}")}),
        );
    }

    (200, json!({"ok": true, "data": {"status": "ok"}}))
}

#[cfg(test)]
mod tests {
    use super::{persist_on_boot_enabled, sanitize_wifi_input_value};
    use serde_json::json;

    #[test]
    fn wifi_keys_keep_special_characters() {
        let input = r#"Pass$word'";\|<>&`!"#;
        assert_eq!(sanitize_wifi_input_value("key_5g", input), input);
        assert_eq!(sanitize_wifi_input_value("guest_key", input), input);
    }

    #[test]
    fn wifi_keys_strip_control_characters_only() {
        let input = "line1\nline2\rline3\u{0000}";
        assert_eq!(
            sanitize_wifi_input_value("key_2g", input),
            "line1line2line3"
        );
    }

    #[test]
    fn non_key_values_remain_sanitized() {
        let input = r#"wifi$';`"\name|<&"#;
        assert_eq!(sanitize_wifi_input_value("ssid_5g", input), "wifiname");
    }

    #[test]
    fn persist_on_boot_defaults_to_true_when_missing() {
        assert!(persist_on_boot_enabled(&json!({})));
    }

    #[test]
    fn persist_on_boot_parses_common_disabled_values() {
        assert!(!persist_on_boot_enabled(&json!({"persist_on_boot": false})));
        assert!(!persist_on_boot_enabled(&json!({"persist_on_boot": 0})));
        assert!(!persist_on_boot_enabled(&json!({"persist_on_boot": "off"})));
        assert!(!persist_on_boot_enabled(&json!({"persist_on_boot": "0"})));
    }

    #[test]
    fn persist_on_boot_parses_common_enabled_values() {
        assert!(persist_on_boot_enabled(&json!({"persist_on_boot": true})));
        assert!(persist_on_boot_enabled(&json!({"persist_on_boot": 1})));
        assert!(persist_on_boot_enabled(&json!({"persist_on_boot": "on"})));
        assert!(persist_on_boot_enabled(&json!({"persist_on_boot": "1"})));
    }
}
