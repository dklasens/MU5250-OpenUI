use crate::process::{BoundedCommand, DEFAULT_DEADLINE};
use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

/// Deadline for poll-driven reads behind an `Observed` source. These run on
/// the dashboard heartbeat, where a hung firmware daemon should cost a few
/// seconds and a stale reading, not the default 10 s per source.
pub const POLL_DEADLINE: Duration = Duration::from_secs(3);

/// Call `ubus call <object> <method> [<params>]` and parse JSON output.
pub fn call(object: &str, method: &str, params: Option<&str>) -> Result<Value, String> {
    call_within(object, method, params, DEFAULT_DEADLINE)
}

/// `call` with `POLL_DEADLINE`. Only for idempotent reads that are retried on
/// the next poll anyway; mutations and slow on-demand calls use `call`.
pub fn call_poll(object: &str, method: &str, params: Option<&str>) -> Result<Value, String> {
    call_within(object, method, params, POLL_DEADLINE)
}

fn call_within(
    object: &str,
    method: &str,
    params: Option<&str>,
    timeout: Duration,
) -> Result<Value, String> {
    let mut cmd = Command::new("ubus");
    cmd.args(["call", object, method]);
    if let Some(p) = params {
        cmd.arg(p);
    }
    let output = cmd
        .bounded_output_within(timeout)
        .map_err(|e| format!("ubus exec: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = match stderr.find("\nUsage:") {
            Some(pos) => &stderr[..pos],
            None => &stderr,
        };
        return Err(format!(
            "ubus call {object} {method} failed: {}",
            stderr.trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(trimmed).map_err(|e| format!("ubus JSON parse: {e}"))
}

/// Dump a whole UCI config in one `uci -N show` and return it as a map of
/// `section.option` -> value.
///
/// One subprocess instead of one per key. `wifi_status` used to issue ~28
/// separate `uci get` calls, each a fork+exec, to build a single response.
///
/// A failed read yields an empty map, so every key reads as missing. Callers
/// that must tell "unreadable" from "empty" use `try_uci_show`.
pub fn uci_show(config: &str) -> HashMap<String, String> {
    try_uci_show(config).unwrap_or_default()
}

/// `uci_show`, reporting a failed `uci show` as an error instead of an empty map.
pub fn try_uci_show(config: &str) -> Result<HashMap<String, String>, String> {
    let output = Command::new("uci")
        .args(["show", config])
        .bounded_output()
        .map_err(|e| format!("uci exec: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "uci show {config}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(parse_uci_show(
        config,
        &String::from_utf8_lossy(&output.stdout),
    ))
}

/// Parse `uci show <config>` output into `section.option` -> unquoted value.
fn parse_uci_show(config: &str, stdout: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in stdout.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        // Strip the `<config>.` prefix so callers key off `section.option`,
        // matching the tail of what they'd pass to `uci_get`.
        let Some(key) = key.strip_prefix(config).and_then(|k| k.strip_prefix('.')) else {
            continue;
        };
        map.insert(key.to_string(), uci_unquote(value));
    }
    map
}

/// Undo the shell-style quoting `uci show` applies to values.
///
/// Values are wrapped in single quotes and any embedded quote is written as
/// `'\''`. Naively trimming quotes mangles Wi-Fi passphrases that contain one.
fn uci_unquote(raw: &str) -> String {
    let trimmed = raw.trim();
    let inner = trimmed
        .strip_prefix('\'')
        .and_then(|v| v.strip_suffix('\''))
        .unwrap_or(trimmed);
    inner.replace("'\\''", "'")
}

/// Run `uci get <key>` and return the value.
pub fn uci_get(key: &str) -> Result<String, String> {
    let output = Command::new("uci")
        .args(["get", key])
        .bounded_output()
        .map_err(|e| format!("uci exec: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("uci get {key}: {stderr}"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Run `uci set <key>=<value>` followed by `uci commit <config>`.
#[allow(dead_code)]
pub fn uci_set(key: &str, value: &str) -> Result<(), String> {
    uci_set_no_commit(key, value)?;
    let config = key.split('.').next().unwrap_or(key);
    uci_commit(config)
}

/// Run `uci set <key>=<value>` without committing.
pub fn uci_set_no_commit(key: &str, value: &str) -> Result<(), String> {
    let set_out = Command::new("uci")
        .args(["set", &format!("{key}={value}")])
        .bounded_output()
        .map_err(|e| format!("uci set: {e}"))?;
    if !set_out.status.success() {
        return Err(format!(
            "uci set {key}: {}",
            String::from_utf8_lossy(&set_out.stderr)
        ));
    }
    Ok(())
}

/// Run `uci commit <config>`.
pub fn uci_commit(config: &str) -> Result<(), String> {
    let commit_out = Command::new("uci")
        .args(["commit", config])
        .bounded_output()
        .map_err(|e| format!("uci commit: {e}"))?;
    if !commit_out.status.success() {
        return Err(format!(
            "uci commit {config}: {}",
            String::from_utf8_lossy(&commit_out.stderr)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_uci_show, uci_unquote};

    #[test]
    fn parses_show_output_into_section_option_keys() {
        let out = "zwrt_router.network=zte_router\n\
                   zwrt_router.network.lan_ipaddr='192.168.0.1'\n\
                   zwrt_router.dhcp.leasetime='86400'\n\
                   zwrt_routerx.other.key='ignored'\n\
                   garbage line\n";
        let map = parse_uci_show("zwrt_router", out);
        assert_eq!(map["network"], "zte_router");
        assert_eq!(map["network.lan_ipaddr"], "192.168.0.1");
        assert_eq!(map["dhcp.leasetime"], "86400");
        assert_eq!(map.len(), 3, "{map:?}");
    }

    #[test]
    fn unquotes_plain_values() {
        assert_eq!(uci_unquote("'MyNetwork'"), "MyNetwork");
        assert_eq!(uci_unquote("  '36'  "), "36");
        // Section-type lines are unquoted.
        assert_eq!(uci_unquote("wifi-device"), "wifi-device");
        assert_eq!(uci_unquote("''"), "");
    }

    #[test]
    fn unquotes_embedded_single_quotes() {
        // uci writes an embedded ' as '\''
        assert_eq!(uci_unquote(r"'pass'\''word'"), "pass'word");
        assert_eq!(uci_unquote(r"'it'\''s'\''fine'"), "it's'fine");
    }

    #[test]
    fn preserves_other_shell_metacharacters() {
        assert_eq!(uci_unquote(r#"'Pass$w0rd|<>&`"'"#), r#"Pass$w0rd|<>&`""#);
    }
}
