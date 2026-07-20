use std::fs;
use std::os::unix::fs as unix_fs;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use crate::handlers::AppState;
use crate::ubus;

const GADGET_DIR: &str = "/sys/kernel/config/usb_gadget/g1";
const CONFIG_DIR: &str = "/sys/kernel/config/usb_gadget/g1/configs/c.1";
const UDC_PATH: &str = "/sys/kernel/config/usb_gadget/g1/UDC";
const NCM_FUNC: &str = "/sys/kernel/config/usb_gadget/g1/functions/ncm.0";
const ECM_GSI_FUNC: &str = "/sys/kernel/config/usb_gadget/g1/functions/gsi.ecm";
const RNDIS_GSI_FUNC: &str = "/sys/kernel/config/usb_gadget/g1/functions/gsi.rndis";
const MASS_STORAGE_FUNC: &str = "/sys/kernel/config/usb_gadget/g1/functions/mass_storage.0";
const NCM_LAST_ERROR_PATH: &str = "/tmp/zte-agent-usb-ncm.last_error";
const PERSIST_CONFIG_PATH: &str = "/data/local/tmp/usb_config.json";
/// Legacy location: NCM persistence used to share the Wi-Fi snapshot file.
/// On first read we migrate the key out, so the two concerns are decoupled.
const LEGACY_PERSIST_CONFIG_PATH: &str = "/data/local/tmp/wifi_config.json";
const USB_DEFAULT_MODE_KEY: &str = "usb_default_mode";

// ---------------------------------------------------------------------------
// Boot persistence
// ---------------------------------------------------------------------------

pub fn enforce_usb_mode_on_boot() {
    if parse_usb_default_mode(&read_persisted_config()) != Some("ncm") {
        return;
    }

    // Powering the device off while it stays plugged in soft-reboots into a
    // charging state with no WAN. Re-presenting the NCM tether there makes the
    // tethered host route all traffic through a dead link. Skip enforcement in
    // that state, mirroring the stock Wi-Fi boot guard (zte_start_wlan_at_boot.sh).
    if is_power_off_charging() {
        return;
    }

    thread::spawn(|| {
        wait_for_usb_boot_ready(Duration::from_secs(75));
        // Re-check: mode_main_state can still be settling early in boot, and the
        // readiness wait above can run for up to 75s.
        if is_power_off_charging() {
            return;
        }
        if detect_active_usb_mode() == Some("ncm") {
            let _ = fs::remove_file(NCM_LAST_ERROR_PATH);
            return;
        }

        match switch_to_ncm_now() {
            Ok(()) => {
                let _ = fs::remove_file(NCM_LAST_ERROR_PATH);
            }
            Err(e) => {
                let _ = fs::write(NCM_LAST_ERROR_PATH, format!("boot NCM persistence: {e}"));
            }
        }
    });
}

/// Stock power-off-while-charging detection. When the device is switched off but
/// left plugged into USB, it soft-reboots into a charging state where
/// `zwrt_zte_mc_tmp.mode.mode_main_state` reads one of these values. The stock
/// Wi-Fi boot script guards on the same values; we mirror it so NCM persistence
/// doesn't re-present the tether to a host that has no WAN behind it.
fn is_power_off_charging() -> bool {
    is_power_off_charging_state(read_mode_main_state().as_deref())
}

fn is_power_off_charging_state(state: Option<&str>) -> bool {
    matches!(
        state,
        Some("mode_power_off_charger") | Some("mode_power_off_unreal")
    )
}

