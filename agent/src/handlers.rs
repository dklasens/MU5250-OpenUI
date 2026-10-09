use std::sync::Arc;

use serde_json::{json, Value};

use std::time::Duration;

use crate::at_cmd::AtPort;
use crate::auth::{self, AuthState};
use crate::cache::{Cached, Observed, Sample};
use crate::charge_policy::ChargeLimitEnforcer;
use crate::connection_logger::ConnectionLogger;
use crate::signal_logger::SignalLogger;
use crate::system::{self, CpuTracker, FirmwareInfo, ProcessTracker, SpeedTracker};
use crate::ubus;

// Per-source freshness for the dashboard batch. Every entry below costs a
// fork+exec, so the client's poll rate is deliberately decoupled from the rate
// at which each source is actually re-read — and concurrent clients (phone plus
// laptop) collapse onto one refresh instead of multiplying the load.
const SIGNAL_TTL: Duration = Duration::from_secs(1);
const THERMAL_TTL: Duration = Duration::from_secs(10);
const WAN_TTL: Duration = Duration::from_secs(30);
const DATA_USAGE_TTL: Duration = Duration::from_secs(30);
const BEARER_TTL: Duration = Duration::from_secs(10);
/// Billing-cycle dates move once a month.
const CYCLE_DATE_TTL: Duration = Duration::from_secs(300);

#[derive(Default)]
pub struct DashboardCache {
    wan: Observed<Value>,
    wan6: Observed<Value>,
    thermal: Observed<Value>,
    data_usage: Observed<Value>,
    bearer: Cached<Value>,
    cycle_dates: Cached<(Option<String>, Option<String>)>,
}

pub struct AppState {
    pub binding: Arc<crate::lan::Binding>,
    pub lan: Arc<crate::lan::LanManager>,
    pub auth: AuthState,
    pub cpu: CpuTracker,
    pub speed: SpeedTracker,
    pub firmware: FirmwareInfo,
    pub proc_tracker: ProcessTracker,
    pub at_port: AtPort,
    pub dash: DashboardCache,
    pub radio: Arc<Observed<Value>>,
    pub charge_limit: Arc<ChargeLimitEnforcer>,
    pub signal_logger: Arc<SignalLogger>,
    pub connection_logger: Arc<ConnectionLogger>,
}

impl AppState {
    pub fn new() -> Self {
        let binding = Arc::new(crate::lan::Binding::new());
        let lan = Arc::new(crate::lan::LanManager::new(binding.clone()));
        Self {
            binding,
            lan,
            auth: AuthState::new(),
            cpu: CpuTracker::new(),
            speed: SpeedTracker::new(),
            firmware: FirmwareInfo::new(),
            proc_tracker: ProcessTracker::new(),
            at_port: AtPort::new(),
            dash: DashboardCache::default(),
            radio: Arc::new(Observed::default()),
            charge_limit: Arc::new(ChargeLimitEnforcer::new()),
            signal_logger: Arc::new(SignalLogger::new()),
            connection_logger: Arc::new(ConnectionLogger::new()),
        }
    }
}

/// POST /api/auth/login — body: {"password": "..."} or {"pin": "..."}
pub fn login(
    state: &AppState,
    body: &[u8],
    client_ip: &str,
    user_agent: Option<&str>,
) -> (u16, Value) {
    let parsed: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({"ok": false, "error": "invalid JSON"})),
    };

    let password = parsed["password"].as_str();
    let pin = parsed["pin"].as_str();
    if password.is_some() && pin.is_some() {
        return (
            400,
            json!({"ok": false, "error": "provide either password or pin"}),
        );
    }

    let result = if let Some(password) = password {
        state.auth.login_password(password, client_ip)
    } else if let Some(pin) = pin {
        if !is_mobile_user_agent(user_agent.unwrap_or_default()) {
            return (
                403,
                json!({"ok": false, "error": "PIN login is only available from mobile devices"}),
            );
        }
        state.auth.login_pin(pin, client_ip)
    } else {
        return (
            400,
            json!({"ok": false, "error": "missing 'password' or 'pin' field"}),
        );
    };

    match result {
        auth::LoginResult::Ok { token } => (200, json!({"ok": true, "data": {"token": token}})),
        auth::LoginResult::Invalid => (401, json!({"ok": false, "error": "invalid credentials"})),
        auth::LoginResult::Locked { retry_after_secs } => (
            429,
            json!({"ok": false, "error": format!("too many attempts, retry in {retry_after_secs}s")}),
        ),
    }
}

