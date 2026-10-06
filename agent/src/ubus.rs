use crate::process::BoundedCommand;
use std::collections::HashMap;
use std::process::Command;

use serde_json::Value;

/// Call `ubus call <object> <method> [<params>]` and parse JSON output.
pub fn call(object: &str, method: &str, params: Option<&str>) -> Result<Value, String> {
    let mut cmd = Command::new("ubus");
    cmd.args(["call", object, method]);
    if let Some(p) = params {
        cmd.arg(p);
    }
    let output = cmd
        .bounded_output()
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

/// Whether `object` exposes `method`, from `ubus -v list <object>`. `None` when
/// ubus could not answer, so callers can ask again instead of caching a guess.
pub fn has_method(object: &str, method: &str) -> Option<bool> {
    let output = Command::new("ubus")
        .args(["-v", "list", object])
        .bounded_output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(lists_method(
        &String::from_utf8_lossy(&output.stdout),
        method,
    ))
}

fn lists_method(listing: &str, method: &str) -> bool {
    let needle = format!("\"{method}\":");
    listing
        .lines()
        .any(|line| line.trim_start().starts_with(&needle))
}

/// Dump a whole UCI config in one `uci -N show` and return it as a map of
/// `section.option` -> value.
///
/// One subprocess instead of one per key. `wifi_status` used to issue ~28
/// separate `uci get` calls, each a fork+exec, to build a single response.
pub fn uci_show(config: &str) -> HashMap<String, String> {
    let output = match Command::new("uci").args(["show", config]).bounded_output() {
        Ok(o) if o.status.success() => o,
        _ => return HashMap::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
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
    use super::{lists_method, uci_unquote};

    #[test]
    fn method_listing_matches_whole_names() {
        let b31 = "'zwrt_bsp.usb' @5a6476ce\n\t\"list\":{}\n";
        assert!(lists_method(b31, "list"));
        assert!(!lists_method(b31, "set"));
        let b04 = "'zwrt_bsp.usb' @1\n\t\"list\":{}\n\t\"set\":{\"mode\":\"String\"}\n";
        assert!(lists_method(b04, "set"));
        assert!(!lists_method(b04, "se"));
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