fn read_mode_main_state() -> Option<String> {
    let output = Command::new("uci")
        .args(["get", "zwrt_zte_mc_tmp.mode.mode_main_state"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn wait_for_usb_boot_ready(max: Duration) {
    let deadline = Instant::now() + max;
    let mut consecutive_ready = 0;
    while Instant::now() < deadline {
        let configfs_ready = Path::new(GADGET_DIR).exists()
            && Path::new(CONFIG_DIR).exists()
            && Path::new(NCM_FUNC).exists()
            && Path::new(MASS_STORAGE_FUNC).exists();
        let controller_ready = first_udc_name().is_some() || read_trimmed(UDC_PATH).is_some();
        let stock_composition_ready =
            !current_composition_functions().is_empty() || detect_active_usb_mode().is_some();
        // Stock adds ecm0/rndis0 to br-lan at the end of its init. Once we
        // see it bridged the stock USB stack is done tinkering, so it's
        // safe to rebuild the gadget on top.
        let stock_bridged = bridge_members("br-lan")
            .iter()
            .any(|m| m == "ecm0" || m == "rndis0");
        if configfs_ready && controller_ready && stock_composition_ready && stock_bridged {
            consecutive_ready += 1;
            // 4 × 250ms = 1s steady state, in case stock makes one more pass.
            if consecutive_ready >= 4 {
                return;
            }
        } else {
            consecutive_ready = 0;
        }
        thread::sleep(Duration::from_millis(250));
    }
}

/// Detect the currently active USB function by probing `/sys/class/net/`.
/// `zwrt_bsp.usb list` returns `mode: "user"` (a permission flag, not the
/// function), so we look at which gadget interface actually exists.
fn detect_active_usb_mode() -> Option<&'static str> {
    let functions = current_composition_functions();
    if functions.iter().any(|f| f == "ncm.0") {
        Some("ncm")
    } else if functions.iter().any(|f| f == "gsi.ecm" || f == "ecm.ecm") {
        Some("ecm")
    } else if functions
        .iter()
        .any(|f| f == "gsi.rndis" || f == "rndis.rndis")
    {
        Some("rndis")
    } else if Path::new("/sys/class/net/ncm0").exists() {
        Some("ncm")
    } else if Path::new("/sys/class/net/ecm0").exists() {
        Some("ecm")
    } else if Path::new("/sys/class/net/rndis0").exists() {
        Some("rndis")
    } else {
        None
    }
}

fn current_composition_functions() -> Vec<String> {
    let mut functions = Vec::new();
    let entries = match fs::read_dir(CONFIG_DIR) {
        Ok(v) => v,
        Err(_) => return functions,
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with('f') {
            continue;
        }
        let Ok(target) = fs::read_link(entry.path()) else {
            continue;
        };
        if let Some(function) = target.file_name().and_then(|s| s.to_str()) {
            functions.push(function.to_string());
        }
    }
    functions.sort();
    functions
}

fn bridge_members(bridge: &str) -> Vec<String> {
    let mut members = Vec::new();
    let path = format!("/sys/class/net/{bridge}/brif");
    let entries = match fs::read_dir(path) {
        Ok(v) => v,
        Err(_) => return members,
    };
    for entry in entries.flatten() {
        members.push(entry.file_name().to_string_lossy().to_string());
    }
    members.sort();
    members
}

fn read_trimmed(path: &str) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn ncm_interface_name() -> Option<String> {
    let ifname = read_trimmed(&format!("{NCM_FUNC}/ifname"))?;
    if ifname == "(unnamed net_device)" {
        return None;
    }
    Path::new(&format!("/sys/class/net/{ifname}"))
        .exists()
        .then_some(ifname)
}

fn read_persisted_config() -> Value {
    if let Some(value) = fs::read_to_string(PERSIST_CONFIG_PATH)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    {
        return value;
    }
    // Migrate the USB key out of the legacy Wi-Fi snapshot file on first read.
    let legacy = match fs::read_to_string(LEGACY_PERSIST_CONFIG_PATH) {
        Ok(s) => s,
        Err(_) => return json!({}),
    };
    let mut parsed: Value = match serde_json::from_str(&legacy) {
        Ok(v) => v,
        Err(_) => return json!({}),
    };
    let Some(legacy_obj) = parsed.as_object_mut() else {
        return json!({});
    };
    let Some(value) = legacy_obj.remove(USB_DEFAULT_MODE_KEY) else {
        return json!({});
    };
    let migrated = json!({ USB_DEFAULT_MODE_KEY: value });
    let _ = fs::write(
        PERSIST_CONFIG_PATH,
        serde_json::to_string(&migrated).unwrap_or_default(),
    );
    let _ = fs::write(
        LEGACY_PERSIST_CONFIG_PATH,
        serde_json::to_string(&parsed).unwrap_or_default(),
    );
    migrated
}

fn write_persisted_config(persisted: &Value) -> Result<(), String> {
    fs::write(
        PERSIST_CONFIG_PATH,
        serde_json::to_string(persisted).map_err(|e| format!("encode persistence config: {e}"))?,
    )
    .map_err(|e| format!("write {PERSIST_CONFIG_PATH}: {e}"))
}

fn parse_usb_default_mode(persisted: &Value) -> Option<&'static str> {
    match persisted
        .get(USB_DEFAULT_MODE_KEY)
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("ncm") => Some("ncm"),
        Some("ecm") => Some("ecm"),
        _ => None,
    }
}