fn is_mobile_user_agent(user_agent: &str) -> bool {
    let ua = user_agent.to_ascii_lowercase();
    ua.contains("mobile")
        || ua.contains("android")
        || ua.contains("iphone")
        || ua.contains("ipad")
        || ua.contains("ipod")
        || ua.contains("blackberry")
        || ua.contains("iemobile")
        || ua.contains("opera mini")
}

/// GET /api/device
pub fn device(state: &AppState) -> (u16, Value) {
    let mut data = device_json(state);
    data["auth"] = json!({"pin_enabled": state.auth.has_pin()});
    (200, json!({"ok": true, "data": data}))
}

/// Shared by `/api/device` and the dashboard batch. `kernel` is the raw
/// `/proc/version`; `firmware` is the ZTE build, which is what users mean.
fn device_json(state: &AppState) -> Value {
    let info = system::read_device_info();
    let firmware = state.firmware.get();
    json!({
        "hostname": info.hostname,
        "uptime_secs": info.uptime_secs,
        "load_avg": info.load_avg,
        "kernel": info.kernel,
        "firmware": firmware.as_ref().and_then(|f| f.version.clone()),
        "hardware": firmware.and_then(|f| f.hardware),
    })
}

/// GET /api/cpu
pub fn cpu(state: &AppState) -> (u16, Value) {
    let usage = state.cpu.sample();
    (200, json!({"ok": true, "data": usage}))
}

/// GET /api/memory
pub fn memory(_state: &AppState) -> (u16, Value) {
    match system::read_meminfo() {
        Some(m) => (200, json!({"ok": true, "data": m})),
        None => (
            503,
            json!({"ok": false, "error": "memory info not available"}),
        ),
    }
}

