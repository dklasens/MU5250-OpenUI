use std::sync::Arc;

use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::at_cmd;
use crate::cell;
use crate::connection_logger;
use crate::device_ext;
use crate::handlers::{self, AppState};
use crate::lan_test;
use crate::modem_ext;
use crate::network_ext;
use crate::router;
use crate::scheduler;
use crate::signal_logger;
use crate::sim;
use crate::sms;
use crate::sms_forward;
use crate::speedtest;
use crate::telephony;
use crate::usb;
use crate::wifi;

pub fn start(bind: &str, threads: usize, state: Arc<AppState>) {
    let server = match Server::http(bind) {
        Ok(s) => s,
        Err(_) => {
            std::process::exit(1);
        }
    };
    // Seed the CPU tracker with initial sample (speed tracker self-seeds)
    state.cpu.sample();

    let server = Arc::new(server);
    let mut handles = Vec::new();

    for _ in 0..threads {
        let server = Arc::clone(&server);
        let state = Arc::clone(&state);
        let handle = std::thread::spawn(move || loop {
            let request = match server.recv() {
                Ok(r) => r,
                Err(_) => {
                    continue;
                }
            };
            handle_request(request, &state);
        });
        handles.push(handle);
    }

    for h in handles {
        let _ = h.join();
    }
}

const DESTRUCTIVE_PATHS: &[&str] = &[
    "/api/device/factory-reset",
    "/api/device/reboot",
    "/api/device/shutdown",
];

fn cors_headers(origin: Option<&str>) -> Vec<Header> {
    let allowed = origin
        .and_then(|o| is_lan_origin(o).then_some(o))
        .unwrap_or("");
    vec![
        Header::from_bytes("Access-Control-Allow-Origin", allowed).unwrap(),
        Header::from_bytes(
            "Access-Control-Allow-Methods",
            "GET, POST, PUT, DELETE, OPTIONS",
        )
        .unwrap(),
        Header::from_bytes(
            "Access-Control-Allow-Headers",
            "Authorization, Content-Type, X-Confirm",
        )
        .unwrap(),
        Header::from_bytes("Access-Control-Max-Age", "86400").unwrap(),
    ]
}

fn is_lan_origin(origin: &str) -> bool {
    if !origin.starts_with("http://") {
        return false;
    }
    let host = &origin[7..];
    let host = host.split(':').next().unwrap_or(host);
    if host == "localhost" || host == "127.0.0.1" || host == "::1" {
        return true;
    }
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    let octets: Vec<u8> = parts.iter().filter_map(|p| p.parse().ok()).collect();
    if octets.len() != 4 {
        return false;
    }
    octets[0] == 10
        || (octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31)
        || (octets[0] == 192 && octets[1] == 168)
}