fn set_usb_default_mode(mode: &str) -> Result<(), String> {
    let mut persisted = read_persisted_config();
    if !persisted.is_object() {
        persisted = json!({});
    }
    let Some(obj) = persisted.as_object_mut() else {
        return Err("persistence config is not an object".into());
    };
    obj.insert(USB_DEFAULT_MODE_KEY.into(), json!(mode));
    write_persisted_config(&persisted)
}

fn supported_modes() -> Vec<&'static str> {
    let mut modes = vec!["rndis", "ecm"];
    if Path::new(NCM_FUNC).exists() {
        modes.push("ncm");
    }
    modes
}

/// Map a Linux UDC speed string (`current_speed`/`maximum_speed`) to a friendly
/// USB generation label and its line rate in Mbit/s.
fn usb_speed_label(speed: &str) -> Option<(&'static str, f64)> {
    match speed {
        "low-speed" => Some(("USB 1.0", 1.5)),
        "full-speed" => Some(("USB 1.1", 12.0)),
        "high-speed" => Some(("USB 2.0", 480.0)),
        "super-speed" => Some(("USB 3.0", 5000.0)),
        "super-speed-plus" => Some(("USB 3.1 Gen2", 10000.0)),
        _ => None,
    }
}

/// Negotiated vs. maximum USB link speed reported by the device controller. The
/// negotiated value reflects the weakest of {controller, cable, host port}, so a
/// 10 Gbit/s cable reads `super-speed-plus` while a USB 2.0 cable reads
/// `high-speed` even though the SDX75 dwc3 controller can do SuperSpeed+.
fn usb_link_info() -> Option<Value> {
    let udc = read_trimmed(UDC_PATH).or_else(first_udc_name)?;
    let base = format!("/sys/class/udc/{udc}");
    let negotiated = read_trimmed(&format!("{base}/current_speed"));
    let max = read_trimmed(&format!("{base}/maximum_speed"));
    if negotiated.is_none() && max.is_none() {
        return None;
    }

    let mut link = Map::new();
    let mut neg_mbps = None;
    if let Some(n) = negotiated {
        if let Some((label, mbps)) = usb_speed_label(&n) {
            link.insert("negotiated_label".into(), json!(label));
            link.insert("negotiated_mbps".into(), json!(mbps));
            neg_mbps = Some(mbps);
        }
        link.insert("negotiated".into(), json!(n));
    }
    let mut max_mbps = None;
    if let Some(m) = max {
        if let Some((label, mbps)) = usb_speed_label(&m) {
            link.insert("max_label".into(), json!(label));
            link.insert("max_mbps".into(), json!(mbps));
            max_mbps = Some(mbps);
        }
        link.insert("max".into(), json!(m));
    }
    if let (Some(n), Some(mx)) = (neg_mbps, max_mbps) {
        link.insert("at_full_speed".into(), json!(n >= mx));
    }
    Some(Value::Object(link))
}

