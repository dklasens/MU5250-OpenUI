//! QoS the network granted on the current data session: the QCI/5QI of the
//! default bearer and the AMBR (the operator's rate cap).
//!
//! The QMI messages carrying these are not public, but the stock
//! `zte_topsw_data` daemon decodes them and leaves two traces: the
//! `zwrt_data_tmp.wwaniface1.qci` UCI state and one `[DATA]` line per session
//! setup in /logfs/key.log. The log runs to 8 MiB before rotating, so it is
//! scanned backwards once and afterwards only appended bytes are read.

use serde_json::{json, Value};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Mutex;

const KEY_LOG: &str = "/logfs/key.log";
const KEY_LOG_ROTATED: &str = "/logfs/key.log.0";
const CHUNK: u64 = 256 * 1024;

/// Bytes of KEY_LOG already read, and the newest AMBR found in them.
static LOG: Mutex<(u64, Option<Ambr>)> = Mutex::new((0, None));

#[derive(Debug, Clone, PartialEq)]
struct Ambr {
    apn: String,
    dl_mbps: f64,
    ul_mbps: f64,
}

pub fn read() -> Value {
    let qci = crate::ubus::uci_get("zwrt_data_tmp.wwaniface1.qci")
        .ok()
        .and_then(|v| v.trim().parse::<u8>().ok());
    let ambr = current_ambr();
    if qci.is_none() && ambr.is_none() {
        return Value::Null;
    }
    json!({
        "qci": qci,
        "ambr_dl_mbps": ambr.as_ref().map(|a| a.dl_mbps),
        "ambr_ul_mbps": ambr.as_ref().map(|a| a.ul_mbps),
    })
}

fn current_ambr() -> Option<Ambr> {
    let mut log = LOG.lock().unwrap_or_else(|e| e.into_inner());
    let (read_len, ambr) = &mut *log;
    let len = std::fs::metadata(KEY_LOG).map_or(0, |m| m.len());
    if *read_len == 0 || len < *read_len {
        // First look, or the log rotated.
        *ambr = scan_backwards(KEY_LOG).or_else(|| scan_backwards(KEY_LOG_ROTATED));
    } else if let Some(newer) = read_range(KEY_LOG, *read_len, len).and_then(|t| latest(&t)) {
        *ambr = Some(newer);
    }
    *read_len = len;
    ambr.clone()
}

fn scan_backwards(path: &str) -> Option<Ambr> {
    let mut end = std::fs::metadata(path).ok()?.len();
    while end > 0 {
        let start = end.saturating_sub(CHUNK);
        let text = read_range(path, start, end)?;
        // Skip the line cut at `start`; the next, earlier chunk ends after it.
        let cut = if start == 0 { 0 } else { text.find('\n')? + 1 };
        if let Some(a) = latest(&text[cut..]) {
            return Some(a);
        }
        end = start + cut as u64;
    }
    None
}

fn read_range(path: &str, start: u64, end: u64) -> Option<String> {
    let mut f = File::open(path).ok()?;
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.take(end.saturating_sub(start))
        .read_to_end(&mut buf)
        .ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// The newest AMBR line in `text` for a data session (not IMS).
fn latest(text: &str) -> Option<Ambr> {
    text.lines()
        .rev()
        .filter_map(parse_line)
        .find(|a| !a.apn.eq_ignore_ascii_case("ims"))
}

/// Parses the two line shapes `zte_topsw_data` writes:
///
/// 5G PDU session accept, value × unit:
/// `dnn=3gnet session_ambr_dl=2000 session_ambr_dl_unit=6(1Mbps) …`
///
/// LTE default bearer activation, the three APN-AMBR octets of TS 24.301
/// §9.9.4.2, with ext2 printed as a running total including the base octet:
/// `access_point=… apn_ambr_dl=8640kbps apn_ambr_dl_ext=176.000Mbps apn_ambr_dl_ext2=1208.640Mbps …`
fn parse_line(line: &str) -> Option<Ambr> {
    if let Some(apn) = field(line, "dnn") {
        return Some(Ambr {
            apn: apn.into(),
            dl_mbps: nr_rate(line, "dl")?,
            ul_mbps: nr_rate(line, "ul")?,
        });
    }
    let apn = field(line, "access_point")?;
    Some(Ambr {
        apn: apn.into(),
        dl_mbps: lte_rate(line, "dl")?,
        ul_mbps: lte_rate(line, "ul")?,
    })
}

fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = &line[line.find(&format!(" {key}="))? + key.len() + 2..];
    rest.split_whitespace().next()
}

fn nr_rate(line: &str, dir: &str) -> Option<f64> {
    let value: f64 = field(line, &format!("session_ambr_{dir}"))?.parse().ok()?;
    // TS 24.501 unit, printed as e.g. `6(1Mbps)` or `7(4Mbps)`.
    let unit = field(line, &format!("session_ambr_{dir}_unit"))?
        .split_once('(')?
        .1
        .trim_end_matches(')');
    let digits = unit.find(|c: char| !c.is_ascii_digit())?;
    let scale = match &unit[digits..] {
        "Kbps" => 0.001,
        "Mbps" => 1.0,
        "Gbps" => 1000.0,
        _ => return None,
    };
    Some(value * unit[..digits].parse::<f64>().ok()? * scale)
}

fn lte_rate(line: &str, dir: &str) -> Option<f64> {
    let num = |key: String, suffix| field(line, &key)?.strip_suffix(suffix)?.parse::<f64>().ok();
    let base = num(format!("apn_ambr_{dir}"), "kbps")? / 1000.0;
    let ext = num(format!("apn_ambr_{dir}_ext"), "Mbps")?;
    let ext2 = num(format!("apn_ambr_{dir}_ext2"), "Mbps")?;
    // A base octet of 8640 kbps defers to the extended octets.
    let mbps = if ext2 > 0.0 {
        ext2 - base
    } else if base >= 8.64 {
        ext
    } else {
        base
    };
    Some((mbps * 1000.0).round() / 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_and_apn_ambr() {
        let log = "\
            12:14:56 : [DATA] eps_bearer_id=5 msg_type=193 access_point=internet.mnc001.mcc260.gprs apn_ambr_dl=8640kbps apn_ambr_ul=8640kbps apn_ambr_dl_ext=176.000Mbps apn_ambr_ul_ext=200.000Mbps apn_ambr_dl_ext2=1208.640Mbps apn_ambr_ul_ext2=0.000Mbps\n\
            12:14:56 : [DATA] pdu_session_id=2 msg_type=194 dnn=3gnet session_ambr_dl=2000 session_ambr_dl_unit=6(1Mbps) session_ambr_ul=200 session_ambr_ul_unit=6(1Mbps)\n\
            12:14:56 : [DATA] pdu_session_id=1 msg_type=194 dnn=IMS session_ambr_dl=30000 session_ambr_dl_unit=1(1Kbps) session_ambr_ul=30000 session_ambr_ul_unit=1(1Kbps)\n";
        let lte = parse_line(log.lines().next().unwrap()).unwrap();
        assert_eq!((lte.dl_mbps, lte.ul_mbps), (1200.0, 200.0));
        let nr = latest(log).unwrap();
        assert_eq!(
            nr,
            Ambr {
                apn: "3gnet".into(),
                dl_mbps: 2000.0,
                ul_mbps: 200.0
            }
        );
    }
}