fn handle_request(mut request: Request, state: &AppState) {
    let method = request.method().clone();
    let url = request.url().to_string();
    let path = url.split('?').next().unwrap_or(&url).to_string();
    let origin = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Origin"))
        .map(|h| h.value.as_str().to_string());
    let origin_ref = origin.as_deref();
    let client_ip = request
        .remote_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_default();
    let user_agent = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("User-Agent"))
        .map(|h| h.value.as_str().to_string());
    let user_agent_ref = user_agent.as_deref();

    if method == Method::Options {
        let mut response = Response::empty(200);
        for h in cors_headers(origin_ref) {
            response = response.with_header(h);
        }
        let _ = request.respond(response);
        return;
    }

    let needs_auth = path != "/api/auth/login";
    if needs_auth {
        let authorized = request
            .headers()
            .iter()
            .find(|h| h.field.as_str().to_ascii_lowercase() == "authorization")
            .and_then(|h| h.value.as_str().strip_prefix("Bearer "))
            .map(|token| state.auth.validate(token))
            .unwrap_or(false);

        if !state.auth.has_password() {
            respond(
                request,
                403,
                json!({"ok": false, "error": "no password configured. Set ZTE_AGENT_PASSWORD environment variable."}),
                origin_ref,
            );
            return;
        } else if !authorized {
            respond(
                request,
                401,
                json!({"ok": false, "error": "unauthorized"}),
                origin_ref,
            );
            return;
        }
    }

    if DESTRUCTIVE_PATHS.contains(&path.as_str()) {
        let confirmed = request
            .headers()
            .iter()
            .any(|h| h.field.equiv("X-Confirm") && h.value.as_str() == "true");
        if !confirmed {
            respond(
                request,
                400,
                json!({"ok": false, "error": "destructive action requires X-Confirm: true header"}),
                origin_ref,
            );
            return;
        }
    }

    match (&method, path.as_str()) {
        (&Method::Get, "/api/lan/download") => {
            let size = parse_query_usize(&url, "size").unwrap_or(50 * 1024 * 1024);
            let size = size.min(2 * 1024 * 1024 * 1024);
            lan_test::download(request, size, cors_headers(origin_ref));
            return;
        }
        (&Method::Post, "/api/lan/upload") => {
            let (status, body_json) = lan_test::upload(&mut request);
            respond(request, status, body_json, origin_ref);
            return;
        }
        _ => {}
    }

    let mut body = Vec::new();
    let mut reader = request.as_reader();
    let mut limited = std::io::Read::take(&mut reader, 1024 * 1024);
    if let Err(e) = std::io::Read::read_to_end(&mut limited, &mut body) {
        respond(
            request,
            400,
            json!({"ok": false, "error": format!("failed to read body: {e}")}),
            origin_ref,
        );
        return;
    }

    let (status, body_json) = route(&method, &path, state, &body, &client_ip, user_agent_ref);
    respond(request, status, body_json, origin_ref);
}

fn parse_query_usize(url: &str, key: &str) -> Option<usize> {
    let query = url.split('?').nth(1)?;
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        if kv.next()? == key {
            return kv.next()?.parse().ok();
        }
    }
    None
}