pub fn usb_status(_state: &AppState) -> (u16, Value) {
    let mut payload = match ubus::call("zwrt_bsp.usb", "list", Some("{}")) {
        Ok(Value::Object(m)) => m,
        Ok(_) => Map::new(),
        Err(e) => return (503, json!({"ok": false, "error": e})),
    };
    let ncm_available = Path::new(NCM_FUNC).exists();
    let bridge_members = bridge_members("br-lan");
    let persisted = read_persisted_config();
    let default_mode = parse_usb_default_mode(&persisted).unwrap_or("ecm");
    payload.insert("active_mode".into(), json!(detect_active_usb_mode()));
    payload.insert("default_mode".into(), json!(default_mode));
    payload.insert("ncm_persist_on_boot".into(), json!(default_mode == "ncm"));
    payload.insert("supported_modes".into(), json!(supported_modes()));
    payload.insert(
        "experimental_modes".into(),
        json!(if ncm_available {
            vec!["ncm"]
        } else {
            Vec::<&str>::new()
        }),
    );
    payload.insert(
        "mode_capabilities".into(),
        json!([
            {
                "mode": "rndis",
                "supported": Path::new(RNDIS_GSI_FUNC).exists(),
                "experimental": false,
                "function": "gsi.rndis"
            },
            {
                "mode": "ecm",
                "supported": Path::new(ECM_GSI_FUNC).exists(),
                "experimental": false,
                "function": "gsi.ecm"
            },
            {
                "mode": "ncm",
                "supported": ncm_available,
                "experimental": true,
                "function": "ncm.0",
                "note": "configfs NCM exists, but ZTE's ubus USB switch does not expose it"
            }
        ]),
    );
    payload.insert(
        "configfs".into(),
        json!({
            "present": Path::new(GADGET_DIR).exists(),
            "ncm": ncm_available,
            "gsi_ecm": Path::new(ECM_GSI_FUNC).exists(),
            "gsi_rndis": Path::new(RNDIS_GSI_FUNC).exists(),
        }),
    );
    payload.insert(
        "composition_functions".into(),
        json!(current_composition_functions()),
    );
    payload.insert(
        "bridge".into(),
        json!({"name": "br-lan", "members": bridge_members}),
    );
    payload.insert(
        "interfaces".into(),
        json!({
            "ecm0": Path::new("/sys/class/net/ecm0").exists(),
            "rndis0": Path::new("/sys/class/net/rndis0").exists(),
            "ncm0": Path::new("/sys/class/net/ncm0").exists(),
            "ncm_ifname": if detect_active_usb_mode() == Some("ncm") {
                ncm_interface_name()
            } else {
                None
            },
        }),
    );
    payload.insert(
        "usb_ids".into(),
        json!({
            "vendor": read_trimmed(&format!("{GADGET_DIR}/idVendor")),
            "product": read_trimmed(&format!("{GADGET_DIR}/idProduct")),
        }),
    );
    if let Some(last_error) = read_trimmed(NCM_LAST_ERROR_PATH) {
        payload.insert("ncm_last_error".into(), json!(last_error));
    }
    if let Some(link) = usb_link_info() {
        payload.insert("link".into(), link);
    }
    (200, json!({"ok": true, "data": payload}))
}

pub fn usb_mode_set(_state: &AppState, body: &[u8]) -> (u16, Value) {
    let parsed: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({"ok": false, "error": "invalid JSON"})),
    };
    let mode = match parsed["mode"].as_str() {
        Some(v) => v,
        None => return (400, json!({"ok": false, "error": "mode is required"})),
    };
    if mode == "ncm" {
        let confirmed = parsed["confirm_experimental"].as_bool().unwrap_or(false);
        if !confirmed {
            return (
                400,
                json!({
                    "ok": false,
                    "error": "NCM is experimental and disrupts USB. Retry with confirm_experimental=true from a Wi-Fi management path."
                }),
            );
        }
        if let Err(e) = preflight_ncm_switch() {
            return (400, json!({"ok": false, "error": e}));
        }
        schedule_ncm_switch();
        return (
            202,
            json!({
                "ok": true,
                "data": {
                    "status": "scheduled",
                    "mode": "ncm",
                    "experimental": true,
                    "delay_ms": 1000,
                    "rollback": "reboot or switch back to ECM after reconnecting"
                }
            }),
        );
    }
    if mode == "ecm" && current_composition_functions().iter().any(|f| f == "ncm.0") {
        if let Err(e) = preflight_ecm_switch() {
            return (400, json!({"ok": false, "error": e}));
        }
        schedule_ecm_switch();
        return (
            202,
            json!({
                "ok": true,
                "data": {
                    "status": "scheduled",
                    "mode": "ecm",
                    "delay_ms": 1000
                }
            }),
        );
    }

    match ubus::call("zwrt_bsp.usb", "set", Some(&parsed.to_string())) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

