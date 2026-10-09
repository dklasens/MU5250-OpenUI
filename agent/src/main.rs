use crate::process::BoundedCommand;
mod at_cmd;
mod auth;
mod bearer;
mod cache;
mod cell;
mod charge_policy;
mod connection_logger;
mod csv_utils;
mod device_ext;
mod diag_log;
mod event_bus;
mod handlers;
mod lan;
mod logging;
mod network_ext;
mod process;
mod router;
mod server;
mod signal_logger;
mod sim;
mod sms;
mod storage;
mod system;
mod ubus;
mod uci_transaction;
mod usb;
mod util;
mod validate;
mod wifi;

use std::sync::Arc;

use event_bus::EventBus;
use handlers::AppState;

const DEFAULT_THREADS: usize = 4;
const STARTUP_SCRIPT: &str = "/data/local/tmp/start_zte_agent.sh";

fn main() {
    diag_log::init();
    eprintln!(
        "[agent] zte-agent {} starting (pid {})",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    );

    let threads: usize = std::env::var("ZTE_AGENT_THREADS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_THREADS);

    migrate_drop_removed_features();

    let state = Arc::new(AppState::new());

    // Set password from environment if provided
    if let Ok(pw) = std::env::var("ZTE_AGENT_PASSWORD") {
        state.auth.set_password(&pw);
    } else if let Some(pw) = read_startup_export("ZTE_AGENT_PASSWORD") {
        // Fallback: try reading from the startup script if executed manually
        state.auth.set_password(&pw);
    }

    let pin = std::env::var("ZTE_AGENT_PIN")
        .ok()
        .or_else(|| read_startup_export("ZTE_AGENT_PIN"));
    if let Some(pin) = pin {
        if let Err(e) = state.auth.set_pin(&pin) {
            eprintln!("[WARN] ignoring invalid ZTE_AGENT_PIN: {e}");
        }
    }

    // Event bus: single `ubus listen` process dispatches to subscribers
    let event_bus = EventBus::new();
    let charger_rx = event_bus.subscribe("BSP_CHARGER_EVENT");
    event_bus.start();

    state.charge_limit.start(charger_rx);

    // Apply persisted TTL settings if they exist
    let _ = std::process::Command::new("sh")
        .arg("/data/local/tmp/start_ttl.sh")
        .bounded_output();

    usb::enforce_usb_mode_on_boot();

    state.lan.recover();
    server::start(threads.clamp(1, 16), state);
}

/// Read `export KEY='value'` out of the startup script.
fn read_startup_export(key: &str) -> Option<String> {
    let script = std::fs::read_to_string(STARTUP_SCRIPT).ok()?;
    let prefix = format!("export {key}=");
    script.lines().find_map(|line| {
        line.strip_prefix(&prefix)
            .map(|v| v.trim_matches(|c| c == '\'' || c == '"').to_string())
    })
}

/// One-shot cleanup for features removed from the agent (DoH proxy, SMS
/// forwarder, job scheduler).
///
/// The DoH part matters: enabling DoH pointed dnsmasq at the agent's own
/// resolver on 127.0.0.1:5353, and the module that undid that is gone. Without
/// this, a device that had DoH enabled would come back up forwarding DNS to a
/// port nothing listens on. Safe to run when DoH was never enabled — the config
/// file only exists if it was configured at least once.
fn migrate_drop_removed_features() {
    const DOH_CONFIG: &str = "/data/local/tmp/doh_config.json";

    if std::path::Path::new(DOH_CONFIG).exists() {
        eprintln!("[migrate] DoH was configured on this device — restoring dnsmasq defaults");
        let _ = std::process::Command::new("sh")
            .args([
                "-c",
                "rm -f /tmp/dnsmasq.d/doh.conf; \
                 uci delete dhcp.lan_dns.server 2>/dev/null; \
                 uci delete dhcp.lan_dns.noresolv 2>/dev/null; \
                 uci commit dhcp; \
                 /etc/init.d/dnsmasq restart",
            ])
            .bounded_output();
        let _ = std::fs::remove_file(DOH_CONFIG);
    }

    for orphan in [
        "/data/local/tmp/sms_forward.json",
        "/data/local/tmp/sms_forward_state.json",
        "/data/local/tmp/scheduler.json",
    ] {
        let _ = std::fs::remove_file(orphan);
    }
}
