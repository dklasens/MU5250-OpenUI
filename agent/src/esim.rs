//! eSIM profile management for a removable eUICC (for example a 9eSIM card) in the SIM slot.
//!
//! Card work runs in `zte-esim`, an on-demand helper (`/data/zte-esim`): it carries the
//! SGP.22 card code and the TLS stack, so the agent carries neither. This module owns detection,
//! the cached profile list, one profile change at a time, the change's journal and read-back,
//! and the operator receipts a change leaves on the card.
//!
//! Rules this module keeps:
//! - Reads never write. Only `enable`, `disable` and `nickname` requests change the card, and
//!   receipts are sent only for changes made here (never the backlog found on the card).
//! - A profile write is never retried. An unanswered write is settled by reading the card back;
//!   if that is impossible the outcome stays "unknown". A restart reads back, never replays.
//! - Identifiers stay inside: clients get opaque handles and the last four digits; the EID is
//!   kept in memory and persisted only as a salted hash.
//! - Every time here is monotonic: the wall clock jumps when the network sets it after a switch.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::cache::Cached;
use crate::process::BoundedCommand;
use crate::util::MutexExt;

pub const HELPER_PATH: &str = "/data/zte-esim";
const STATE_PATH: &str = "/data/local/tmp/esim_state.json";
const JOURNAL_PATH: &str = "/data/local/tmp/esim_journal.json";
const HELPER_PROTOCOL: u64 = 2;
/// Card detection from the dashboard heartbeat runs at most this often.
const DETECT_TTL: Duration = Duration::from_secs(60);
/// A profile-list request re-reads the card at most this often.
const INVENTORY_TTL: Duration = Duration::from_secs(20);
const READ_DEADLINE: Duration = Duration::from_secs(15);
const WRITE_DEADLINE: Duration = Duration::from_secs(30);
const NOTIFY_DEADLINE: Duration = Duration::from_secs(120);
/// A download: authentication, the preview, the profile package and the install.
const DOWNLOAD_DEADLINE: Duration = Duration::from_secs(400);
const MAX_ACTIVATION_CODE: usize = 512;
const MAX_CONFIRMATION_CODE: usize = 64;
/// Time the helper gets after SIGTERM to close its logical channel.
const HELPER_GRACE: Duration = Duration::from_secs(5);
const HELPER_OUTPUT_LIMIT: usize = 256 * 1024;
const WARNINGS_TTL: Duration = Duration::from_secs(30);
/// Receipt rounds after a change: first try, then backoff. Unsent receipts stay on the card.
const RECEIPT_DELAYS: [Duration; 6] = [
    Duration::from_secs(30),
    Duration::from_secs(120),
    Duration::from_secs(300),
    Duration::from_secs(600),
    Duration::from_secs(1800),
    Duration::from_secs(3600),
];
const RECEIPT_MAX_ROUNDS: u32 = 24;

/// Waits around a switch. The card's management application is unreachable for 5-25 s while
/// the modem reloads the SIM.
#[derive(Clone, Copy)]
pub struct Timing {
    pub verify_delay: Duration,
    pub verify_interval: Duration,
    pub verify_window: Duration,
    /// After the data connection returns: the first request after an attach is unreliable.
    pub receipt_settle: Duration,
    pub wan_poll: Duration,
    /// After an eSIM change ends, how long an unreadable card still counts as reloading rather
    /// than absent.
    pub reload_window: Duration,
    /// How long a download's preview waits for Install or Cancel.
    pub preview_limit: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            verify_delay: Duration::from_secs(5),
            verify_interval: Duration::from_secs(3),
            verify_window: Duration::from_secs(90),
            receipt_settle: Duration::from_secs(5),
            wan_poll: Duration::from_secs(15),
            reload_window: Duration::from_secs(120),
            preview_limit: Duration::from_secs(120),
        }
    }
}

// ---------------------------------------------------------------------------
// The outside world
// ---------------------------------------------------------------------------

/// What one helper run produced.
#[derive(Debug, Clone)]
pub enum HelperOutcome {
    Ok(Value),
    Failed {
        kind: String,
        message: String,
        detail: Value,
    },
    /// No helper binary, or one that speaks another protocol.
    Missing(String),
    /// It outlived its deadline and grace and was killed: a card channel may be left open.
    Killed,
    /// No readable answer.
    Garbled(String),
}

/// Inputs for the pre-switch warnings, read from the firmware.
#[derive(Debug, Clone, Default)]
pub struct WarningInputs {
    pub manual_apn: bool,
    pub roaming_enabled: Option<bool>,
    pub sim_mcc: Option<String>,
    pub network_mcc: Option<String>,
}

/// One event from a running download.
#[derive(Debug)]
pub enum RunEvent {
    Progress {
        step: String,
        data: Value,
    },
    /// Nothing new before the caller's `until`.
    Wait,
    Done(HelperOutcome),
}

/// A download in progress: progress lines out, control lines in.
pub trait DownloadRun: Send {
    fn next(&mut self, until: Instant) -> RunEvent;
    /// Sends `{"control": <control>}`; false once the helper can no longer take it.
    fn send(&mut self, control: &str) -> bool;
}

pub trait Env: Send + Sync {
    fn helper(&self, request: &Value, deadline: Duration) -> HelperOutcome;
    /// Starts a download. The request carries the activation code, which goes nowhere else.
    fn start_download(&self, request: &Value, deadline: Duration) -> Box<dyn DownloadRun>;
    /// Changes when the SIM changes or reloads (state, home network, SPN). Read from ubus.
    fn sim_key(&self) -> Option<String>;
    /// The modem reports the SIM as ready (`sim_states: "sim ready"`).
    fn sim_ready(&self) -> bool;
    fn wan_up(&self) -> bool;
    fn warning_inputs(&self) -> WarningInputs;
    /// Drop cached SIM, radio and WAN readings after a change.
    fn invalidate_network(&self);
}

pub struct DeviceEnv {
    pub helper_path: PathBuf,
    pub node: u32,
    pub slot: u8,
    pub radio: Arc<crate::cache::Observed<Value>>,
    pub dash: Arc<crate::handlers::DashboardCache>,
}