pub fn usb_default_set(_state: &AppState, body: &[u8]) -> (u16, Value) {
    let parsed: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({"ok": false, "error": "invalid JSON"})),
    };
    let mode = match parsed["mode"].as_str() {
        Some("ecm") => "ecm",
        Some("ncm") => {
            let confirmed = parsed["confirm_experimental"].as_bool().unwrap_or(false);
            if !confirmed {
                return (
                    400,
                    json!({
                        "ok": false,
                        "error": "NCM persistence is experimental. Retry with confirm_experimental=true from a Wi-Fi management path."
                    }),
                );
            }
            if let Err(e) = preflight_ncm_switch() {
                return (400, json!({"ok": false, "error": e}));
            }
            "ncm"
        }
        Some(_) => {
            return (
                400,
                json!({"ok": false, "error": "mode must be ecm or ncm"}),
            )
        }
        None => return (400, json!({"ok": false, "error": "mode is required"})),
    };

    match set_usb_default_mode(mode) {
        Ok(()) => (
            200,
            json!({
                "ok": true,
                "data": {
                    "default_mode": mode,
                    "ncm_persist_on_boot": mode == "ncm"
                }
            }),
        ),
        Err(e) => (500, json!({"ok": false, "error": e})),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_usb_default_mode;
    use serde_json::json;

    #[test]
    fn usb_default_mode_accepts_supported_modes() {
        assert_eq!(
            parse_usb_default_mode(&json!({"usb_default_mode": "ncm"})),
            Some("ncm")
        );
        assert_eq!(
            parse_usb_default_mode(&json!({"usb_default_mode": "ECM"})),
            Some("ecm")
        );
    }

    #[test]
    fn usb_default_mode_ignores_unknown_values() {
        assert_eq!(parse_usb_default_mode(&json!({})), None);
        assert_eq!(
            parse_usb_default_mode(&json!({"usb_default_mode": "rndis"})),
            None
        );
        assert_eq!(
            parse_usb_default_mode(&json!({"usb_default_mode": true})),
            None
        );
    }

    #[test]
    fn usb_speed_label_maps_known_speeds() {
        assert_eq!(
            super::usb_speed_label("high-speed"),
            Some(("USB 2.0", 480.0))
        );
        assert_eq!(
            super::usb_speed_label("super-speed-plus"),
            Some(("USB 3.1 Gen2", 10000.0))
        );
        assert_eq!(super::usb_speed_label("UNKNOWN"), None);
    }

    #[test]
    fn power_off_charging_matches_stock_guard() {
        assert!(super::is_power_off_charging_state(Some(
            "mode_power_off_charger"
        )));
        assert!(super::is_power_off_charging_state(Some(
            "mode_power_off_unreal"
        )));
        assert!(!super::is_power_off_charging_state(Some(
            "mode_power_on_charger"
        )));
        assert!(!super::is_power_off_charging_state(Some("mode_power_on")));
        assert!(!super::is_power_off_charging_state(None));
    }
}

fn preflight_ncm_switch() -> Result<(), String> {
    if !Path::new(GADGET_DIR).exists() {
        return Err("USB gadget configfs is not mounted".into());
    }
    if !Path::new(NCM_FUNC).exists() {
        return Err("NCM configfs function is not available on this firmware".into());
    }
    if !Path::new(MASS_STORAGE_FUNC).exists() {
        return Err(
            "mass_storage.0 function is missing; refusing to build partial composition".into(),
        );
    }
    if first_udc_name().is_none() && read_trimmed(UDC_PATH).is_none() {
        return Err("no USB device controller is available".into());
    }
    Ok(())
}

fn preflight_ecm_switch() -> Result<(), String> {
    if !Path::new(GADGET_DIR).exists() {
        return Err("USB gadget configfs is not mounted".into());
    }
    if !Path::new(ECM_GSI_FUNC).exists() {
        return Err("GSI ECM configfs function is not available on this firmware".into());
    }
    if !Path::new(MASS_STORAGE_FUNC).exists() {
        return Err(
            "mass_storage.0 function is missing; refusing to build partial composition".into(),
        );
    }
    if first_udc_name().is_none() && read_trimmed(UDC_PATH).is_none() {
        return Err("no USB device controller is available".into());
    }
    Ok(())
}