pub fn route(
    method: &Method,
    path: &str,
    state: &AppState,
    body: &[u8],
    client_ip: &str,
    user_agent: Option<&str>,
) -> (u16, Value) {
    match (method, path) {
        // Auth
        (&Method::Post, "/api/auth/login") => handlers::login(state, body, client_ip, user_agent),
        // Batch
        (&Method::Get, "/api/dashboard") => handlers::dashboard(state),
        // Device info (sysfs)
        (&Method::Get, "/api/device") => handlers::device(state),
        (&Method::Get, "/api/battery") => handlers::battery(state),
        (&Method::Get, "/api/cpu") => handlers::cpu(state),
        (&Method::Get, "/api/memory") => handlers::memory(state),
        // Network
        (&Method::Get, "/api/network/signal") => handlers::network_signal(state),
        (&Method::Get, "/api/network/traffic") => handlers::network_traffic(state),
        (&Method::Get, "/api/network/speed") => handlers::network_speed(state),
        (&Method::Get, "/api/network/wan") => network_ext::network_wan(state),
        (&Method::Get, "/api/network/wan6") => network_ext::network_wan6(state),
        (&Method::Get, "/api/network/lan-status") => network_ext::network_lan_status(state),
        (&Method::Get, "/api/network/clients") => network_ext::network_clients(state),
        (&Method::Get, "/api/network/speeds") => network_ext::network_speeds(state),
        (&Method::Get, "/api/network/rmnet") => network_ext::network_rmnet(state),
        // Device (ubus)
        (&Method::Get, "/api/device/battery-info") => network_ext::network_battery_ubus(state),
        (&Method::Get, "/api/device/thermal") => device_ext::device_thermal(state),
        (&Method::Get, "/api/device/thermal/all") => device_ext::device_thermal_all(state),
        (&Method::Get, "/api/device/battery/detail") => device_ext::device_battery_detail(state),
        (&Method::Get, "/api/device/charger") => device_ext::device_charger(state),
        (&Method::Get, "/api/device/system") => device_ext::device_system(state),
        (&Method::Post, "/api/device/reboot") => device_ext::device_reboot(state),
        (&Method::Post, "/api/device/shutdown") => device_ext::device_shutdown(state),
        (&Method::Post, "/api/device/factory-reset") => device_ext::device_factory_reset(state),
        (&Method::Post, "/api/device/power-save") => device_ext::device_power_save_get(state, body),
        (&Method::Put, "/api/device/power-save") => device_ext::device_power_save_set(state, body),
        (&Method::Get, "/api/device/fast-boot") => device_ext::device_fast_boot_get(state),
        (&Method::Put, "/api/device/fast-boot") => device_ext::device_fast_boot_set(state, body),
        // System
        (&Method::Post, "/api/system/restart-agent") => device_ext::agent_restart(state),
        (&Method::Get, "/api/system/top") => handlers::system_top(state),
        (&Method::Post, "/api/system/kill-bloat") => handlers::system_kill_bloat(state, body),
        // WiFi
        (&Method::Get, "/api/wifi/status") => wifi::wifi_status(state),
        (&Method::Put, "/api/wifi/settings") => wifi::wifi_set(state, body),
        (&Method::Get, "/api/wifi/guest") => wifi::guest_status(state),
        (&Method::Put, "/api/wifi/guest") => wifi::guest_set(state, body),
        // Modem
        (&Method::Get, "/api/data-usage") => handlers::data_usage(state),
        (&Method::Put, "/api/data-usage/reset-day") => {
            handlers::data_usage_reset_day_set(state, body)
        }
        (&Method::Get, "/api/modem/status") => handlers::modem_status(state),
        (&Method::Post, "/api/modem/online") => handlers::modem_online(state),
        (&Method::Get, "/api/modem/data") => modem_ext::modem_data_get(state),
        (&Method::Put, "/api/modem/data") => modem_ext::modem_data_set(state, body),
        (&Method::Post, "/api/modem/airplane") => modem_ext::modem_airplane(state, body),
        (&Method::Put, "/api/modem/network-mode") => modem_ext::modem_network_mode_set(state, body),
        (&Method::Post, "/api/modem/scan") => modem_ext::modem_scan(state),
        (&Method::Get, "/api/modem/scan/status") => modem_ext::modem_scan_status(state),
        (&Method::Get, "/api/modem/scan/results") => modem_ext::modem_scan_results(state),
        (&Method::Post, "/api/modem/register") => modem_ext::modem_register(state, body),
        (&Method::Get, "/api/modem/register/result") => modem_ext::modem_register_result(state),
        // SMS
        (&Method::Post, "/api/sms/list") => sms::sms_list(state, body),
        (&Method::Get, "/api/sms/capacity") => sms::sms_capacity(state),
        (&Method::Post, "/api/sms/send") => sms::sms_send(state, body),
        (&Method::Post, "/api/sms/delete") => sms::sms_delete(state, body),
        (&Method::Post, "/api/sms/read") => sms::sms_mark_read(state, body),
        // SMS forwarding
        (&Method::Get, "/api/sms/forward/config") => sms_forward::config_get(state),
        (&Method::Put, "/api/sms/forward/config") => sms_forward::config_set(state, body),
        (&Method::Post, "/api/sms/forward/rules") => sms_forward::rules_create(state, body),
        (&Method::Put, "/api/sms/forward/rules") => sms_forward::rules_update(state, body),
        (&Method::Delete, "/api/sms/forward/rules") => sms_forward::rules_delete(state, body),
        (&Method::Put, "/api/sms/forward/rules/toggle") => sms_forward::rules_toggle(state, body),
        (&Method::Post, "/api/sms/forward/test") => sms_forward::test_forward(state, body),
        (&Method::Get, "/api/sms/forward/log") => sms_forward::log_get(state),
        (&Method::Post, "/api/sms/forward/log/clear") => sms_forward::log_clear(state),
        (&Method::Post, "/api/sms/forward/retry") => sms_forward::retry_forward(state, body),
        // SIM
        (&Method::Get, "/api/sim/info") => sim::sim_info(state),
        (&Method::Get, "/api/sim/imei") => sim::sim_imei(state),
        (&Method::Post, "/api/sim/pin/verify") => sim::sim_pin_verify(state, body),
        (&Method::Post, "/api/sim/pin/change") => sim::sim_pin_change(state, body),
        (&Method::Post, "/api/sim/pin/mode") => sim::sim_pin_mode(state, body),
        (&Method::Post, "/api/sim/unlock") => sim::sim_unlock(state, body),
        (&Method::Get, "/api/sim/lock-trials") => sim::sim_lock_trials(state),
        // Cell
        (&Method::Post, "/api/cell/lock/nr") => cell::cell_lock_nr(state, body),
        (&Method::Post, "/api/cell/lock/lte") => cell::cell_lock_lte(state, body),
        (&Method::Post, "/api/cell/lock/reset") => cell::cell_lock_reset(state),
        (&Method::Post, "/api/cell/neighbors/scan") => cell::cell_neighbors_scan(state),
        (&Method::Get, "/api/cell/neighbors/nr") => cell::cell_neighbors_nr(state),
        (&Method::Get, "/api/cell/neighbors/lte") => cell::cell_neighbors_lte(state),
        (&Method::Post, "/api/cell/band/nr") => cell::cell_band_nr(state, body),
        (&Method::Post, "/api/cell/band/lte") => cell::cell_band_lte(state, body),
        (&Method::Post, "/api/cell/band/reset") => cell::cell_band_reset(state),
        (&Method::Get, "/api/cell/stc/params") => cell::cell_stc_params_get(state),
        (&Method::Put, "/api/cell/stc/params") => cell::cell_stc_params_set(state, body),
        (&Method::Get, "/api/cell/stc/status") => cell::cell_stc_status(state),
        (&Method::Post, "/api/cell/stc/enable") => cell::cell_stc_enable(state),
        (&Method::Post, "/api/cell/stc/disable") => cell::cell_stc_disable(state),
        (&Method::Post, "/api/cell/stc/reset") => cell::cell_stc_reset(state),
        (&Method::Post, "/api/cell/signal-detect/start") => cell::cell_signal_detect_start(state),
        (&Method::Post, "/api/cell/signal-detect/stop") => cell::cell_signal_detect_stop(state),
        (&Method::Get, "/api/cell/signal-detect/results") => {
            cell::cell_signal_detect_results(state)
        }
        (&Method::Get, "/api/cell/signal-detect/progress") => {
            cell::cell_signal_detect_progress(state)
        }
        // Router
        (&Method::Get, "/api/router/dns") => router::router_dns_get(state),
        (&Method::Put, "/api/router/dns") => router::router_dns_set(state, body),
        (&Method::Get, "/api/router/lan") => router::router_lan_get(state),
        (&Method::Put, "/api/router/lan") => router::router_lan_set(state, body),
        (&Method::Get, "/api/router/firewall") => router::router_firewall_get(state),
        (&Method::Put, "/api/router/firewall/switch") => {
            router::router_firewall_switch_set(state, body)
        }
        (&Method::Put, "/api/router/firewall/level") => {
            router::router_firewall_level_set(state, body)
        }
        (&Method::Put, "/api/router/firewall/nat") => router::router_firewall_nat_set(state, body),
        (&Method::Put, "/api/router/firewall/dmz") => router::router_firewall_dmz_set(state, body),
        (&Method::Get, "/api/router/firewall/upnp") => router::router_firewall_upnp_get(state),
        (&Method::Put, "/api/router/firewall/upnp") => {
            router::router_firewall_upnp_set(state, body)
        }
        (&Method::Get, "/api/router/firewall/port-forward") => {
            router::router_firewall_port_forward_get(state)
        }
        (&Method::Post, "/api/router/firewall/port-forward") => {
            router::router_firewall_port_forward_set(state, body)
        }
        (&Method::Put, "/api/router/firewall/port-forward/switch") => {
            router::router_firewall_port_forward_switch(state, body)
        }
        (&Method::Get, "/api/router/firewall/filter-rules") => {
            router::router_firewall_filter_rules(state)
        }
        (&Method::Get, "/api/router/vpn") => router::router_vpn_get(state),
        (&Method::Put, "/api/router/vpn") => router::router_vpn_set(state, body),
        (&Method::Get, "/api/router/qos") => router::router_qos_get(state),
        (&Method::Put, "/api/router/qos") => router::router_qos_set(state, body),
        (&Method::Get, "/api/router/domain-filter") => router::router_domain_filter_get(state),
        (&Method::Put, "/api/router/domain-filter") => {
            router::router_domain_filter_set(state, body)
        }
        (&Method::Get, "/api/router/apn/mode") => router::router_apn_mode_get(state),
        (&Method::Put, "/api/router/apn/mode") => router::router_apn_mode_set(state, body),
        (&Method::Get, "/api/router/apn/profiles") => router::router_apn_profiles_get(state),
        (&Method::Post, "/api/router/apn/profiles") => router::router_apn_profiles_add(state, body),
        (&Method::Put, "/api/router/apn/profiles") => {
            router::router_apn_profiles_modify(state, body)
        }
        (&Method::Get, "/api/router/apn/auto-profiles") => router::router_apn_auto_profiles(state),
        (&Method::Post, "/api/router/apn/profiles/delete") => {
            router::router_apn_profiles_delete(state, body)
        }
        (&Method::Post, "/api/router/apn/profiles/activate") => {
            router::router_apn_profiles_activate(state, body)
        }
        // USB
        (&Method::Get, "/api/usb/status") => usb::usb_status(state),
        (&Method::Put, "/api/usb/mode") => usb::usb_mode_set(state, body),
        (&Method::Put, "/api/usb/powerbank") => usb::usb_powerbank_set(state, body),
        // Telephony — calls
        (&Method::Post, "/api/call/dial") => telephony::call_dial(state, body),
        (&Method::Post, "/api/call/hangup") => telephony::call_hangup(state),
        (&Method::Post, "/api/call/answer") => telephony::call_answer(state),
        (&Method::Get, "/api/call/status") => telephony::call_status(state),
        (&Method::Post, "/api/call/dtmf") => telephony::call_dtmf(state, body),
        (&Method::Post, "/api/call/mute") => telephony::call_mute(state, body),
        // Telephony — USSD
        (&Method::Post, "/api/ussd/send") => telephony::ussd_send(state, body),
        (&Method::Post, "/api/ussd/respond") => telephony::ussd_respond(state, body),
        (&Method::Post, "/api/ussd/cancel") => telephony::ussd_cancel(state),
        // Telephony — STK
        (&Method::Get, "/api/stk/menu") => telephony::stk_menu(state),
        (&Method::Post, "/api/stk/select") => telephony::stk_select(state, body),
        // DoH
        (&Method::Get, "/api/doh/status") => doh_status(state),
        (&Method::Put, "/api/doh/config") => doh_config_set(state, body),
        (&Method::Post, "/api/doh/enable") => doh_enable(state),
        (&Method::Post, "/api/doh/disable") => doh_disable(state),
        (&Method::Get, "/api/doh/cache") => doh_cache_list(state),
        (&Method::Post, "/api/doh/cache/clear") => doh_cache_clear(state),
        // LAN test (download/upload handled above before body read)
        (&Method::Get, "/api/lan/ping") => lan_test::ping(),
        // Speed test
        (&Method::Get, "/api/speedtest/servers") => speedtest::servers(state),
        (&Method::Post, "/api/speedtest/start") => speedtest::start(state, body),
        (&Method::Get, "/api/speedtest/progress") => speedtest::progress(state),
        (&Method::Post, "/api/speedtest/stop") => speedtest::stop(state, body),
        // Scheduler
        (&Method::Get, "/api/scheduler/jobs") => scheduler::jobs_list(state),
        (&Method::Post, "/api/scheduler/jobs") => scheduler::jobs_create(state, body),
        (&Method::Put, "/api/scheduler/jobs") => scheduler::jobs_update(state, body),
        (&Method::Delete, "/api/scheduler/jobs") => scheduler::jobs_delete(state, body),
        (&Method::Put, "/api/scheduler/jobs/toggle") => scheduler::jobs_toggle(state, body),
        // TTL clamping
        (&Method::Get, "/api/ttl/status") => ttl_status(),
        (&Method::Put, "/api/ttl/set") => ttl_set(body),
        (&Method::Delete, "/api/ttl/clear") => ttl_clear(),
        // AT console
        (&Method::Post, "/api/at/send") => at_console(state, body),
        // Signal logger
        (&Method::Post, "/api/logger/signal/start") => signal_logger::start_logging(state, body),
        (&Method::Post, "/api/logger/signal/stop") => signal_logger::stop_logging(state),
        (&Method::Get, "/api/logger/signal/status") => signal_logger::status(state),
        (&Method::Get, "/api/logger/signal/download") => signal_logger::download(state),
        // Connection logger
        (&Method::Post, "/api/logger/connection/start") => {
            connection_logger::start_logging(state, body)
        }
        (&Method::Post, "/api/logger/connection/stop") => connection_logger::stop_logging(state),
        (&Method::Get, "/api/logger/connection/status") => connection_logger::status(state),
        (&Method::Get, "/api/logger/connection/download") => connection_logger::download(state),
        // Fallback
        _ => (404, json!({"ok": false, "error": "not found"})),
    }
}

