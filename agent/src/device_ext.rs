use std::fs;

use serde_json::{json, Value};

use crate::handlers::AppState;
use crate::ubus;
use crate::validate::validate_ubus_input;

pub fn device_thermal(_state: &AppState) -> (u16, Value) {
    match ubus::call("zwrt_bsp.thermal", "get_cpu_temp", Some("{}")) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

/// GET /api/device/thermal/all — read all useful thermal zones from sysfs
pub fn device_thermal_all(_state: &AppState) -> (u16, Value) {
    let zones: &[(&str, &str)] = &[
        ("cpu_0", "/sys/class/thermal/thermal_zone16/temp"),
        ("cpu_1", "/sys/class/thermal/thermal_zone17/temp"),
        ("cpu_2", "/sys/class/thermal/thermal_zone18/temp"),
        ("cpu_3", "/sys/class/thermal/thermal_zone19/temp"),
        ("modem", "/sys/class/thermal/thermal_zone22/temp"), // mdmq6-0
        ("modem_ss0", "/sys/class/thermal/thermal_zone24/temp"), // mdmss-0
        ("modem_ss1", "/sys/class/thermal/thermal_zone25/temp"), // mdmss-1
        ("modem_ss2", "/sys/class/thermal/thermal_zone26/temp"), // mdmss-2
        ("battery", "/sys/class/thermal/thermal_zone39/temp"),
        ("usb", "/sys/class/thermal/thermal_zone38/temp"),
        ("eth_phy", "/sys/class/thermal/thermal_zone20/temp"), // ethphy-0
        ("pmic", "/sys/class/thermal/thermal_zone28/temp"),    // pmx75_tz
        ("xo_therm", "/sys/class/thermal/thermal_zone35/temp"), // crystal osc (ambient proxy)
        ("pa", "/sys/class/thermal/thermal_zone0/temp"),       // sdr0_pa
        ("sdr", "/sys/class/thermal/thermal_zone1/temp"),      // sdr0
    ];

    let mut data = serde_json::Map::new();
    for (name, path) in zones {
        if let Ok(s) = fs::read_to_string(path) {
            if let Ok(millideg) = s.trim().parse::<i64>() {
                // Skip invalid readings: -273000 = sensor offline, valid range -40°C to +150°C
                if millideg > -40_000 && millideg < 150_000 {
                    let temp_c = millideg as f64 / 1000.0;
                    data.insert(name.to_string(), json!(temp_c));
                }
            }
        }
    }
    (200, json!({"ok": true, "data": data}))
}

/// GET /api/device/battery/detail — extended battery stats from sysfs
pub fn device_battery_detail(_state: &AppState) -> (u16, Value) {
    let read_sysfs = |name: &str| -> Option<String> {
        fs::read_to_string(format!("/sys/class/power_supply/battery/{name}"))
            .ok()
            .map(|s| s.trim().to_string())
    };
    let read_i64 = |name: &str| -> Option<i64> { read_sysfs(name)?.parse().ok() };

    let capacity = read_i64("capacity").unwrap_or(0);
    let status = read_sysfs("status").unwrap_or_default();
    let voltage_uv = read_i64("voltage_now").unwrap_or(0);
    let voltage_max_uv = read_i64("voltage_max").unwrap_or(0);
    let voltage_ocv_uv = read_i64("voltage_ocv").unwrap_or(0);
    let current_ua = read_i64("current_now").unwrap_or(0);
    // power_now sysfs is unreliable on PM7550B — compute from V * I instead
    let _ = read_i64("power_now"); // ignore sysfs value
    let temp_tenths = read_i64("temp").unwrap_or(0);
    let charge_type = read_sysfs("charge_type").unwrap_or_default();
    let health = read_sysfs("health").unwrap_or_default();
    let cycle_count = read_i64("cycle_count").unwrap_or(0);
    let charge_counter_uah = read_i64("charge_counter").unwrap_or(0);
    let charge_full_uah = read_i64("charge_full").unwrap_or(0);
    let charge_full_design_uah = read_i64("charge_full_design").unwrap_or(0);
    let time_to_full = read_i64("time_to_full_avg").unwrap_or(-1);
    let time_to_empty = read_i64("time_to_empty_avg").unwrap_or(-1);

    // Compute power from voltage * current (more accurate than sysfs power_now)
    let power_mw = (voltage_uv as f64 * current_ua as f64 / 1e9) as i64;

    (
        200,
        json!({"ok": true, "data": {
            "capacity": capacity,
            "status": status,
            "voltage_mv": voltage_uv / 1000,
            "voltage_max_mv": voltage_max_uv / 1000,
            "voltage_ocv_mv": voltage_ocv_uv / 1000,
            "current_ma": current_ua / 1000,
            "power_mw": power_mw,
            "temperature_c": temp_tenths as f64 / 10.0,
            "charge_type": charge_type,
            "health": health,
            "cycle_count": cycle_count,
            "charge_counter_mah": charge_counter_uah / 1000,
            "charge_full_mah": charge_full_uah / 1000,
            "charge_full_design_mah": charge_full_design_uah / 1000,
            "time_to_full_secs": time_to_full,
            "time_to_empty_secs": time_to_empty,
        }}),
    )
}

pub fn device_charger(_state: &AppState) -> (u16, Value) {
    match ubus::call("zwrt_bsp.charger", "list", Some("{}")) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

pub fn device_system(_state: &AppState) -> (u16, Value) {
    match ubus::call("system", "info", Some("{}")) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

pub fn device_reboot(_state: &AppState) -> (u16, Value) {
    match ubus::call("system", "reboot", Some("{}")) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

pub fn device_shutdown(_state: &AppState) -> (u16, Value) {
    match ubus::call(
        "zwrt_mc.device.manager",
        "device_poweroff",
        Some(r#"{"moduleName":"zte-agent"}"#),
    ) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

pub fn agent_restart(_state: &AppState) -> (u16, Value) {
    // Spawn a detached process that waits, then kills and restarts the agent.
    // We respond first so the client gets a 200 before we die.
    let script = "sleep 1; kill $(pidof zte-agent) 2>/dev/null; sleep 1; sh /data/local/tmp/start_zte_agent.sh >/dev/null 2>&1 &";
    match std::process::Command::new("sh")
        .args(["-c", script])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_) => (
            200,
            json!({"ok": true, "message": "Agent restarting in ~2 seconds"}),
        ),
        Err(e) => (
            500,
            json!({"ok": false, "error": format!("failed to spawn restart: {e}")}),
        ),
    }
}

// factory_reset is ZTE-specific (zwrt_bsp.power) — may require re-enabling that daemon
pub fn device_factory_reset(_state: &AppState) -> (u16, Value) {
    match ubus::call("zwrt_bsp.power", "factory_reset", Some("{}")) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

pub fn device_power_save_get(_state: &AppState, body: &[u8]) -> (u16, Value) {
    let parsed: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({"ok": false, "error": "invalid JSON"})),
    };
    if let Err(e) = validate_ubus_input(&parsed) {
        return (400, json!({"ok": false, "error": e}));
    }
    match ubus::call(
        "zwrt_mc.device.manager",
        "get_device_info",
        Some(&parsed.to_string()),
    ) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

pub fn device_power_save_set(_state: &AppState, body: &[u8]) -> (u16, Value) {
    let parsed: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({"ok": false, "error": "invalid JSON"})),
    };
    if let Err(e) = validate_ubus_input(&parsed) {
        return (400, json!({"ok": false, "error": e}));
    }
    match ubus::call(
        "zwrt_mc.device.manager",
        "set_device_info",
        Some(&parsed.to_string()),
    ) {
        Ok(data) => (200, json!({"ok": true, "data": data})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

pub fn device_fast_boot_get(_state: &AppState) -> (u16, Value) {
    let params = r#"{"deviceInfoList":["quicken_power_on"]}"#;
    match ubus::call("zwrt_mc.device.manager", "get_device_info", Some(params)) {
        Ok(data) => {
            let val = data["quicken_power_on"].as_str().unwrap_or("0");
            (200, json!({"ok": true, "data": {"fast_boot": val}}))
        }
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

pub fn device_fast_boot_set(_state: &AppState, body: &[u8]) -> (u16, Value) {
    let parsed: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({"ok": false, "error": "invalid JSON"})),
    };
    let val = match parsed["fast_boot"].as_str() {
        Some(v @ ("0" | "1")) => v,
        _ => {
            return (
                400,
                json!({"ok": false, "error": "fast_boot must be \"0\" or \"1\""}),
            )
        }
    };
    let params = format!(r#"{{"deviceInfoList":{{"quicken_power_on":"{val}"}}}}"#);
    match ubus::call("zwrt_mc.device.manager", "set_device_info", Some(&params)) {
        Ok(_) => (200, json!({"ok": true, "data": {"fast_boot": val}})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}