fn schedule_ncm_switch() {
    thread::spawn(|| {
        thread::sleep(Duration::from_millis(1000));
        let result = switch_to_ncm_now();
        match result {
            Ok(()) => {
                let _ = fs::remove_file(NCM_LAST_ERROR_PATH);
            }
            Err(e) => {
                let _ = fs::write(NCM_LAST_ERROR_PATH, e);
            }
        }
    });
}

fn schedule_ecm_switch() {
    thread::spawn(|| {
        thread::sleep(Duration::from_millis(1000));
        let result = switch_to_ecm_now();
        match result {
            Ok(()) => {
                let _ = fs::remove_file(NCM_LAST_ERROR_PATH);
            }
            Err(e) => {
                let _ = fs::write(NCM_LAST_ERROR_PATH, e);
            }
        }
    });
}

fn switch_to_ncm_now() -> Result<(), String> {
    preflight_ncm_switch()?;
    let udc = read_trimmed(UDC_PATH)
        .or_else(first_udc_name)
        .unwrap_or_default();

    fs::write(UDC_PATH, "").map_err(|e| format!("unbind UDC: {e}"))?;
    remove_config_links()?;

    write_if_exists(&format!("{GADGET_DIR}/idVendor"), "0x19d2")?;
    write_if_exists(&format!("{GADGET_DIR}/idProduct"), "0x1406")?;
    write_if_exists(
        &format!("{GADGET_DIR}/strings/0x409/product"),
        "ZTE Mobile Broadband",
    )?;
    write_if_exists(
        &format!("{CONFIG_DIR}/strings/0x409/configuration"),
        "NCM_MASS_STORAGE",
    )?;
    write_if_exists(&format!("{GADGET_DIR}/bDeviceClass"), "0x02")?;
    write_if_exists(&format!("{GADGET_DIR}/os_desc/use"), "1")?;
    write_if_exists(&format!("{GADGET_DIR}/os_desc/b_vendor_code"), "0x04")?;
    write_if_exists(&format!("{GADGET_DIR}/os_desc/qw_sign"), "MSFT100")?;

    unix_fs::symlink(NCM_FUNC, format!("{CONFIG_DIR}/f1"))
        .map_err(|e| format!("link ncm.0: {e}"))?;
    unix_fs::symlink(MASS_STORAGE_FUNC, format!("{CONFIG_DIR}/f2"))
        .map_err(|e| format!("link mass_storage.0: {e}"))?;

    fs::write(UDC_PATH, &udc).map_err(|e| format!("bind UDC {udc}: {e}"))?;
    let ifname = wait_for_ncm_interface(Duration::from_secs(10))
        .ok_or_else(|| "NCM interface did not appear after binding".to_string())?;
    run_command("ifconfig", &[&ifname, "up"])?;
    add_bridge_member("br-lan", &ifname)?;
    Ok(())
}