// --- AT console ---

fn at_console(state: &AppState, body: &[u8]) -> (u16, Value) {
    let parsed: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({"ok": false, "error": "invalid JSON"})),
    };
    let command = match parsed["command"].as_str() {
        Some(c) if !c.is_empty() => c,
        _ => return (400, json!({"ok": false, "error": "missing 'command'"})),
    };
    let timeout = parsed["timeout"].as_u64().unwrap_or(2).min(30);

    if !is_at_command_allowed(command) {
        return (
            403,
            json!({"ok": false, "error": "command not allowed. Only read-only AT commands are permitted."}),
        );
    }

    match at_cmd::send(&state.at_port, command, timeout) {
        Ok(resp) => (200, json!({"ok": true, "data": {"response": resp.trim()}})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

const AT_BLOCKED_PREFIXES: &[&str] = &[
    "AT+CFUN",
    "AT^",
    "AT$QCRMCALL",
    "AT+CLCK",
    "AT+CMGD",
    "AT+CMGF=1;+CMGS",
    "AT+CGDCONT=",
    "AT+CGACT=",
];

const AT_ALLOWED_PREFIXES: &[&str] = &[
    "ATI",
    "AT+CSQ",
    "AT+COPS",
    "AT+CGDCONT?",
    "AT+CREG",
    "AT+CGREG",
    "AT+CEREG",
    "AT+CGPADDR",
    "AT+CGACT?",
    "AT+CLAC",
    "AT+CGSN",
    "AT+CGMI",
    "AT+CGMM",
    "AT+CGMR",
    "AT+QENG",
    "AT+QNWINFO",
    "AT+QRSRP",
    "AT+QRSRQ",
    "AT+QINISTAT",
    "AT+QSPN",
    "AT+QCIDINCOMING",
    "AT+CGDCONT?",
    "AT+CGCONTRDP",
    "AT+CGPADDR",
    "AT",
];

fn is_at_command_allowed(cmd: &str) -> bool {
    let upper = cmd.trim().to_uppercase();
    if upper.is_empty() {
        return false;
    }
    for prefix in AT_BLOCKED_PREFIXES {
        if upper.starts_with(prefix) {
            return false;
        }
    }
    for prefix in AT_ALLOWED_PREFIXES {
        if upper.starts_with(prefix) {
            return true;
        }
    }
    false
}

// --- TTL handlers ---

fn ttl_status() -> (u16, Value) {
    let ipv4 = std::process::Command::new("iptables")
        .args(["-t", "mangle", "-L", "PREROUTING", "-n"])
        .output();
    let ipv6 = std::process::Command::new("ip6tables")
        .args(["-t", "mangle", "-L", "PREROUTING", "-n"])
        .output();
    let mut active = false;
    let mut ttl_value: u32 = 0;
    if let Ok(out) = &ipv4 {
        let s = String::from_utf8_lossy(&out.stdout);
        for line in s.lines() {
            if line.contains("TTL set to") {
                active = true;
                if let Some(v) = line.rsplit("TTL set to ").next() {
                    ttl_value = v.trim().parse().unwrap_or(0);
                }
                break;
            }
        }
    }
    let mut hl_active = false;
    if let Ok(out) = &ipv6 {
        let s = String::from_utf8_lossy(&out.stdout);
        for line in s.lines() {
            if line.contains("HL set to") {
                hl_active = true;
                if ttl_value == 0 {
                    if let Some(v) = line.rsplit("HL set to ").next() {
                        ttl_value = v.trim().parse().unwrap_or(0);
                    }
                }
                break;
            }
        }
    }
    (
        200,
        json!({"ok": true, "data": {
            "active": active,
            "ipv6_active": hl_active,
            "ttl_value": ttl_value,
        }}),
    )
}

fn ttl_set(body: &[u8]) -> (u16, Value) {
    let val: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({"ok": false, "error": "invalid JSON"})),
    };
    let ttl = match val.get("ttl").and_then(|v| v.as_u64()) {
        Some(v) if v >= 1 && v <= 255 => v as u32,
        _ => return (400, json!({"ok": false, "error": "ttl must be 1-255"})),
    };
    // Clear existing rules first
    let _ = std::process::Command::new("sh").args(["-c",
        "iptables -t mangle -S PREROUTING 2>/dev/null | grep 'TTL --ttl-set' | while read -r rule; do iptables -t mangle $(echo \"$rule\" | sed 's/-A/-D/'); done"
    ]).output();
    let _ = std::process::Command::new("sh").args(["-c",
        "ip6tables -t mangle -S PREROUTING 2>/dev/null | grep 'HL --hl-set' | while read -r rule; do ip6tables -t mangle $(echo \"$rule\" | sed 's/-A/-D/'); done"
    ]).output();
    // Add new rules
    let r4 = std::process::Command::new("iptables")
        .args([
            "-t",
            "mangle",
            "-A",
            "PREROUTING",
            "-i",
            "br-lan",
            "-j",
            "TTL",
            "--ttl-set",
            &ttl.to_string(),
        ])
        .output();
    let r6 = std::process::Command::new("ip6tables")
        .args([
            "-t",
            "mangle",
            "-A",
            "PREROUTING",
            "-i",
            "br-lan",
            "-j",
            "HL",
            "--hl-set",
            &ttl.to_string(),
        ])
        .output();
    let ok4 = r4.map(|o| o.status.success()).unwrap_or(false);
    let ok6 = r6.map(|o| o.status.success()).unwrap_or(false);
    // Persist to start_ttl.sh
    let script = format!(
        "#!/bin/sh\niptables  -t mangle -C PREROUTING -i br-lan -j TTL --ttl-set {ttl} 2>/dev/null ||   iptables  -t mangle -A PREROUTING -i br-lan -j TTL --ttl-set {ttl}\nip6tables -t mangle -C PREROUTING -i br-lan -j HL  --hl-set  {ttl} 2>/dev/null ||   ip6tables -t mangle -A PREROUTING -i br-lan -j HL  --hl-set  {ttl}\n"
    );
    let _ = std::fs::write("/data/local/tmp/start_ttl.sh", script);
    if ok4 || ok6 {
        (
            200,
            json!({"ok": true, "data": {"ttl": ttl, "ipv4": ok4, "ipv6": ok6}}),
        )
    } else {
        (
            500,
            json!({"ok": false, "error": format!("ipv4={ok4} ipv6={ok6}")}),
        )
    }
}