impl Env for DeviceEnv {
    fn helper(&self, request: &Value, deadline: Duration) -> HelperOutcome {
        let mut request = request.clone();
        request["protocol"] = json!(HELPER_PROTOCOL);
        request["node"] = json!(self.node);
        request["slot"] = json!(self.slot);
        request["deadline_ms"] = json!(deadline.as_millis() as u64);
        // One request line; the helper reserves later lines for controlling long operations.
        let input = format!("{request}\n");
        // The helper stops itself at `deadline`; SIGTERM shortly after is the backstop.
        let output = Command::new(&self.helper_path).bounded_output_with_grace(
            input.as_bytes(),
            deadline + Duration::from_secs(2),
            HELPER_GRACE,
            HELPER_OUTPUT_LIMIT,
        );
        match output {
            Ok(out) => parse_helper_output(&out.stdout),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                HelperOutcome::Missing(format!("{} is not installed", self.helper_path.display()))
            }
            Err(e) if e.to_string() == process_runner::KILLED_AFTER_GRACE => HelperOutcome::Killed,
            Err(e) => HelperOutcome::Garbled(e.to_string()),
        }
    }

    fn start_download(&self, request: &Value, deadline: Duration) -> Box<dyn DownloadRun> {
        let mut request = request.clone();
        request["protocol"] = json!(HELPER_PROTOCOL);
        request["node"] = json!(self.node);
        request["slot"] = json!(self.slot);
        request["deadline_ms"] = json!(deadline.as_millis() as u64);
        let line = format!("{request}\n");
        drop(request);
        match Command::new(&self.helper_path).bounded_session(
            line.as_bytes(),
            deadline + Duration::from_secs(5),
            HELPER_GRACE,
        ) {
            Ok(session) => Box::new(SessionRun {
                session: Some(session),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Box::new(Finished(Some(
                HelperOutcome::Missing(format!("{} is not installed", self.helper_path.display())),
            ))),
            Err(e) => Box::new(Finished(Some(HelperOutcome::Garbled(e.to_string())))),
        }
    }

    fn sim_ready(&self) -> bool {
        crate::ubus::call("zwrt_zte_mdm.api", "get_sim_info", Some("{}"))
            .ok()
            .is_some_and(|v| v["sim_states"].as_str() == Some("sim ready"))
    }

    fn sim_key(&self) -> Option<String> {
        let v = crate::ubus::call("zwrt_zte_mdm.api", "get_sim_info", Some("{}")).ok()?;
        let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
        Some(format!(
            "{}|{}|{}|{}",
            s("sim_states"),
            s("mdm_mcc"),
            s("mdm_mnc"),
            s("spn_name_data")
        ))
    }

    fn wan_up(&self) -> bool {
        crate::ubus::call("network.interface.zte_wan", "status", Some("{}"))
            .ok()
            .is_some_and(|v| v["up"].as_bool() == Some(true))
    }

    fn warning_inputs(&self) -> WarningInputs {
        let apn = crate::ubus::call("zwrt_apn_object", "get_apn_mode", Some("{}")).ok();
        let wwan = crate::ubus::call(
            "zwrt_data",
            "get_wwaniface",
            Some(r#"{"source_module":"web","cid":1}"#),
        )
        .ok();
        let sim = crate::ubus::call("zwrt_zte_mdm.api", "get_sim_info", Some("{}")).ok();
        let radio = crate::handlers::read_radio(&self.radio).value;
        let text = |v: &Value| match v {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        };
        WarningInputs {
            manual_apn: apn.is_some_and(|v| v["apn_mode"].as_u64() == Some(1)),
            roaming_enabled: wwan.and_then(|v| v["roam_enable"].as_u64()).map(|r| r == 1),
            sim_mcc: sim.and_then(|v| text(&v["mdm_mcc"])),
            network_mcc: radio.and_then(|v| text(&v["rmcc"])),
        }
    }

    fn invalidate_network(&self) {
        self.radio.invalidate();
        self.dash.invalidate_network();
    }
}

/// A download run over a line session with the helper.
struct SessionRun {
    session: Option<process_runner::LineSession>,
}

impl DownloadRun for SessionRun {
    fn next(&mut self, until: Instant) -> RunEvent {
        let Some(session) = self.session.as_mut() else {
            return RunEvent::Done(HelperOutcome::Garbled("the download already ended".into()));
        };
        let event = match session.next_line(until) {
            Ok(process_runner::LineEvent::Wait) => return RunEvent::Wait,
            Ok(process_runner::LineEvent::Line(line)) => {
                match serde_json::from_str::<Value>(&line) {
                    Ok(v) if v.get("progress").is_some() => {
                        return RunEvent::Progress {
                            step: v["progress"].as_str().unwrap_or_default().to_string(),
                            data: v["data"].clone(),
                        }
                    }
                    _ => parse_helper_output(line.as_bytes()),
                }
            }
            Ok(process_runner::LineEvent::Eof) => {
                HelperOutcome::Garbled("the helper ended without an answer".into())
            }
            Err(e) if e.to_string() == process_runner::KILLED_AFTER_GRACE => HelperOutcome::Killed,
            Err(e) => HelperOutcome::Garbled(e.to_string()),
        };
        // The answer is the last line; dropping the session waits for the exit (with grace).
        self.session = None;
        RunEvent::Done(event)
    }

    fn send(&mut self, control: &str) -> bool {
        self.session.as_mut().is_some_and(|s| {
            s.send_line(&json!({ "control": control }).to_string())
                .is_ok()
        })
    }
}

/// A run that never started.
struct Finished(Option<HelperOutcome>);

impl DownloadRun for Finished {
    fn next(&mut self, _until: Instant) -> RunEvent {
        RunEvent::Done(
            self.0
                .take()
                .unwrap_or_else(|| HelperOutcome::Garbled("the download already ended".into())),
        )
    }
    fn send(&mut self, _control: &str) -> bool {
        false
    }
}

/// The helper prints one JSON line: `{"ok":true,"protocol":1,"data":…}` or
/// `{"ok":false,"protocol":1,"error":{kind,message,detail},…}`.
pub fn parse_helper_output(stdout: &[u8]) -> HelperOutcome {
    let text = String::from_utf8_lossy(stdout);
    let Some(line) = text.lines().rev().find(|l| !l.trim().is_empty()) else {
        return HelperOutcome::Garbled("the helper printed nothing".into());
    };
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return HelperOutcome::Garbled("the helper's answer is not JSON".into());
    };
    if v["protocol"].as_u64() != Some(HELPER_PROTOCOL) {
        return HelperOutcome::Missing("the installed eSIM helper is incompatible".into());
    }
    if v["channel"]["leak_suspected"].as_bool() == Some(true) {
        eprintln!(
            "[esim] the helper may have left logical channel {} open (it clears when the SIM resets)",
            v["channel"]["last"]
        );
    }
    if v["ok"].as_bool() == Some(true) {
        HelperOutcome::Ok(v["data"].clone())
    } else {
        let e = &v["error"];
        let kind = e["kind"].as_str().unwrap_or("transport").to_string();
        if kind == "invalid_request" {
            return HelperOutcome::Missing(format!(
                "the eSIM helper rejected the request: {}",
                e["message"].as_str().unwrap_or_default()
            ));
        }
        HelperOutcome::Failed {
            kind,
            message: e["message"].as_str().unwrap_or_default().to_string(),
            detail: e["detail"].clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Card data
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct CardProfile {
    pub iccid: String,
    pub handle: String,
    pub state: String,
    pub class: String,
    pub nickname: Option<String>,
    pub provider: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Inventory {
    /// In memory only.
    eid: String,
    info: Value,
    profiles: Vec<CardProfile>,
    notifications: Vec<u64>,
    observed: Instant,
}

impl Inventory {
    fn enabled(&self) -> Option<&CardProfile> {
        self.profiles.iter().find(|p| p.state == "enabled")
    }
    fn find(&self, handle: &str) -> Option<&CardProfile> {
        self.profiles.iter().find(|p| p.handle == handle)
    }
    /// What a client's revision covers: the card and its profiles' states and names.
    fn same_content(&self, other: &Inventory) -> bool {
        self.eid == other.eid && self.profiles == other.profiles
    }
}

fn opt_text(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn parse_inventory(data: &Value, salt: &[u8]) -> Option<Inventory> {
    let eid = data["eid"].as_str()?.to_string();
    let profiles = data["profiles"]
        .as_array()?
        .iter()
        .filter_map(|p| {
            let iccid = p["iccid"].as_str()?.to_string();
            Some(CardProfile {
                handle: handle_for(salt, &iccid),
                iccid,
                state: p["state"].as_str().unwrap_or("unknown").to_string(),
                class: p["class"].as_str().unwrap_or("operational").to_string(),
                nickname: opt_text(&p["nickname"]),
                provider: opt_text(&p["provider"]),
                name: opt_text(&p["name"]),
            })
        })
        .collect();
    let notifications = data["notifications"]
        .as_array()
        .map(|a| a.iter().filter_map(|n| n["seq"].as_u64()).collect())
        .unwrap_or_default();
    Some(Inventory {
        eid,
        info: data["info"].clone(),
        profiles,
        notifications,
        observed: Instant::now(),
    })
}

fn salted(salt: &[u8], value: &str) -> String {
    let mut h = Sha256::new();
    h.update(salt);
    h.update(b":");
    h.update(value.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// A stable, opaque profile handle for clients.
pub fn handle_for(salt: &[u8], iccid: &str) -> String {
    format!("p{}", &salted(salt, &format!("iccid:{iccid}"))[..16])
}

fn card_hash(salt: &[u8], eid: &str) -> String {
    salted(salt, &format!("eid:{}", eid.to_ascii_lowercase()))[..32].to_string()
}

/// `…1234`: the last four digits, as everywhere in the UI and the log.
pub fn mask(id: &str) -> String {
    let tail: String = id
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("…{tail}")
}

/// Names that are really identifiers (some operators put the ICCID in the profile name).
pub fn identifier_like(text: &str, iccid: &str, eid: &str) -> bool {
    let t = text.trim();
    let compact: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    (compact.len() >= 6 && compact.bytes().all(|b| b.is_ascii_digit()))
        || (compact.len() >= 16 && compact.bytes().all(|b| b.is_ascii_hexdigit()))
        || (!iccid.is_empty() && compact.contains(iccid))
        || (!eid.is_empty()
            && compact
                .to_ascii_lowercase()
                .contains(&eid.to_ascii_lowercase()))
}

/// E.164 country code from an ICCID (`89` + country code + issuer).
pub fn iccid_country(iccid: &str) -> Option<&str> {
    let rest = iccid.strip_prefix("89")?;
    // North American issuers write their code as "01".
    let rest = rest.strip_prefix('0').unwrap_or(rest);
    let two = rest.get(..2)?;
    let len = match rest.as_bytes().first()? {
        b'1' | b'7' => 1,
        _ if matches!(
            two,
            "20" | "27"
                | "30"
                | "31"
                | "32"
                | "33"
                | "34"
                | "36"
                | "39"
                | "40"
                | "41"
                | "43"
                | "44"
                | "45"
                | "46"
                | "47"
                | "48"
                | "49"
                | "51"
                | "52"
                | "53"
                | "54"
                | "55"
                | "56"
                | "57"
                | "58"
                | "60"
                | "61"
                | "62"
                | "63"
                | "64"
                | "65"
                | "66"
                | "81"
                | "82"
                | "84"
                | "86"
                | "90"
                | "91"
                | "92"
                | "93"
                | "94"
                | "95"
                | "98"
        ) =>
        {
            2
        }
        _ => 3,
    };
    rest.get(..len)
}

/// Read-only warnings shown before activating `target`.
pub fn switch_warnings(
    target: &CardProfile,
    active: Option<&CardProfile>,
    inputs: &WarningInputs,
) -> Vec<&'static str> {
    let mut w = Vec::new();
    if inputs.manual_apn {
        w.push("manual_apn");
    }
    // Roaming: the target's issuing country differs from a profile that is at home here.
    let at_home = inputs.sim_mcc.is_some() && inputs.sim_mcc == inputs.network_mcc;
    if inputs.roaming_enabled == Some(false) && at_home {
        if let (Some(a), Some(t)) = (
            active.and_then(|a| iccid_country(&a.iccid)),
            iccid_country(&target.iccid),
        ) {
            if a != t {
                w.push("roaming_off");
            }
        }
    }
    w
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Not looked yet.
    Detecting,
    Ready,
    /// The SIM has no eUICC management application (an ordinary SIM).
    Unsupported,
    /// The modem's SIM service did not answer.
    Unavailable,
    HelperMissing,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Detecting => "detecting",
            Status::Ready => "ready",
            Status::Unsupported => "unsupported",
            Status::Unavailable => "unavailable",
            Status::HelperMissing => "helper_missing",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Enable,
    Disable,
    Nickname,
    Download,
    Delete,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Enable => "enable",
            Kind::Disable => "disable",
            Kind::Nickname => "nickname",
            Kind::Download => "download",
            Kind::Delete => "delete",
        }
    }
    fn parse(s: &str) -> Option<Kind> {
        match s {
            "enable" => Some(Kind::Enable),
            "disable" => Some(Kind::Disable),
            "nickname" => Some(Kind::Nickname),
            "download" => Some(Kind::Download),
            "delete" => Some(Kind::Delete),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpState {
    /// Sent to the card.
    Pending,
    /// Accepted; the modem is reloading the SIM.
    Reloading,
    /// The card's answer was lost (or the agent restarted): reading it back.
    Verifying,
    Succeeded,
    /// Refused because the card is busy (SIM Toolkit). Nothing changed.
    Busy,
    /// Refused by the card or because the selection was stale. Nothing changed.
    Refused,
    Failed,
    /// Could not be established; the card may or may not have changed.
    Unknown,
    /// Download: talking to the operator's server (cancellable).
    Connecting,
    /// Download: the preview is shown and waits for Install or Cancel.
    AwaitingConfirmation,
    /// Download: the card is installing the profile; nothing cancels it.
    Installing,
    /// Download: declined, cancelled or not confirmed in time; nothing installed.
    Cancelled,
}

impl OpState {
    fn as_str(self) -> &'static str {
        match self {
            OpState::Pending => "pending",
            OpState::Reloading => "reloading",
            OpState::Verifying => "verifying",
            OpState::Succeeded => "succeeded",
            OpState::Busy => "busy",
            OpState::Refused => "refused",
            OpState::Failed => "failed",
            OpState::Unknown => "unknown",
            OpState::Connecting => "connecting",
            OpState::AwaitingConfirmation => "awaiting_confirmation",
            OpState::Installing => "installing",
            OpState::Cancelled => "cancelled",
        }
    }
    fn resolved(self) -> bool {
        !matches!(
            self,
            OpState::Pending
                | OpState::Reloading
                | OpState::Verifying
                | OpState::Connecting
                | OpState::AwaitingConfirmation
                | OpState::Installing
        )
    }
}

#[derive(Debug, Clone)]
pub struct Operation {
    pub id: String,
    pub kind: Kind,
    pub target: String,
    pub previous: Option<String>,
    pub state: OpState,
    pub message: Option<String>,
    /// Receipts already on the card before the change; only newer ones are sent.
    receipt_before: Vec<u64>,
    /// Download: the helper's last step, the server's preview (provider, name, class) and when
    /// the preview expires.
    step: Option<String>,
    preview: Option<Value>,
    preview_until: Option<Instant>,
    /// Download: the nickname to write after the install, and the way to reach the run.
    nickname: Option<String>,
    control: Option<mpsc::Sender<&'static str>>,
    /// Download: the handles on the card before, to recognise the new profile on read-back.
    profiles_before: Vec<String>,
}

impl Operation {
    fn new(id: &str, kind: Kind, target: &str, previous: Option<String>) -> Self {
        Operation {
            id: id.to_string(),
            kind,
            target: target.to_string(),
            previous,
            state: OpState::Pending,
            message: None,
            receipt_before: Vec::new(),
            step: None,
            preview: None,
            preview_until: None,
            nickname: None,
            control: None,
            profiles_before: Vec::new(),
        }
    }
}

#[derive(Default)]
struct Receipts {
    pending: Vec<u64>,
    round: u32,
    worker: bool,
    last_error: Option<String>,
    gave_up: bool,
}

struct Inner {
    salt: Vec<u8>,
    card: Option<String>,
    locked: BTreeSet<String>,
    status: Status,
    diagnostic: Option<String>,
    detect_running: bool,
    detected_at: Option<Instant>,
    sim_key: Option<String>,
    inventory: Option<Inventory>,
    revision: u64,
    read_error: Option<(String, String)>,
    read_attempt: Option<Instant>,
    operation: Option<Operation>,
    /// When the last operation ended (for the reload window).
    finished_at: Option<Instant>,
    receipts: Receipts,
}

/// An admitted change: the operation, its target, the active profile and the card's EID.
type Admitted = (Operation, CardProfile, Option<CardProfile>, String);

pub struct EsimManager {
    env: Arc<dyn Env>,
    inner: Mutex<Inner>,
    /// Held across every helper run: one card session at a time.
    card: Mutex<()>,
    state_path: PathBuf,
    journal_path: PathBuf,
    timing: Timing,
    warnings: Cached<WarningInputs>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

fn random_salt() -> Vec<u8> {
    use std::io::Read;
    let mut salt = vec![0u8; 16];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut salt))
        .is_err()
    {
        // Not secret, only unlinkable across devices: fall back to time and pid.
        let seed = format!("{:?}{}", std::time::SystemTime::now(), std::process::id());
        salt = Sha256::digest(seed.as_bytes())[..16].to_vec();
    }
    salt
}

impl EsimManager {
    pub fn new(env: Arc<dyn Env>) -> Arc<Self> {
        Self::with_paths(
            env,
            PathBuf::from(STATE_PATH),
            PathBuf::from(JOURNAL_PATH),
            Timing::default(),
        )
    }

    pub fn with_paths(
        env: Arc<dyn Env>,
        state_path: PathBuf,
        journal_path: PathBuf,
        timing: Timing,
    ) -> Arc<Self> {
        let saved: Value = std::fs::read(&state_path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Value::Null);
        let salt = saved["salt"]
            .as_str()
            .and_then(unhex)
            .filter(|s| s.len() == 16)
            .unwrap_or_else(random_salt);
        let inner = Inner {
            salt,
            card: opt_text(&saved["card"]),
            locked: saved["locked"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|h| h.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            status: Status::Detecting,
            diagnostic: None,
            detect_running: false,
            detected_at: None,
            sim_key: None,
            inventory: None,
            revision: 0,
            read_error: None,
            read_attempt: None,
            operation: None,
            finished_at: None,
            receipts: Receipts {
                pending: saved["receipts"]
                    .as_array()
                    .map(|a| a.iter().filter_map(Value::as_u64).collect())
                    .unwrap_or_default(),
                ..Default::default()
            },
        };
        // The tab appears only after a successful read, even for a card seen before: the slot may
        // hold an ordinary SIM now.
        let manager = Arc::new(EsimManager {
            env,
            inner: Mutex::new(inner),
            card: Mutex::new(()),
            state_path,
            journal_path,
            timing,
            warnings: Cached::default(),
        });
        manager.save_state();
        manager
    }

    fn save_state(&self) {
        let snapshot = {
            let inner = self.inner.safe_lock();
            json!({
                "version": 1,
                "salt": hex(&inner.salt),
                "card": inner.card,
                "locked": inner.locked,
                "receipts": inner.receipts.pending,
            })
        };
        if let Err(e) =
            crate::storage::atomic_write(&self.state_path, snapshot.to_string().as_bytes())
        {
            eprintln!("[esim] saving state failed: {e}");
        }
    }

    // -- journal ------------------------------------------------------------

    fn write_journal(&self, op: &Operation) -> std::io::Result<()> {
        let j = json!({
            "version": 1,
            "id": op.id,
            "kind": op.kind.as_str(),
            "target": op.target,
            "previous": op.previous,
            "receipt_before": op.receipt_before,
            "profiles_before": op.profiles_before,
        });
        crate::storage::atomic_write(&self.journal_path, j.to_string().as_bytes())
    }

    fn clear_journal(&self) {
        if self.journal_path.exists() {
            if let Err(e) = crate::storage::durable_remove(&self.journal_path) {
                eprintln!("[esim] clearing the operation journal failed: {e}");
            }
        }
    }

    /// After a restart: an operation that was in flight is read back, never sent again.
    pub fn recover(self: &Arc<Self>) {
        let journal: Option<Value> = std::fs::read(&self.journal_path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let pending = !self.inner.safe_lock().receipts.pending.is_empty();
        if pending {
            self.start_receipt_worker();
        }
        let Some(j) = journal else { return };
        let (Some(kind), Some(id)) = (j["kind"].as_str().and_then(Kind::parse), opt_text(&j["id"]))
        else {
            self.clear_journal();
            return;
        };
        let target = opt_text(&j["target"]).unwrap_or_default();
        if target.is_empty() && kind != Kind::Download {
            self.clear_journal();
            return;
        }
        eprintln!(
            "[esim] an {} was in flight at restart; reading the card back",
            kind.as_str()
        );
        let mut op = Operation::new(&id, kind, &target, opt_text(&j["previous"]));
        op.state = OpState::Verifying;
        op.message = Some("The agent restarted during this change; checking the card.".into());
        op.receipt_before = j["receipt_before"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_u64).collect())
            .unwrap_or_default();
        op.profiles_before = j["profiles_before"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|h| h.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        self.inner.safe_lock().operation = Some(op);
        let me = Arc::clone(self);
        if kind == Kind::Download {
            // An interrupted download is never resumed: a new profile on the card is the only
            // sign it finished. The helper cancels on its own when its stdin closes.
            std::thread::spawn(move || me.settle_download(false));
        } else {
            std::thread::spawn(move || me.verify(kind, false));
        }
    }

    // -- reads --------------------------------------------------------------

    /// One inventory read; updates detection and the cached list.
    fn read_card(&self) -> Result<(), (String, String)> {
        let _card = self.card.safe_lock();
        let outcome = self.env.helper(&json!({"op": "inventory"}), READ_DEADLINE);
        self.absorb_read(outcome)
    }

    fn absorb_read(&self, outcome: HelperOutcome) -> Result<(), (String, String)> {
        // An ordinary SIM and an eSIM card that is reloading after a switch give the same answer
        // (no eSIM management application, QMI error 80). Only around an eSIM change can it be a
        // reload; otherwise a ready SIM without one is not an eSIM card. Asked before the lock.
        let no_isdr =
            matches!(&outcome, HelperOutcome::Failed { kind, .. } if kind == "isdr_unavailable");
        let sim_ready = no_isdr && self.env.sim_ready();
        let mut inner = self.inner.safe_lock();
        inner.read_attempt = Some(Instant::now());
        let error = match outcome {
            HelperOutcome::Ok(data) => match parse_inventory(&data, &inner.salt) {
                Some(inv) => {
                    let hash = card_hash(&inner.salt, &inv.eid);
                    let changed = inner
                        .inventory
                        .as_ref()
                        .is_none_or(|old| !old.same_content(&inv));
                    if changed {
                        inner.revision += 1;
                    }
                    let new_card = inner.card.as_deref() != Some(hash.as_str());
                    if new_card {
                        eprintln!("[esim] eUICC card {} detected", mask(&inv.eid));
                        inner.card = Some(hash);
                        // Receipts belong to the card they were found on.
                        inner.receipts = Receipts {
                            worker: inner.receipts.worker,
                            ..Receipts::default()
                        };
                    }
                    inner.inventory = Some(inv);
                    inner.status = Status::Ready;
                    inner.diagnostic = None;
                    inner.read_error = None;
                    drop(inner);
                    if new_card {
                        self.save_state();
                    }
                    return Ok(());
                }
                None => (
                    "read_failed".to_string(),
                    "unreadable profile list".to_string(),
                ),
            },
            HelperOutcome::Failed { kind, message, .. } => {
                let known_card = inner.card.is_some();
                match kind.as_str() {
                    "lock_busy" => ("busy".into(), message),
                    "isdr_unavailable" => {
                        let in_change = inner
                            .operation
                            .as_ref()
                            .is_some_and(|o| !o.state.resolved())
                            || inner
                                .finished_at
                                .is_some_and(|t| t.elapsed() < self.timing.reload_window);
                        if known_card && (in_change || !sim_ready) {
                            ("sim_reloading".into(), message)
                        } else {
                            if inner.status != Status::Unsupported {
                                eprintln!("[esim] the SIM in the slot is not an eSIM card");
                            }
                            // The card and its locks are remembered for when it comes back.
                            inner.status = Status::Unsupported;
                            inner.diagnostic = Some(
                                "The SIM in the slot has no eSIM management application.".into(),
                            );
                            if inner.inventory.take().is_some() {
                                inner.revision += 1;
                            }
                            inner.read_error = None;
                            return Err(("unsupported".into(), message));
                        }
                    }
                    "management_unavailable" | "transport" | "cancelled" => {
                        if !known_card {
                            inner.status = Status::Unavailable;
                            inner.diagnostic = Some(message.clone());
                        }
                        ("management_unavailable".into(), message)
                    }
                    _ => ("read_failed".into(), message),
                }
            }
            HelperOutcome::Missing(m) => {
                inner.status = Status::HelperMissing;
                inner.diagnostic = Some(m.clone());
                ("helper_missing".into(), m)
            }
            HelperOutcome::Killed => (
                "read_failed".into(),
                "the card read was stopped at its deadline".into(),
            ),
            HelperOutcome::Garbled(m) => ("read_failed".into(), m),
        };
        inner.read_error = Some(error.clone());
        Err(error)
    }

    /// The dashboard heartbeat's summary. Detection runs in the background at most once per
    /// minute, and not again for an ordinary SIM until the SIM changes.
    pub fn summary(self: &Arc<Self>) -> Value {
        let mut inner = self.inner.safe_lock();
        let due = !inner.detect_running
            && inner.operation.as_ref().is_none_or(|o| o.state.resolved())
            && inner.detected_at.is_none_or(|t| t.elapsed() >= DETECT_TTL);
        if due {
            inner.detect_running = true;
            inner.detected_at = Some(Instant::now());
            let me = Arc::clone(self);
            std::thread::spawn(move || me.detect());
        }
        json!({
            "status": inner.status.as_str(),
            "available": inner.status == Status::Ready,
        })
    }

    fn detect(&self) {
        let key = self.env.sim_key();
        let skip = {
            let inner = self.inner.safe_lock();
            let same_sim = key.is_some() && key == inner.sim_key;
            // An ordinary SIM stays unsupported until the SIM changes; a known card is
            // re-read by the tab itself.
            same_sim
                && matches!(inner.status, Status::Unsupported | Status::Ready)
                && (inner.status == Status::Unsupported || inner.inventory.is_some())
        };
        if !skip {
            let _ = self.read_card();
        }
        let mut inner = self.inner.safe_lock();
        inner.sim_key = key;
        inner.detect_running = false;
    }

    /// GET /api/esim/profiles
    pub fn profiles(&self) -> (u16, Value) {
        let read_due = {
            let inner = self.inner.safe_lock();
            let busy = inner
                .operation
                .as_ref()
                .is_some_and(|o| !o.state.resolved());
            !busy
                && inner.status != Status::Unsupported
                && inner.status != Status::HelperMissing
                && inner
                    .read_attempt
                    .is_none_or(|t| t.elapsed() >= INVENTORY_TTL)
        };
        if read_due {
            // Never queue behind another card session; serve the cached list instead.
            if let Ok(_card) = self.card.try_lock() {
                let outcome = self.env.helper(&json!({"op": "inventory"}), READ_DEADLINE);
                let _ = self.absorb_read(outcome);
            }
        }
        (200, json!({"ok": true, "data": self.view()}))
    }

    /// POST /api/esim/refresh: a serialized fresh read.
    pub fn refresh(&self) -> (u16, Value) {
        let busy = self
            .inner
            .safe_lock()
            .operation
            .as_ref()
            .is_some_and(|o| !o.state.resolved());
        if !busy {
            let _ = self.read_card();
        }
        (200, json!({"ok": true, "data": self.view()}))
    }

    fn view(&self) -> Value {
        let inputs = self
            .warnings
            .get_or_refresh(WARNINGS_TTL, || self.env.warning_inputs());
        let inner = self.inner.safe_lock();
        let inv = inner.inventory.as_ref();
        let active = inv.and_then(Inventory::enabled);
        let eid = inv.map(|i| i.eid.as_str()).unwrap_or_default();
        let mut profiles: Vec<Value> = inv
            .map(|i| {
                i.profiles
                    .iter()
                    .map(|p| {
                        let clean = |t: &Option<String>| {
                            t.as_deref()
                                .filter(|t| !identifier_like(t, &p.iccid, eid))
                                .map(str::to_string)
                        };
                        let nickname = clean(&p.nickname);
                        let name = clean(&p.name);
                        let provider = clean(&p.provider);
                        let display = nickname.clone().or(name.clone()).or(provider.clone());
                        let warnings = if p.state == "disabled" && p.class == "operational" {
                            switch_warnings(p, active, &inputs)
                        } else {
                            Vec::new()
                        };
                        json!({
                            "handle": p.handle,
                            "display_name": display,
                            "nickname": nickname,
                            "name": name,
                            "provider": provider,
                            "iccid_masked": mask(&p.iccid),
                            "state": p.state,
                            "class": p.class,
                            "locked": inner.locked.contains(&p.handle),
                            "warnings": warnings,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Active first, then by name; unnamed last.
        profiles.sort_by_key(|p| {
            (
                p["state"] != "enabled",
                p["display_name"].is_null(),
                p["display_name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_lowercase(),
            )
        });
        let op_busy = inner
            .operation
            .as_ref()
            .is_some_and(|o| !o.state.resolved());
        let block = if inner.status == Status::HelperMissing {
            Some("helper_missing")
        } else if inv.is_none() {
            Some("no_inventory")
        } else if op_busy {
            Some("operation_in_progress")
        } else {
            None
        };
        let card = inv.map(|i| {
            json!({
                "id": mask(&i.eid),
                "firmware": i.info["firmware"],
                "sgp22_version": i.info["sgp22_version"],
                "profile_version": i.info["profile_version"],
                "free_nvm_bytes": i.info["free_nvm_bytes"],
                "free_ram_bytes": i.info["free_ram_bytes"],
                "pending_receipts": i.notifications.len(),
            })
        });
        json!({
            "status": inner.status.as_str(),
            "available": inner.status == Status::Ready,
            "diagnostic": inner.diagnostic,
            "card": card,
            "profiles": profiles,
            "revision": inner.revision,
            "freshness": {
                "age_ms": inv.map(|i| i.observed.elapsed().as_millis() as u64),
                "stale": inner.read_error.is_some(),
                "error": inner.read_error.as_ref().map(|(k, m)| json!({"kind": k, "message": m})),
            },
            "operation": inner.operation.as_ref().map(|o| {
                let label = |h: &str| inv.and_then(|i| i.find(h)).map(|p| {
                    let clean = |t: &Option<String>| t.as_deref()
                        .filter(|t| !identifier_like(t, &p.iccid, eid)).map(str::to_string);
                    clean(&p.nickname).or(clean(&p.name)).or(clean(&p.provider))
                        .unwrap_or_else(|| mask(&p.iccid))
                });
                json!({
                    "id": o.id,
                    "kind": o.kind.as_str(),
                    "target": (!o.target.is_empty()).then_some(&o.target),
                    "target_name": label(&o.target),
                    "previous": o.previous,
                    "state": o.state.as_str(),
                    "resolved": o.state.resolved(),
                    "message": o.message,
                    "step": o.step,
                    "preview": o.preview,
                    "can_cancel": o.kind == Kind::Download
                        && matches!(o.state, OpState::Connecting | OpState::AwaitingConfirmation),
                    "preview_expires_in_ms": o.preview_until
                        .filter(|_| o.state == OpState::AwaitingConfirmation)
                        .map(|t| t.saturating_duration_since(Instant::now()).as_millis() as u64),
                })
            }),
            "receipts": {
                "pending": inner.receipts.pending.len(),
                "last_error": inner.receipts.last_error,
                "gave_up": inner.receipts.gave_up,
                "sending": inner.receipts.worker,
            },
            "actions": {"enabled": block.is_none(), "blocked_by": block},
            "hardware_validated": {"switching": true},
        })
    }

    // -- writes -------------------------------------------------------------

    /// Common checks for a change request; returns the operation to start or an answer.
    fn admit(&self, kind: Kind, body: &Value) -> Result<Admitted, (u16, Value)> {
        let err = |code: u16, msg: &str| (code, json!({"ok": false, "error": msg}));
        let id = body["operation_id"]
            .as_str()
            .filter(|s| valid_operation_id(s))
            .ok_or_else(|| err(400, "operation_id must be 8-64 letters, digits, '-' or '_'"))?;
        let handle = body["handle"]
            .as_str()
            .ok_or_else(|| err(400, "missing handle"))?;
        let revision = body["revision"]
            .as_u64()
            .ok_or_else(|| err(400, "missing revision"))?;
        let mut inner = self.inner.safe_lock();
        // A repeated request gets the operation it already started.
        if let Some(op) = inner.operation.as_ref().filter(|o| o.id == id) {
            if op.kind == kind && op.target == handle {
                drop(inner);
                return Err((200, json!({"ok": true, "data": self.view()})));
            }
            return Err(err(409, "operation_id was already used for another change"));
        }
        if inner
            .operation
            .as_ref()
            .is_some_and(|o| !o.state.resolved())
        {
            return Err(err(409, "another eSIM change is still in progress"));
        }
        if inner.status == Status::HelperMissing {
            return Err(err(
                503,
                "eSIM management is unavailable: the helper is missing",
            ));
        }
        let inv = inner
            .inventory
            .as_ref()
            .ok_or_else(|| err(503, "the profile list has not been read yet"))?;
        if revision != inner.revision {
            return Err(err(409, "the profile list changed; refresh and try again"));
        }
        let target = inv
            .find(handle)
            .cloned()
            .ok_or_else(|| err(404, "no such profile"))?;
        let active = inv.enabled().cloned();
        match kind {
            Kind::Enable => {
                if target.class != "operational" {
                    return Err(err(409, "only operational profiles can be activated"));
                }
                if target.state != "disabled" {
                    return Err(err(409, "the profile is not inactive"));
                }
                if inner.locked.contains(handle) {
                    return Err(err(423, "the profile is locked against activation"));
                }
            }
            Kind::Disable => {
                if target.state != "enabled" {
                    return Err(err(409, "the profile is not active"));
                }
            }
            Kind::Delete => {
                if inner.locked.contains(handle) {
                    return Err(err(423, "locked profiles can't be deleted"));
                }
                if target.state != "disabled" {
                    return Err(err(409, "only an inactive profile can be deleted"));
                }
            }
            Kind::Nickname | Kind::Download => {}
        }
        let previous = match kind {
            Kind::Enable | Kind::Disable => active.as_ref().map(|a| a.handle.clone()),
            _ => None,
        };
        let mut op = Operation::new(id, kind, handle, previous);
        op.receipt_before = inv.notifications.clone();
        let eid = inv.eid.clone();
        // Claimed under the same lock as the checks, so two requests cannot both be admitted.
        inner.operation = Some(op.clone());
        Ok((op, target, active, eid))
    }

    /// POST /api/esim/enable and /api/esim/disable: accepted promptly, settled in the
    /// background.
    pub fn switch(self: &Arc<Self>, kind: Kind, body: &[u8]) -> (u16, Value) {
        let body: Value = match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(_) => return (400, json!({"ok": false, "error": "invalid JSON"})),
        };
        let (op, target, active, eid) = match self.admit(kind, &body) {
            Ok(x) => x,
            Err(answer) => return answer,
        };
        if let Err(e) = self.write_journal(&op) {
            self.inner.safe_lock().operation = None;
            return (
                503,
                json!({"ok": false, "error": format!("cannot record the change: {e}")}),
            );
        }
        eprintln!(
            "[esim] {} of {} requested (operation {})",
            kind.as_str(),
            mask(&target.iccid),
            op.id
        );
        let request = match kind {
            Kind::Delete => json!({"op": "delete", "iccid": target.iccid, "expect_eid": eid}),
            _ => json!({
                "op": kind.as_str(),
                "iccid": target.iccid,
                "expect_eid": eid,
                "expect_enabled": active.map(|a| a.iccid),
            }),
        };
        let me = Arc::clone(self);
        std::thread::spawn(move || me.run_switch(kind, request));
        (202, json!({"ok": true, "data": self.view()}))
    }

    fn set_op(&self, state: OpState, message: Option<String>) {
        let mut inner = self.inner.safe_lock();
        if let Some(op) = inner.operation.as_mut() {
            op.state = state;
            op.message = message;
            if state.resolved() {
                eprintln!(
                    "[esim] {} {}: {}",
                    op.kind.as_str(),
                    state.as_str(),
                    op.message.as_deref().unwrap_or("")
                );
            }
        }
    }

    fn finish(&self, state: OpState, message: Option<String>) {
        self.set_op(state, message);
        self.inner.safe_lock().finished_at = Some(Instant::now());
        self.clear_journal();
    }

    fn run_switch(self: Arc<Self>, kind: Kind, request: Value) {
        let outcome = {
            let _card = self.card.safe_lock();
            self.env.helper(&request, WRITE_DEADLINE)
        };
        match outcome {
            HelperOutcome::Ok(_) if kind == Kind::Delete => {
                self.set_op(
                    OpState::Verifying,
                    Some("The card accepted the deletion; reading the card back.".into()),
                );
                self.verify(kind, true);
            }
            HelperOutcome::Ok(_) => {
                self.set_op(
                    OpState::Reloading,
                    Some("The card accepted the change; the modem is reloading the SIM.".into()),
                );
                self.env.invalidate_network();
                self.verify(kind, true);
            }
            HelperOutcome::Failed {
                kind: k,
                message,
                detail,
            } => match k.as_str() {
                "refused" if kind != Kind::Delete && detail["code"].as_u64() == Some(5) => self
                    .finish(
                        OpState::Busy,
                        Some(
                            "The card is busy (its SIM Toolkit is in a session). Nothing changed; \
                         it usually accepts a retry within a few minutes. Right after the last \
                         profile was deactivated this took about 4 minutes on the tested card."
                                .into(),
                        ),
                    ),
                "refused" => self.finish(
                    OpState::Refused,
                    Some(format!(
                        "The card refused the change ({message}). Nothing changed."
                    )),
                ),
                "state_changed" | "card_changed" => {
                    self.finish(
                        OpState::Refused,
                        Some(format!("{message}. Nothing changed.")),
                    );
                    let _ = self.read_card();
                }
                "outcome_unknown" => {
                    self.set_op(
                        OpState::Verifying,
                        Some("The card's answer was lost; reading the card back.".into()),
                    );
                    self.env.invalidate_network();
                    self.verify(kind, false);
                }
                "lock_busy" => self.finish(
                    OpState::Failed,
                    Some("Another eSIM operation held the card. Nothing changed.".into()),
                ),
                _ => self.finish(
                    OpState::Failed,
                    Some(format!("The change was not sent ({message}).")),
                ),
            },
            HelperOutcome::Killed | HelperOutcome::Garbled(_) => {
                self.set_op(
                    OpState::Verifying,
                    Some(
                        "The change may or may not have reached the card; reading it back.".into(),
                    ),
                );
                self.env.invalidate_network();
                self.verify(kind, false);
            }
            HelperOutcome::Missing(m) => {
                self.inner.safe_lock().status = Status::HelperMissing;
                self.finish(OpState::Failed, Some(m));
            }
        }
    }

    /// Reads the card back until the change shows, the other outcome shows, or the window
    /// closes. `accepted`: the card answered the write with success.
    fn verify(self: &Arc<Self>, kind: Kind, accepted: bool) {
        let t = self.timing;
        std::thread::sleep(t.verify_delay);
        let start = Instant::now();
        loop {
            let read = self.read_card();
            if read.is_ok() {
                let (target_state, receipts_new) = {
                    let inner = self.inner.safe_lock();
                    let op = inner.operation.as_ref();
                    let inv = inner.inventory.as_ref();
                    let state = op
                        .zip(inv)
                        .and_then(|(o, i)| i.find(&o.target))
                        .map(|p| p.state.clone());
                    let new: Vec<u64> = match (op, inv) {
                        (Some(o), Some(i)) => i
                            .notifications
                            .iter()
                            .copied()
                            .filter(|s| !o.receipt_before.contains(s))
                            .collect(),
                        _ => Vec::new(),
                    };
                    (state, new)
                };
                let done = match kind {
                    Kind::Enable => target_state.as_deref() == Some("enabled"),
                    Kind::Delete => target_state.is_none(),
                    _ => target_state.as_deref() == Some("disabled"),
                };
                if done {
                    self.finish(
                        OpState::Succeeded,
                        Some(match kind {
                            Kind::Enable => "The profile is active on the card. Connectivity is \
                                             separate and can take a minute."
                                .into(),
                            Kind::Delete => "The profile was deleted from the card.".into(),
                            _ => "No profile is active. Cellular service has stopped.".into(),
                        }),
                    );
                    if kind != Kind::Delete {
                        self.env.invalidate_network();
                    }
                    self.queue_change_receipts(receipts_new);
                    return;
                }
                match target_state.as_deref() {
                    Some(_) if !accepted => {
                        self.finish(
                            OpState::Failed,
                            Some("The card shows the change was not applied.".into()),
                        );
                        return;
                    }
                    None => {
                        self.finish(
                            OpState::Unknown,
                            Some("The profile is no longer on the card.".into()),
                        );
                        return;
                    }
                    // Accepted but not visible yet: keep reading.
                    Some(_) => {}
                }
            } else if let Err((k, _)) = &read {
                if kind != Kind::Delete
                    && (k == "sim_reloading" || k == "management_unavailable" || k == "busy")
                {
                    self.set_op(
                        if accepted {
                            OpState::Reloading
                        } else {
                            OpState::Verifying
                        },
                        Some("The modem is reloading the SIM; the card can't be read yet.".into()),
                    );
                }
            }
            if start.elapsed() >= t.verify_window {
                self.finish(
                    OpState::Unknown,
                    Some(
                        "The card could not be read back. The change may or may not have \
                         happened; refresh in a minute."
                            .into(),
                    ),
                );
                return;
            }
            std::thread::sleep(t.verify_interval);
        }
    }

    // -- downloads ----------------------------------------------------------

    /// POST /api/esim/download: starts a download in the background; the activation code goes
    /// to the helper's stdin and nowhere else (not the log, the journal or any answer).
    pub fn download(self: &Arc<Self>, body: &[u8]) -> (u16, Value) {
        let err = |code: u16, msg: &str| (code, json!({"ok": false, "error": msg}));
        let body: Value = match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(_) => return err(400, "invalid JSON"),
        };
        let Some(id) = body["operation_id"]
            .as_str()
            .filter(|s| valid_operation_id(s))
        else {
            return err(400, "operation_id must be 8-64 letters, digits, '-' or '_'");
        };
        let Some(code) = body["activation_code"].as_str() else {
            return err(400, "missing activation_code");
        };
        let needs_confirmation = match check_activation_code(code) {
            Ok(needs) => needs,
            Err(e) => return err(400, e),
        };
        let confirmation = body["confirmation_code"]
            .as_str()
            .map(str::trim)
            .filter(|c| !c.is_empty());
        if let Some(c) = confirmation {
            if c.len() > MAX_CONFIRMATION_CODE || c.chars().any(char::is_control) {
                return err(
                    400,
                    "the confirmation code is too long or contains control characters",
                );
            }
        } else if needs_confirmation {
            return err(400, "this activation code needs a confirmation code");
        }
        let (rx, eid) = {
            let mut inner = self.inner.safe_lock();
            if let Some(op) = inner.operation.as_ref().filter(|o| o.id == id) {
                if op.kind == Kind::Download {
                    drop(inner);
                    return (200, json!({"ok": true, "data": self.view()}));
                }
                return err(409, "operation_id was already used for another change");
            }
            if inner
                .operation
                .as_ref()
                .is_some_and(|o| !o.state.resolved())
            {
                return err(409, "another eSIM change is still in progress");
            }
            if inner.status == Status::HelperMissing {
                return err(503, "eSIM management is unavailable: the helper is missing");
            }
            let Some(inv) = inner.inventory.as_ref() else {
                return err(409, "the profile list has not been read yet");
            };
            let eid = inv.eid.clone();
            let mut op = Operation::new(id, Kind::Download, "", None);
            op.state = OpState::Connecting;
            op.receipt_before = inv.notifications.clone();
            op.profiles_before = inv.profiles.iter().map(|p| p.handle.clone()).collect();
            let (tx, rx) = mpsc::channel();
            op.control = Some(tx);
            // Claimed under the same lock as the checks, so two requests cannot both start.
            inner.operation = Some(op);
            (rx, eid)
        };
        // The download runs over the router's own data, so it needs a data connection.
        if !self.env.wan_up() {
            self.inner.safe_lock().operation = None;
            return err(
                409,
                "the router has no mobile data connection; a download needs an active profile with data",
            );
        }
        let journaled = self
            .inner
            .safe_lock()
            .operation
            .clone()
            .map(|op| self.write_journal(&op));
        if let Some(Err(e)) = journaled {
            self.inner.safe_lock().operation = None;
            return err(503, &format!("cannot record the change: {e}"));
        }
        eprintln!("[esim] download requested (operation {id})");
        let mut request =
            json!({"op": "download", "activation_code": code.trim(), "expect_eid": eid});
        if let Some(c) = confirmation {
            request["confirmation_code"] = json!(c);
        }
        let me = Arc::clone(self);
        std::thread::spawn(move || me.run_download(request, rx));
        (202, json!({"ok": true, "data": self.view()}))
    }

    fn update_op(&self, f: impl FnOnce(&mut Operation)) {
        if let Some(op) = self.inner.safe_lock().operation.as_mut() {
            f(op);
        }
    }

    fn run_download(self: Arc<Self>, request: Value, controls: mpsc::Receiver<&'static str>) {
        let outcome = {
            let _card = self.card.safe_lock();
            let mut run = self.env.start_download(&request, DOWNLOAD_DEADLINE);
            drop(request);
            let mut timed_out = false;
            loop {
                while let Ok(control) = controls.try_recv() {
                    run.send(control);
                }
                let expired = {
                    let inner = self.inner.safe_lock();
                    inner.operation.as_ref().is_some_and(|o| {
                        o.state == OpState::AwaitingConfirmation
                            && o.preview_until.is_some_and(|t| Instant::now() >= t)
                    })
                };
                if expired && !timed_out {
                    timed_out = true;
                    run.send("timeout");
                }
                match run.next(Instant::now() + Duration::from_millis(250)) {
                    RunEvent::Wait => {}
                    RunEvent::Progress { step, data } => self.on_progress(&step, &data),
                    RunEvent::Done(outcome) => break outcome,
                }
            }
        };
        match outcome {
            HelperOutcome::Ok(data) => {
                self.update_op(|o| {
                    o.state = OpState::Verifying;
                    o.message = Some("Installed; reading the card back.".into());
                    if let Some(before) = data["notifications_before"].as_array() {
                        o.receipt_before = before.iter().filter_map(Value::as_u64).collect();
                    }
                });
                self.settle_download(true);
            }
            HelperOutcome::Failed {
                kind,
                message,
                detail,
            } => {
                if let Some(before) = detail["notifications_before"].as_array() {
                    let before: Vec<u64> = before.iter().filter_map(Value::as_u64).collect();
                    self.update_op(|o| o.receipt_before = before);
                }
                match kind.as_str() {
                    "download_cancelled" => self.finish(
                        OpState::Cancelled,
                        Some(match message.as_str() {
                            "declined at the preview" => {
                                "Declined at the preview. Nothing was installed.".to_string()
                            }
                            "not confirmed in time" => "The preview was not confirmed within 2 \
                                                        minutes. Nothing was installed."
                                .to_string(),
                            _ => "Cancelled. Nothing was installed.".to_string(),
                        }),
                    ),
                    "download_failed" => {
                        self.finish(OpState::Failed, Some(explain_download_failure(&message)));
                        // A card-reported install failure leaves a receipt for the operator.
                        self.queue_new_receipts_after_read();
                    }
                    "outcome_unknown" => self.settle_download(false),
                    "lock_busy" => self.finish(
                        OpState::Failed,
                        Some("Another eSIM operation held the card. Nothing was installed.".into()),
                    ),
                    _ => self.finish(
                        OpState::Failed,
                        Some(format!("The download did not start: {message}.")),
                    ),
                }
            }
            HelperOutcome::Killed | HelperOutcome::Garbled(_) => self.settle_download(false),
            HelperOutcome::Missing(m) => {
                self.inner.safe_lock().status = Status::HelperMissing;
                self.finish(OpState::Failed, Some(m));
            }
        }
    }

    fn on_progress(&self, step: &str, data: &Value) {
        let eid = self
            .inner
            .safe_lock()
            .inventory
            .as_ref()
            .map(|i| i.eid.clone())
            .unwrap_or_default();
        self.update_op(|o| {
            o.step = Some(step.to_string());
            match step {
                "es8p_metadata_parse" => {
                    let iccid = data["iccid"].as_str().unwrap_or_default();
                    // Operators sometimes put identifiers in names; never show those.
                    let clean =
                        |v: &Value| opt_text(v).filter(|t| !identifier_like(t, iccid, &eid));
                    o.preview = Some(json!({
                        "provider": clean(&data["provider"]),
                        "name": clean(&data["name"]),
                        "class": data["class"].as_str().unwrap_or("operational"),
                    }));
                }
                "preview" => {
                    o.state = OpState::AwaitingConfirmation;
                    o.preview_until = Some(Instant::now() + self.timing.preview_limit);
                    o.message = None;
                }
                "es10b_prepare_download" | "es9p_get_bound_profile_package" => {
                    o.state = OpState::Connecting;
                    o.preview_until = None;
                }
                "es10b_load_bound_profile_package" => {
                    o.state = OpState::Installing;
                    o.message =
                        Some("Installing on the card; this takes about 15-30 seconds.".into());
                }
                _ => {}
            }
        });
        if step == "es10b_load_bound_profile_package" {
            eprintln!("[esim] download: installing on the card");
        }
    }

    /// Reads the card back after a download (or a restart during one). A profile that wasn't
    /// there before is the installed one: name it, write the requested nickname and queue the
    /// install receipt. `reported`: the helper said the install succeeded.
    fn settle_download(self: &Arc<Self>, reported: bool) {
        let t = self.timing;
        let start = Instant::now();
        loop {
            if self.read_card().is_ok() {
                let found = {
                    let inner = self.inner.safe_lock();
                    let (Some(op), Some(inv)) =
                        (inner.operation.as_ref(), inner.inventory.as_ref())
                    else {
                        return;
                    };
                    inv.profiles
                        .iter()
                        .find(|p| !op.profiles_before.contains(&p.handle))
                        .map(|p| (p.handle.clone(), p.iccid.clone(), inv.eid.clone()))
                };
                match found {
                    Some((handle, iccid, eid)) => {
                        eprintln!("[esim] download installed {}", mask(&iccid));
                        let nickname = {
                            let mut inner = self.inner.safe_lock();
                            let op = inner.operation.as_mut();
                            op.and_then(|o| {
                                o.target = handle;
                                o.nickname.take()
                            })
                        };
                        let mut note = String::new();
                        if let Some(name) = nickname.filter(|n| !n.is_empty()) {
                            let outcome = {
                                let _card = self.card.safe_lock();
                                self.env.helper(
                                    &json!({"op": "nickname", "iccid": iccid, "expect_eid": eid, "nickname": name}),
                                    WRITE_DEADLINE,
                                )
                            };
                            if !matches!(outcome, HelperOutcome::Ok(_)) {
                                note = " The nickname could not be saved; use Rename.".into();
                            }
                            let _ = self.read_card();
                        }
                        self.finish(
                            OpState::Succeeded,
                            Some(format!("Installed. The new profile is inactive.{note}")),
                        );
                        self.queue_new_receipts_after_read();
                        return;
                    }
                    None if !reported => {
                        self.finish(
                            OpState::Failed,
                            Some("The download was interrupted; nothing was installed.".into()),
                        );
                        return;
                    }
                    None => {}
                }
            }
            if start.elapsed() >= t.verify_window {
                self.finish(
                    OpState::Unknown,
                    Some(
                        "The card could not be read back. The profile may or may not have been \
                         installed; refresh in a minute."
                            .into(),
                    ),
                );
                return;
            }
            std::thread::sleep(t.verify_interval);
        }
    }

    /// Queues the receipts that appeared since the operation started.
    fn queue_new_receipts_after_read(self: &Arc<Self>) {
        let new: Vec<u64> = {
            let inner = self.inner.safe_lock();
            match (inner.operation.as_ref(), inner.inventory.as_ref()) {
                (Some(o), Some(i)) => i
                    .notifications
                    .iter()
                    .copied()
                    .filter(|s| !o.receipt_before.contains(s))
                    .collect(),
                _ => Vec::new(),
            }
        };
        self.queue_change_receipts(new);
    }

    /// POST /api/esim/download/confirm: Install or decline at the preview.
    pub fn download_confirm(&self, body: &[u8]) -> (u16, Value) {
        let err = |code: u16, msg: &str| (code, json!({"ok": false, "error": msg}));
        let body: Value = match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(_) => return err(400, "invalid JSON"),
        };
        let Some(install) = body["install"].as_bool() else {
            return err(400, "missing install");
        };
        let nickname = body["nickname"]
            .as_str()
            .map(str::trim)
            .filter(|n| !n.is_empty());
        if let Some(n) = nickname {
            if let Err(e) = valid_nickname(n) {
                return err(400, e);
            }
        }
        {
            let mut inner = self.inner.safe_lock();
            let Some(op) = inner.operation.as_mut().filter(|o| {
                o.kind == Kind::Download
                    && body["operation_id"].as_str() == Some(o.id.as_str())
                    && o.state == OpState::AwaitingConfirmation
            }) else {
                return err(409, "no download is waiting for confirmation");
            };
            op.nickname = nickname.map(str::to_string);
            let sent = op
                .control
                .as_ref()
                .is_some_and(|c| c.send(if install { "install" } else { "decline" }).is_ok());
            if !sent {
                return err(409, "the download already ended");
            }
            op.state = OpState::Connecting;
            op.preview_until = None;
        }
        (200, json!({"ok": true, "data": self.view()}))
    }

    /// POST /api/esim/download/cancel: stops a download before the install starts.
    pub fn download_cancel(&self, body: &[u8]) -> (u16, Value) {
        let err = |code: u16, msg: &str| (code, json!({"ok": false, "error": msg}));
        let body: Value = match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(_) => return err(400, "invalid JSON"),
        };
        {
            let inner = self.inner.safe_lock();
            let Some(op) = inner.operation.as_ref().filter(|o| {
                o.kind == Kind::Download
                    && body["operation_id"].as_str() == Some(o.id.as_str())
                    && !o.state.resolved()
            }) else {
                return err(409, "no download is running");
            };
            if !matches!(
                op.state,
                OpState::Connecting | OpState::AwaitingConfirmation
            ) {
                return err(
                    409,
                    "the profile is being installed; it can't be cancelled now",
                );
            }
            if !op
                .control
                .as_ref()
                .is_some_and(|c| c.send("cancel").is_ok())
            {
                return err(409, "the download already ended");
            }
        }
        (200, json!({"ok": true, "data": self.view()}))
    }

    /// POST /api/esim/receipts/send: delivers every receipt on the card, on request only.
    pub fn receipts_send(self: &Arc<Self>) -> (u16, Value) {
        let seqs = {
            let inner = self.inner.safe_lock();
            if inner
                .operation
                .as_ref()
                .is_some_and(|o| !o.state.resolved())
            {
                return (
                    409,
                    json!({"ok": false, "error": "another eSIM change is still in progress"}),
                );
            }
            inner
                .inventory
                .as_ref()
                .map(|i| i.notifications.clone())
                .unwrap_or_default()
        };
        if seqs.is_empty() {
            return (200, json!({"ok": true, "data": self.view()}));
        }
        eprintln!(
            "[esim] sending the {} receipt(s) on the card, as requested",
            seqs.len()
        );
        self.queue_receipts(seqs);
        (202, json!({"ok": true, "data": self.view()}))
    }

    /// POST /api/esim/nickname: quick, so answered when done.
    pub fn nickname(self: &Arc<Self>, body: &[u8]) -> (u16, Value) {
        let body: Value = match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(_) => return (400, json!({"ok": false, "error": "invalid JSON"})),
        };
        let Some(name) = body["nickname"].as_str() else {
            return (400, json!({"ok": false, "error": "missing nickname"}));
        };
        if let Err(e) = valid_nickname(name) {
            return (400, json!({"ok": false, "error": e}));
        }
        let (_, target, _, eid) = match self.admit(Kind::Nickname, &body) {
            Ok(x) => x,
            Err(answer) => return answer,
        };
        let outcome = {
            let _card = self.card.safe_lock();
            self.env.helper(
                &json!({"op": "nickname", "iccid": target.iccid, "expect_eid": eid, "nickname": name}),
                WRITE_DEADLINE,
            )
        };
        let (state, message) = match outcome {
            HelperOutcome::Ok(_) => (OpState::Succeeded, None),
            HelperOutcome::Failed { kind, message, .. } if kind == "outcome_unknown" => (
                OpState::Unknown,
                Some(format!("{message}; refresh to see the name")),
            ),
            HelperOutcome::Failed { kind, message, .. }
                if kind == "refused" || kind == "state_changed" || kind == "card_changed" =>
            {
                (OpState::Refused, Some(message))
            }
            HelperOutcome::Failed { message, .. } => (OpState::Failed, Some(message)),
            HelperOutcome::Killed => (
                OpState::Unknown,
                Some("the rename was stopped; refresh to see the name".into()),
            ),
            HelperOutcome::Garbled(m) | HelperOutcome::Missing(m) => (OpState::Failed, Some(m)),
        };
        self.set_op(state, message);
        let _ = self.read_card();
        let code = if state == OpState::Succeeded {
            200
        } else {
            409
        };
        let view = self.view();
        if code == 200 {
            (200, json!({"ok": true, "data": view}))
        } else {
            let error = view["operation"]["message"]
                .as_str()
                .unwrap_or("the rename failed")
                .to_string();
            (code, json!({"ok": false, "error": error, "data": view}))
        }
    }

    /// POST /api/esim/lock: an agent-side guard against activating a profile.
    pub fn lock(&self, body: &[u8]) -> (u16, Value) {
        let body: Value = match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(_) => return (400, json!({"ok": false, "error": "invalid JSON"})),
        };
        let (Some(handle), Some(locked)) = (body["handle"].as_str(), body["locked"].as_bool())
        else {
            return (
                400,
                json!({"ok": false, "error": "expected handle and locked"}),
            );
        };
        {
            let mut inner = self.inner.safe_lock();
            let known = inner
                .inventory
                .as_ref()
                .is_some_and(|i| i.find(handle).is_some());
            if !known {
                return (404, json!({"ok": false, "error": "no such profile"}));
            }
            if locked {
                inner.locked.insert(handle.to_string());
            } else {
                inner.locked.remove(handle);
            }
        }
        self.save_state();
        (200, json!({"ok": true, "data": self.view()}))
    }

    // -- receipts -----------------------------------------------------------

    /// Receipts a change created. Not every profile asks for receipts; say so either way, for
    /// the record.
    fn queue_change_receipts(self: &Arc<Self>, seqs: Vec<u64>) {
        eprintln!(
            "[esim] the change created {} operator receipt(s){}",
            seqs.len(),
            if seqs.is_empty() {
                ""
            } else {
                "; queued for delivery"
            }
        );
        self.queue_receipts(seqs);
    }

    fn queue_receipts(self: &Arc<Self>, seqs: Vec<u64>) {
        if seqs.is_empty() {
            return;
        }
        {
            let mut inner = self.inner.safe_lock();
            for s in seqs {
                if !inner.receipts.pending.contains(&s) {
                    inner.receipts.pending.push(s);
                }
            }
            inner.receipts.round = 0;
            inner.receipts.gave_up = false;
        }
        self.save_state();
        self.start_receipt_worker();
    }

    fn start_receipt_worker(self: &Arc<Self>) {
        {
            let mut inner = self.inner.safe_lock();
            if inner.receipts.worker {
                return;
            }
            inner.receipts.worker = true;
        }
        let me = Arc::clone(self);
        std::thread::spawn(move || me.receipt_worker());
    }

    /// Sends this agent's receipts once data is back, then backs off. Never touches others.
    fn receipt_worker(self: Arc<Self>) {
        let t = self.timing;
        loop {
            let (seqs, eid, round) = {
                let mut inner = self.inner.safe_lock();
                let eid = inner.inventory.as_ref().map(|i| i.eid.clone());
                if inner.receipts.pending.is_empty() || inner.receipts.round >= RECEIPT_MAX_ROUNDS {
                    inner.receipts.gave_up = !inner.receipts.pending.is_empty()
                        && inner.receipts.round >= RECEIPT_MAX_ROUNDS;
                    inner.receipts.worker = false;
                    return;
                }
                (inner.receipts.pending.clone(), eid, inner.receipts.round)
            };
            // After a restart the card has not been read yet: wait for the first read.
            let Some(eid) = eid else {
                std::thread::sleep(t.wan_poll);
                continue;
            };
            // Wait for data; the first request right after an attach tends to fail.
            if !self.env.wan_up() {
                std::thread::sleep(t.wan_poll);
                continue;
            }
            if round == 0 {
                std::thread::sleep(t.receipt_settle);
            }
            let outcome = {
                let _card = self.card.safe_lock();
                self.env.helper(
                    &json!({"op": "notify", "expect_eid": eid, "seqs": seqs}),
                    NOTIFY_DEADLINE,
                )
            };
            {
                let mut inner = self.inner.safe_lock();
                inner.receipts.round += 1;
                match outcome {
                    HelperOutcome::Ok(data) => {
                        let mut errors = Vec::new();
                        for r in data["results"].as_array().into_iter().flatten() {
                            let seq = r["seq"].as_u64().unwrap_or_default();
                            if r["delivered"] == true || r["gone"] == true {
                                eprintln!(
                                    "[esim] receipt {seq} {}",
                                    if r["delivered"] == true {
                                        "delivered"
                                    } else {
                                        "no longer on the card"
                                    }
                                );
                                inner.receipts.pending.retain(|s| *s != seq);
                            } else if let Some(e) = r["error"].as_str() {
                                errors.push(format!(
                                    "{}: {e}",
                                    r["host"].as_str().unwrap_or("server")
                                ));
                            }
                        }
                        inner.receipts.last_error = errors.first().cloned();
                        // Re-read the card on the next view so its receipt count is current.
                        inner.read_attempt = None;
                    }
                    HelperOutcome::Failed { kind, message, .. } if kind == "card_changed" => {
                        inner.receipts.pending.clear();
                        inner.receipts.last_error = Some(message);
                    }
                    HelperOutcome::Failed { message, .. } => {
                        inner.receipts.last_error = Some(message)
                    }
                    HelperOutcome::Killed => {
                        inner.receipts.last_error = Some("the delivery run was stopped".into())
                    }
                    HelperOutcome::Garbled(m) | HelperOutcome::Missing(m) => {
                        inner.receipts.last_error = Some(m)
                    }
                }
                if let Some(e) = &inner.receipts.last_error {
                    eprintln!("[esim] receipt delivery: {e}");
                }
            }
            self.save_state();
            let delay = RECEIPT_DELAYS[(round as usize).min(RECEIPT_DELAYS.len() - 1)];
            let pending = !self.inner.safe_lock().receipts.pending.is_empty();
            if pending {
                std::thread::sleep(if cfg!(test) { t.wan_poll } else { delay });
            }
        }
    }
}

/// A failed download in plain words, keeping the server's own reason in brackets.
pub fn explain_download_failure(message: &str) -> String {
    let m = message.to_ascii_lowercase();
    let reason = message
        .split_once(": ")
        .map_or(message, |(_, r)| r)
        .trim_end_matches('.');
    if m.contains("eid-blacklist") || m.contains("eid blacklist") {
        format!(
            "Nothing was installed. The provider's server refuses this eSIM card ({reason}). Ask \
             the provider whether they support removable eSIM cards."
        )
    } else if m.contains("matchingid") {
        format!(
            "Nothing was installed. The provider refused this activation code ({reason}): it may \
             be used already, cancelled, tied to another device or not yet released. Ask the \
             provider to check or reissue it."
        )
    } else if m.contains("confirmation code") {
        format!(
            "Nothing was installed. The confirmation code was missing or refused ({reason}). \
             Check it with the provider."
        )
    } else if m.contains("http transport failed") || m.contains("http status code error") {
        format!(
            "Nothing was installed. The provider's server could not be reached ({reason}). Check \
             the router's mobile data and try again; the code is usually still usable."
        )
    } else if let Some(rest) = message.strip_prefix("the card refused the profile") {
        format!("Nothing was installed. The card refused the profile{rest}.")
    } else {
        format!("Nothing was installed: {message}.")
    }
}

fn valid_operation_id(s: &str) -> bool {
    (8..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A DNS host name with an optional port; no IP literals.
fn valid_server_address(address: &str) -> bool {
    let (host, port) = match address.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (address, None),
    };
    if port.is_some_and(|p| p.parse::<u16>().map_or(true, |p| p == 0)) {
        return false;
    }
    let labels: Vec<&str> = host.split('.').collect();
    !host.is_empty()
        && host.len() <= 253
        && labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        && !labels
            .last()
            .is_some_and(|l| l.bytes().all(|b| b.is_ascii_digit()))
}

/// Checks an SGP.22 activation code (`LPA:1$<server>$<matching ID>[$<OID>[$1]]`) without ever
/// repeating it. Returns whether a confirmation code is required.
pub fn check_activation_code(code: &str) -> Result<bool, &'static str> {
    let code = code.trim();
    if code.is_empty() || code.len() > MAX_ACTIVATION_CODE || code.chars().any(char::is_control) {
        return Err("the activation code is empty, too long or contains control characters");
    }
    let fields: Vec<&str> = code.split('$').collect();
    if !fields[0].eq_ignore_ascii_case("LPA:1") {
        return Err("the activation code must start with LPA:1$");
    }
    if fields.len() < 3 {
        return Err("the activation code has the wrong number of fields");
    }
    if !valid_server_address(fields[1].trim()) {
        return Err("the activation code has an invalid server address");
    }
    Ok(fields.get(4) == Some(&"1"))
}

/// Up to 64 bytes of UTF-8 (the card's limit); no control characters or invisible format
/// characters such as bidi overrides. Empty clears the nickname.
pub fn valid_nickname(name: &str) -> Result<(), &'static str> {
    if name.len() > 64 {
        return Err("nickname must be at most 64 bytes of UTF-8");
    }
    let format_char = |c: char| {
        matches!(c as u32,
            0x00AD | 0x0600..=0x0605 | 0x061C | 0x06DD | 0x070F | 0x08E2 | 0x180E
            | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F
            | 0xFEFF | 0xFFF9..=0xFFFB | 0xE0001 | 0xE0020..=0xE007F)
    };
    if name.chars().any(|c| c.is_control() || format_char(c)) {
        return Err("nickname contains control or invisible formatting characters");
    }
    Ok(())
}

#[cfg(test)]
#[path = "esim_tests.rs"]
mod tests;