fn switch_to_ecm_now() -> Result<(), String> {
    preflight_ecm_switch()?;
    let udc = read_trimmed(UDC_PATH)
        .or_else(first_udc_name)
        .unwrap_or_default();
    let ncm_ifaces = ncm_bridge_candidates();

    for ifname in &ncm_ifaces {
        let _ = remove_bridge_member("br-lan", ifname);
    }

    fs::write(UDC_PATH, "").map_err(|e| format!("unbind UDC: {e}"))?;
    remove_config_links()?;

    write_if_exists(&format!("{GADGET_DIR}/idVendor"), "0x19d2")?;
    write_if_exists(&format!("{GADGET_DIR}/idProduct"), "0x1405")?;
    write_if_exists(
        &format!("{GADGET_DIR}/strings/0x409/product"),
        "ZTE Mobile Broadband",
    )?;
    write_if_exists(
        &format!("{CONFIG_DIR}/strings/0x409/configuration"),
        "ECM_MASS_STORAGE",
    )?;
    write_if_exists(&format!("{GADGET_DIR}/bDeviceClass"), "0x02")?;
    write_if_exists(&format!("{GADGET_DIR}/os_desc/use"), "1")?;
    write_if_exists(&format!("{GADGET_DIR}/os_desc/b_vendor_code"), "0x04")?;
    write_if_exists(&format!("{GADGET_DIR}/os_desc/qw_sign"), "MSFT100")?;

    unix_fs::symlink(ECM_GSI_FUNC, format!("{CONFIG_DIR}/f1"))
        .map_err(|e| format!("link gsi.ecm: {e}"))?;
    unix_fs::symlink(MASS_STORAGE_FUNC, format!("{CONFIG_DIR}/f2"))
        .map_err(|e| format!("link mass_storage.0: {e}"))?;

    fs::write(UDC_PATH, &udc).map_err(|e| format!("bind UDC {udc}: {e}"))?;
    wait_for_interface("ecm0", Duration::from_secs(10))
        .ok_or_else(|| "ECM interface did not appear after binding".to_string())?;
    run_command("ifconfig", &["ecm0", "up"])?;
    add_bridge_member("br-lan", "ecm0")?;
    for ifname in ncm_ifaces {
        let _ = remove_bridge_member("br-lan", &ifname);
    }
    Ok(())
}

fn first_udc_name() -> Option<String> {
    fs::read_dir("/sys/class/udc")
        .ok()?
        .flatten()
        .find_map(|entry| entry.file_name().to_str().map(|s| s.to_string()))
}

fn write_if_exists(path: &str, value: &str) -> Result<(), String> {
    if Path::new(path).exists() {
        fs::write(path, value).map_err(|e| format!("write {path}: {e}"))?;
    }
    Ok(())
}

fn remove_config_links() -> Result<(), String> {
    let entries = fs::read_dir(CONFIG_DIR).map_err(|e| format!("read {CONFIG_DIR}: {e}"))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('f') && fs::read_link(entry.path()).is_ok() {
            fs::remove_file(entry.path()).map_err(|e| format!("remove {name}: {e}"))?;
        }
    }
    Ok(())
}

fn wait_for_ncm_interface(timeout: Duration) -> Option<String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(ifname) = ncm_interface_name() {
            return Some(ifname);
        }
        if Path::new("/sys/class/net/ncm0").exists() {
            return Some("ncm0".into());
        }
        if current_composition_functions().iter().any(|f| f == "ncm.0")
            && Path::new("/sys/class/net/usb0").exists()
        {
            return Some("usb0".into());
        }
        thread::sleep(Duration::from_millis(250));
    }
    None
}

fn ncm_bridge_candidates() -> Vec<String> {
    let mut candidates = Vec::new();
    if let Some(ifname) = read_trimmed(&format!("{NCM_FUNC}/ifname")) {
        if ifname != "(unnamed net_device)" {
            candidates.push(ifname);
        }
    }
    candidates.push("ncm0".into());
    candidates.push("usb0".into());
    candidates.sort();
    candidates.dedup();
    candidates
}

fn wait_for_interface(ifname: &str, timeout: Duration) -> Option<String> {
    let deadline = Instant::now() + timeout;
    let path = format!("/sys/class/net/{ifname}");
    while Instant::now() < deadline {
        if Path::new(&path).exists() {
            return Some(ifname.to_string());
        }
        thread::sleep(Duration::from_millis(250));
    }
    None
}

fn add_bridge_member(bridge: &str, ifname: &str) -> Result<(), String> {
    if bridge_members(bridge).iter().any(|member| member == ifname) {
        return Ok(());
    }
    run_command("brctl", &["addif", bridge, ifname])
}

fn remove_bridge_member(bridge: &str, ifname: &str) -> Result<(), String> {
    if !bridge_members(bridge).iter().any(|member| member == ifname) {
        return Ok(());
    }
    run_command("brctl", &["delif", bridge, ifname])
}

fn run_command(command: &str, args: &[&str]) -> Result<(), String> {
    let output = Command::new(command)
        .args(args)
        .output()
        .map_err(|e| format!("{command} exec: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "{command} {} failed: {}",
        args.join(" "),
        stderr.trim()
    ))
}