fn ttl_clear() -> (u16, Value) {
    let _ = std::process::Command::new("sh").args(["-c",
        "iptables -t mangle -S PREROUTING 2>/dev/null | grep 'TTL --ttl-set' | while read -r rule; do iptables -t mangle $(echo \"$rule\" | sed 's/-A/-D/'); done"
    ]).output();
    let _ = std::process::Command::new("sh").args(["-c",
        "ip6tables -t mangle -S PREROUTING 2>/dev/null | grep 'HL --hl-set' | while read -r rule; do ip6tables -t mangle $(echo \"$rule\" | sed 's/-A/-D/'); done"
    ]).output();
    // Remove persistence script content (keep file but make it a no-op)
    let _ = std::fs::write(
        "/data/local/tmp/start_ttl.sh",
        "#!/bin/sh\n# TTL disabled\n",
    );
    (200, json!({"ok": true}))
}

// --- DoH handlers ---

fn doh_status(state: &AppState) -> (u16, Value) {
    (200, json!({"ok": true, "data": state.doh.status()}))
}

fn doh_config_set(state: &AppState, body: &[u8]) -> (u16, Value) {
    match state.doh.update_config(body) {
        Ok(()) => (200, json!({"ok": true})),
        Err(e) => (400, json!({"ok": false, "error": e})),
    }
}

