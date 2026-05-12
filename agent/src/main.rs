mod at_cmd;
mod auth;
mod cell;
mod connection_logger;
mod csv_utils;
mod device_ext;
pub mod doh;
mod event_bus;
mod handlers;
mod lan_test;
mod modem_ext;
mod network_ext;
mod router;
mod scheduler;
mod server;
mod signal_logger;
mod sim;
mod sms;
mod sms_forward;
mod speedtest;
mod system;
mod telephony;
mod ubus;
mod usb;
mod util;
mod validate;
mod wifi;

use std::sync::Arc;

use event_bus::EventBus;
use handlers::AppState;

const DEFAULT_BIND: &str = "192.168.0.1:9090";
const DEFAULT_THREADS: usize = 4;

fn main() {
    let bind = std::env::var("ZTE_AGENT_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_string());
    let threads: usize = std::env::var("ZTE_AGENT_THREADS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_THREADS);

    let state = Arc::new(AppState::new());

    // Set password from environment if provided
    if let Ok(pw) = std::env::var("ZTE_AGENT_PASSWORD") {
        state.auth.set_password(&pw);
    } else {
        // Fallback: try reading from the startup script if executed manually
        if let Ok(script) = std::fs::read_to_string("/data/local/tmp/start_zte_agent.sh") {
            for line in script.lines() {
                if line.starts_with("export ZTE_AGENT_PASSWORD=") {
                    let pw = line
                        .trim_start_matches("export ZTE_AGENT_PASSWORD=")
                        .trim_matches(|c| c == '\'' || c == '"');
                    state.auth.set_password(pw);
                    break;
                }
            }
        }
    }

    if let Ok(pin) = std::env::var("ZTE_AGENT_PIN") {
        if let Err(e) = state.auth.set_pin(&pin) {
            eprintln!("[WARN] ignoring invalid ZTE_AGENT_PIN: {e}");
        }
    } else if let Ok(script) = std::fs::read_to_string("/data/local/tmp/start_zte_agent.sh") {
        for line in script.lines() {
            if line.starts_with("export ZTE_AGENT_PIN=") {
                let pin = line
                    .trim_start_matches("export ZTE_AGENT_PIN=")
                    .trim_matches(|c| c == '\'' || c == '"');
                if let Err(e) = state.auth.set_pin(pin) {
                    eprintln!("[WARN] ignoring invalid ZTE_AGENT_PIN: {e}");
                }
                break;
            }
        }
    }

    // Event bus: single `ubus listen` process dispatches to subscribers
    let event_bus = EventBus::new();
    let sms_rx = event_bus.subscribe("zwrt_wms_status_event");
    event_bus.start();

    state.doh.auto_start();
    state.scheduler.start(Arc::clone(&state));
    state.sms_forward.start(sms_rx);

    // Apply persisted TTL settings if they exist
    let _ = std::process::Command::new("sh")
        .arg("/data/local/tmp/start_ttl.sh")
        .output();

    wifi::enforce_wifi_state_on_boot();

    server::start(&bind, threads, state);
}