/// PUT /api/data-usage/reset-day
pub fn data_usage_reset_day_set(state: &AppState, body: &[u8]) -> (u16, Value) {
    let parsed: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({"ok": false, "error": "invalid JSON"})),
    };
    let day = parsed["reset_day"]
        .as_u64()
        .or_else(|| parsed["clearday"].as_u64())
        .unwrap_or(0);
    if !(1..=31).contains(&day) {
        return (
            400,
            json!({"ok": false, "error": "reset_day must be between 1 and 31"}),
        );
    }

    let params = json!({
        "source_module": "web",
        "cid": 1,
        "type": 4,
        "enable": 1,
        "clearday": day,
    });

    match ubus::call(
        "zwrt_data",
        "set_wwandst_clearday",
        Some(&params.to_string()),
    ) {
        // We just changed the cycle, so the cached copies are wrong by definition.
        Ok(_) => {
            state.dash.data_usage.invalidate();
            state.dash.cycle_dates.invalidate();
            match read_data_usage_live(&state.dash) {
                Ok(data) => (200, json!({"ok": true, "data": data})),
                Err(e) => (503, json!({"ok": false, "error": e})),
            }
        }
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

fn read_data_usage_live(cache: &DashboardCache) -> Result<Value, String> {
    let stats = ubus::call(
        "zwrt_data",
        "get_wwandst",
        Some(r#"{"source_module":"web","cid":1,"type":4}"#),
    )?;
    let clear = ubus::call(
        "zwrt_data",
        "get_wwandst_clearday",
        Some(r#"{"source_module":"web","cid":1,"type":4}"#),
    )
    .unwrap_or_else(|_| json!({}));

    let section = "zwrt_data_commit.wwancid1dst";

    let read_stat_period = |prefix: &str| -> Value {
        let get = |suffix: &str| -> Value {
            let key = format!("{prefix}_{suffix}");
            number_value(stats.get(&key)).unwrap_or(Value::Null)
        };
        json!({
            "tx_bytes": get("tx_bytes"),
            "rx_bytes": get("rx_bytes"),
            "time_secs": get("time"),
            "tx_packets": get("tx_packets"),
            "rx_packets": get("rx_packets"),
        })
    };

    let uci = |key: &str| ubus::uci_get(&format!("{section}.{key}")).ok();
    let (reset_day, reset_enabled) =
        reset_config(&clear, || uci("clearday"), || uci("clearday_enable"));

    // These two are billing-cycle dates: they change once a month, but used to
    // cost two `uci get` forks on every dashboard poll.
    let (clear_date_record, next_clear_date) =
        cache.cycle_dates.get_or_refresh(CYCLE_DATE_TTL, || {
            (
                ubus::uci_get(&format!("{section}.clear_date_record")).ok(),
                ubus::uci_get(&format!("{section}.clearday_date")).ok(),
            )
        });

    Ok(json!({
        "day": read_stat_period("day"),
        "month": read_stat_period("month"),
        "cycle": read_stat_period("month"),
        "since_power_on": read_stat_period("real"),
        "total": read_stat_period("total"),
        "reset_day": reset_day,
        "reset_enabled": reset_enabled,
        "clear_date_record": clear_date_record,
        "next_clear_date": next_clear_date,
    }))
}

/// Reset-cycle configuration from the ubus reply, falling back to the persisted
/// UCI values. A field neither source can supply is `null`: the old "day 1,
/// disabled" default told the dashboard something the device never said.
fn reset_config(
    clear: &Value,
    uci_day: impl FnOnce() -> Option<String>,
    uci_enabled: impl FnOnce() -> Option<String>,
) -> (Value, Value) {
    let day = |v: Option<Value>| v.filter(|v| v.as_u64().is_some_and(|d| (1..=31).contains(&d)));
    let enabled = |v: Option<Value>| v.filter(|v| matches!(v.as_u64(), Some(0 | 1)));
    let from_uci = |v: Option<String>| number_value(v.map(Value::from).as_ref());

    let reset_day = day(number_value(clear.get("clearday")))
        .or_else(|| day(from_uci(uci_day())))
        .unwrap_or(Value::Null);
    let reset_enabled = enabled(number_value(clear.get("enable")))
        .or_else(|| enabled(from_uci(uci_enabled())))
        .unwrap_or(Value::Null);
    (reset_day, reset_enabled)
}

fn number_value(value: Option<&Value>) -> Option<Value> {
    match value? {
        Value::Number(n) => Some(Value::Number(n.clone())),
        Value::String(s) => s.parse::<u64>().ok().map(Value::from),
        _ => None,
    }
}

/// GET /api/system/top
pub fn system_top(state: &AppState) -> (u16, Value) {
    let result = state.proc_tracker.sample();
    (200, json!({"ok": true, "data": result}))
}

/// POST /api/system/kill-bloat — body: {"all": true} or {"pids": [1, 2, 3]}
pub fn system_kill_bloat(_state: &AppState, body: &[u8]) -> (u16, Value) {
    let parsed: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({"ok": false, "error": "invalid JSON"})),
    };

    let pids: Option<Vec<u32>> = if parsed["all"].as_bool() == Some(true) {
        None
    } else if let Some(arr) = parsed["pids"].as_array() {
        let ids: Vec<u32> = arr
            .iter()
            .filter_map(|v| v.as_u64().map(|n| n as u32))
            .collect();
        if ids.is_empty() {
            return (400, json!({"ok": false, "error": "pids array is empty"}));
        }
        Some(ids)
    } else {
        return (
            400,
            json!({"ok": false, "error": "expected 'all' or 'pids'"}),
        );
    };

    match system::kill_bloat(pids.as_deref()) {
        Ok(result) => (200, json!({"ok": true, "data": result})),
        Err(error) => (503, json!({"ok": false, "error": error})),
    }
}

/// GET /api/dashboard — batch endpoint aggregating all dashboard data.
///
/// This is the app's heartbeat: Home, Signal and Modem/Data all read it rather
/// than polling their own endpoints. Free sources (procfs/sysfs) are read every
/// time; each ubus/uci source sits behind its own TTL.
pub fn dashboard(state: &AppState) -> (u16, Value) {
    let cache = &state.dash;

    // procfs / sysfs — no subprocess, read fresh every time. The firmware
    // identity inside `device_json` is read once and kept.
    let device = device_json(state);
    let battery = system::read_battery();
    let cpu_usage = state.cpu.sample();
    let meminfo = system::read_meminfo();
    let speed = state.speed.sample();

    // Raw ubus passthroughs — the dashboard client maps these itself.
    let signal = read_radio(&state.radio);
    let wan = cache.wan.read(WAN_TTL, || {
        ubus::call("network.interface.zte_wan", "status", Some("{}"))
    });
    let wan6 = cache.wan6.read(WAN_TTL, || {
        ubus::call("network.interface.zte_wan6", "status", Some("{}"))
    });
    let thermal = cache.thermal.read(THERMAL_TTL, || {
        ubus::call("zwrt_bsp.thermal", "get_cpu_temp", Some("{}"))
    });
    let data_usage = cache
        .data_usage
        .read(DATA_USAGE_TTL, || read_data_usage_live(cache));
    let bearer = cache.bearer.get_or_refresh(BEARER_TTL, crate::bearer::read);

    let mut result = serde_json::Map::new();
    result.insert("device".into(), device);
    result.insert("battery".into(), json!(battery));
    result.insert("cpu".into(), json!(cpu_usage));
    result.insert("memory".into(), json!(meminfo));
    result.insert(
        "speed".into(),
        speed.value.map(|v| json!(v)).unwrap_or(Value::Null),
    );
    result.insert("data_usage".into(), data_usage.value.unwrap_or(Value::Null));
    result.insert("signal".into(), signal.value.unwrap_or(Value::Null));
    result.insert("bearer".into(), bearer);
    result.insert("wan".into(), wan.value.unwrap_or(Value::Null));
    result.insert("wan6".into(), wan6.value.unwrap_or(Value::Null));
    result.insert("thermal".into(), thermal.value.unwrap_or(Value::Null));
    result.insert(
        "sources".into(),
        json!({
            "signal": signal.freshness, "wan": wan.freshness, "wan6": wan6.freshness,
            "thermal": thermal.freshness, "data_usage": data_usage.freshness,
            "speed": speed.freshness,
        }),
    );
    result.insert(
        "charge_control_error".into(),
        json!(state.charge_limit.last_error()),
    );
    (200, json!({"ok": true, "data": result}))
}

pub fn read_radio(source: &Observed<Value>) -> Sample<Value> {
    source.read(SIGNAL_TTL, || {
        ubus::call("zte_nwinfo_api", "nwinfo_get_netinfo", Some("{}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> Option<String> {
        None
    }

    #[test]
    fn reset_config_prefers_ubus_values() {
        let clear = json!({"cid": 1, "enable": 1, "clearday": 16});
        let uci_unused = || -> Option<String> { panic!("UCI read despite a ubus value") };
        assert_eq!(
            reset_config(&clear, uci_unused, uci_unused),
            (json!(16), json!(1))
        );
    }

    #[test]
    fn reset_config_falls_back_to_uci() {
        let (day, enabled) = reset_config(&json!({}), || Some("16".into()), || Some("0".into()));
        assert_eq!((day, enabled), (json!(16), json!(0)));
    }

    #[test]
    fn reset_config_is_unknown_when_both_sources_fail() {
        assert_eq!(
            reset_config(&json!({}), none, none),
            (Value::Null, Value::Null)
        );
    }

    #[test]
    fn reset_config_rejects_out_of_range_values() {
        let clear = json!({"enable": 2, "clearday": 0});
        let (day, enabled) = reset_config(&clear, || Some("32".into()), || Some("x".into()));
        assert_eq!((day, enabled), (Value::Null, Value::Null));
        let (day, _) = reset_config(&json!({"clearday": "31"}), none, none);
        assert_eq!(day, json!(31));
    }
}