fn doh_enable(state: &AppState) -> (u16, Value) {
    if let Err(e) = state.doh.start() {
        return (500, json!({"ok": false, "error": e}));
    }
    // Write DoH forwarding config to dnsmasq.d drop-in
    // (UCI `set` creates a plain option, but dnsmasq init only reads `server` as a list — drop-in is reliable)
    let _ = std::fs::write(
        "/tmp/dnsmasq.d/doh.conf",
        "server=127.0.0.1#5353\nno-resolv\n",
    );
    let _ = std::process::Command::new("sh")
        .args(["-c", "/etc/init.d/dnsmasq restart"])
        .output();
    // Save config
    state.doh.set_enabled(true);
    (200, json!({"ok": true, "data": {"status": "enabled"}}))
}

fn doh_disable(state: &AppState) -> (u16, Value) {
    state.doh.stop();
    dnsmasq_restore_defaults();
    state.doh.set_enabled(false);
    (200, json!({"ok": true, "data": {"status": "disabled"}}))
}

/// Restore dnsmasq to default DNS resolution (remove DoH forwarding).
/// Safe to call even if dnsmasq isn't forwarding to DoH.
pub fn dnsmasq_restore_defaults() {
    let _ = std::process::Command::new("sh")
        .args(["-c", "rm -f /tmp/dnsmasq.d/doh.conf; uci delete dhcp.lan_dns.server 2>/dev/null; uci delete dhcp.lan_dns.noresolv 2>/dev/null; uci commit dhcp; /etc/init.d/dnsmasq restart"])
        .output();
}

fn doh_cache_list(state: &AppState) -> (u16, Value) {
    (200, json!({"ok": true, "data": state.doh.cache_entries()}))
}

fn doh_cache_clear(state: &AppState) -> (u16, Value) {
    state.doh.clear_cache();
    (200, json!({"ok": true}))
}

fn respond(request: Request, status: u16, body: Value, origin: Option<&str>) {
    let body_str = serde_json::to_string(&body).unwrap_or_default();
    let content_type = Header::from_bytes("Content-Type", "application/json").unwrap();
    let mut response = Response::from_string(body_str)
        .with_status_code(status)
        .with_header(content_type);
    for h in cors_headers(origin) {
        response = response.with_header(h);
    }
    let _ = request.respond(response);
}
