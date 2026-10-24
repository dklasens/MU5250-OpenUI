//! zte-esim: the agent's on-demand eSIM helper. One request per run, as one JSON line on stdin;
//! the answer is the last JSON line on stdout, and earlier lines report progress. It reads the
//! card's profiles, enables, disables or deletes one profile, sets a nickname, downloads a profile
//! from an activation code, or delivers named operator receipts. Nothing else: there is no APDU
//! passthrough, and each operation may only send the ES10 commands listed for it. A download
//! takes its decisions as further stdin lines (`{"control": "install" | "decline" | "cancel" |
//! "timeout"}`); stdin closing counts as cancel.
//!
//! Card sessions go over the modem's QMI UIM service (QRTR). A session owns one logical
//! channel; SIGTERM stops the work at the next step and the channel is still closed, because a
//! killed session leaves its channel allocated until the SIM resets. A non-blocking `flock`
//! keeps runs from overlapping even if the agent's own serialisation fails.

mod download;
mod https;
mod qrtr_uim;

use std::cell::Cell;
use std::io::{BufRead, Read, Write};
use std::os::fd::AsRawFd;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use euicc::{Apdu, Euicc};
use serde_json::{json, Value};

/// Version of the stdin/stdout protocol the agent checks before trusting the helper.
const PROTOCOL: u64 = 2;
const MAX_REQUEST: usize = 16 * 1024;
const QMI_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_DEADLINE: Duration = Duration::from_secs(20);
/// Long enough for a download: authentication, a 2-minute preview, the package and the install.
const MAX_DEADLINE: Duration = Duration::from_secs(420);
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_LOCK: &str = "/tmp/zte-esim.lock";
pub const NICKNAME_MAX_BYTES: usize = 64;

static CANCELLED: AtomicBool = AtomicBool::new(false);
static CHANNEL: AtomicU8 = AtomicU8::new(0);
static CHANNEL_LEAK: AtomicBool = AtomicBool::new(false);
/// A switch was accepted: the card resets, which closes every channel, so a failed close after
/// it is not a leak.
static CARD_RESET: AtomicBool = AtomicBool::new(false);
/// A write is on its way to the card: its answer is awaited even after a stop, so the outcome is
/// known rather than guessed.
static WRITE_IN_FLIGHT: AtomicBool = AtomicBool::new(false);
/// A profile package is being loaded: from the first segment on, nothing stops the install.
static INSTALLING: AtomicBool = AtomicBool::new(false);

pub fn cancelled() -> bool {
    CANCELLED.load(Ordering::Relaxed)
}

fn installing() -> bool {
    INSTALLING.load(Ordering::Relaxed)
}

/// Whether a wait may stop early for a cancellation.
pub fn interruptible() -> bool {
    cancelled() && !WRITE_IN_FLIGHT.load(Ordering::Relaxed) && !installing()
}

pub fn note_channel(channel: u8) {
    CHANNEL.store(channel, Ordering::Relaxed);
}

pub fn note_channel_leak(channel: u8, why: &str) {
    if !CARD_RESET.load(Ordering::Relaxed) {
        CHANNEL_LEAK.store(true, Ordering::Relaxed);
        eprintln!("[zte-esim] logical channel {channel} may still be open: {why}");
    }
}

extern "C" fn on_terminate(_: libc::c_int) {
    CANCELLED.store(true, Ordering::Relaxed);
}

/// SIGTERM and SIGINT only set the flag; no SA_RESTART, so blocking calls return early.
fn install_signal_handlers() {
    for sig in [libc::SIGTERM, libc::SIGINT] {
        // SAFETY: a zeroed sigaction with a plain handler that only stores an atomic.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_terminate as extern "C" fn(libc::c_int) as usize;
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(sig, &action, std::ptr::null_mut());
        }
    }
}

// ---------------------------------------------------------------------------
// Failures
// ---------------------------------------------------------------------------

/// Why a request ended without its result. `kind` is what the agent acts on.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub kind: &'static str,
    pub message: String,
    pub extra: Value,
}

impl Failure {
    fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Failure {
            kind,
            message: message.into(),
            extra: Value::Null,
        }
    }
}

/// Kinds: `invalid_request`, `lock_busy`, `cancelled`, `isdr_unavailable` (QMI 80
/// SimFileNotFound: no eUICC, or the SIM is reloading), `management_unavailable`, `transport`,
/// `card_changed`, `state_changed`, `refused` (the card answered a write with a result code),
/// `outcome_unknown` (a write was sent and its answer was lost), `untrusted_ci`.
fn classify_open(e: &str) -> Failure {
    if e.contains("cancelled") {
        Failure::new("cancelled", e)
    } else if e.contains("QMI error 80") {
        Failure::new("isdr_unavailable", e)
    } else if e.contains("QMI error") || e.contains("no UIM service") || e.contains("QRTR socket") {
        Failure::new("management_unavailable", e)
    } else {
        Failure::new("transport", e)
    }
}

// ---------------------------------------------------------------------------
// The command guard
// ---------------------------------------------------------------------------

/// ES10 request tags.
mod tag {
    pub const GET_EID: u16 = 0xBF3E;
    pub const INFO2: u16 = 0xBF22;
    pub const PROFILES: u16 = 0xBF2D;
    pub const LIST_NOTIFICATIONS: u16 = 0xBF28;
    pub const RETRIEVE_NOTIFICATION: u16 = 0xBF2B;
    pub const ENABLE: u16 = 0xBF31;
    pub const DISABLE: u16 = 0xBF32;
    pub const SET_NICKNAME: u16 = 0xBF29;
    pub const REMOVE_NOTIFICATION: u16 = 0xBF30;
    pub const DELETE: u16 = 0xBF33;
    pub const WRITES: [u16; 5] = [ENABLE, DISABLE, SET_NICKNAME, REMOVE_NOTIFICATION, DELETE];
    // Download (ES10b): challenge, EUICCInfo1, AuthenticateServer, PrepareDownload, CancelSession.
    pub const CHALLENGE: u16 = 0xBF2E;
    pub const INFO1: u16 = 0xBF20;
    pub const AUTHENTICATE_SERVER: u16 = 0xBF38;
    pub const PREPARE_DOWNLOAD: u16 = 0xBF21;
    pub const CANCEL_SESSION: u16 = 0xBF41;
    /// LoadBoundProfilePackage segments: the package header with InitialiseSecureChannel, then
    /// firstSequenceOf87, sequenceOf88 and its elements, secondSequenceOf87, sequenceOf86 and its
    /// elements.
    pub const INSTALL: [u16; 7] = [0xBF36, 0xA0, 0xA1, 0x88, 0xA2, 0xA3, 0x86];
}

/// Passes only GET RESPONSE and STORE DATA requests whose ES10 tag is allowed for the running
/// operation, and records whether a write left for the card. A command spread over several STORE
/// DATA blocks is judged by its first block; later blocks pass only as its continuation.
pub struct Guard<T: Apdu> {
    inner: T,
    allowed: &'static [u16],
    write_sent: Rc<Cell<bool>>,
    /// The tag of a multi-block command whose next block is due.
    continuing: Option<u16>,
}

/// The BER tag at the start of a request (one or two bytes).
fn leading_tag(data: &[u8]) -> Option<u16> {
    let first = *data.first()?;
    if first & 0x1F == 0x1F {
        Some(u16::from_be_bytes([first, *data.get(1)?]))
    } else {
        Some(u16::from(first))
    }
}

impl<T: Apdu> Guard<T> {
    fn allows(&mut self, apdu: &[u8]) -> Option<u16> {
        match apdu.get(1)? {
            0xC0 => Some(0),
            0xE2 => {
                let (p1, block) = (*apdu.get(2)?, *apdu.get(3)?);
                let more = match p1 {
                    0x11 => true,
                    0x91 => false,
                    _ => return None,
                };
                let t = if block == 0 {
                    leading_tag(apdu.get(5..)?).filter(|t| self.allowed.contains(t))?
                } else {
                    self.continuing?
                };
                self.continuing = more.then_some(t);
                Some(t)
            }
            _ => None,
        }
    }
}

impl<T: Apdu> Apdu for Guard<T> {
    fn open_channel(&mut self, aid: &[u8]) -> euicc::Result<u8> {
        self.inner.open_channel(aid)
    }
    fn close_channel(&mut self, channel: u8) {
        self.inner.close_channel(channel)
    }
    fn transmit(&mut self, apdu: &[u8]) -> euicc::Result<Vec<u8>> {
        let Some(t) = self.allows(apdu) else {
            return Err(euicc::Error::Transport(
                "command not allowed for this operation".into(),
            ));
        };
        if cancelled() && !self.write_sent.get() && !installing() {
            return Err(euicc::Error::Transport("cancelled".into()));
        }
        if tag::INSTALL.contains(&t) {
            // The install has started: it runs to the end whatever happens.
            self.write_sent.set(true);
            INSTALLING.store(true, Ordering::Relaxed);
        }
        if tag::WRITES.contains(&t) {
            self.write_sent.set(true);
            WRITE_IN_FLIGHT.store(true, Ordering::Relaxed);
            let answer = self.inner.transmit(apdu);
            WRITE_IN_FLIGHT.store(false, Ordering::Relaxed);
            return answer;
        }
        self.inner.transmit(apdu)
    }
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/// An open card session and whether a write has left for the card in it.
type Session<T> = (Euicc<Guard<T>>, Rc<Cell<bool>>);

pub struct Card<F> {
    connect: F,
}

impl<T: Apdu, F: FnMut() -> Result<T, String>> Card<F> {
    fn session(&mut self, allowed: &'static [u16]) -> Result<Session<T>, Failure> {
        if cancelled() {
            return Err(Failure::new("cancelled", "cancelled"));
        }
        let io = (self.connect)().map_err(|e| classify_open(&e))?;
        let write_sent = Rc::new(Cell::new(false));
        let guard = Guard {
            inner: io,
            allowed,
            write_sent: write_sent.clone(),
            continuing: None,
        };
        let e = Euicc::open(guard).map_err(|e| classify_open(&e.to_string()))?;
        Ok((e, write_sent))
    }
}

fn read_err(what: &str, e: euicc::Error) -> Failure {
    let m = format!("{what}: {e}");
    if m.contains("cancelled") {
        Failure::new("cancelled", m)
    } else {
        Failure::new("transport", m)
    }
}

fn check_eid<T: Apdu>(e: &mut Euicc<T>, expect: &str) -> Result<(), Failure> {
    let eid = e.eid().map_err(|x| read_err("reading the EID", x))?;
    if !expect.eq_ignore_ascii_case(&eid) {
        return Err(Failure::new(
            "card_changed",
            "a different eSIM card is in the slot",
        ));
    }
    Ok(())
}

fn profile_json(p: &euicc::es10c::ProfileInfo) -> Value {
    json!({
        "iccid": p.iccid,
        "state": p.profile_state,
        // SGP.22 ProfileClass DEFAULT operational: an absent tag means operational.
        "class": p.profile_class.unwrap_or("operational"),
        "nickname": p.profile_nickname,
        "provider": p.service_provider_name,
        "name": p.profile_name,
    })
}

fn notification_json(n: &euicc::es10b::NotificationMetadata) -> Value {
    json!({
        "seq": n.seq_number,
        "operation": n.profile_management_operation,
        "address": n.notification_address.as_deref().map(str::trim),
        "iccid": n.iccid,
    })
}

fn inventory<T: Apdu>(
    card: &mut Card<impl FnMut() -> Result<T, String>>,
) -> Result<Value, Failure> {
    const TAGS: &[u16] = &[
        tag::GET_EID,
        tag::INFO2,
        tag::PROFILES,
        tag::LIST_NOTIFICATIONS,
    ];
    let (mut e, _) = card.session(TAGS)?;
    let eid = e.eid().map_err(|x| read_err("reading the EID", x))?;
    // Card details are optional: the EID and the profiles are what matter.
    let info = match e.euicc_info2() {
        Ok(i) => {
            let r = i.ext_card_resource.unwrap_or_default();
            json!({
                "firmware": i.euicc_firmware_ver,
                "sgp22_version": i.svn,
                "profile_version": i.profile_version,
                "free_nvm_bytes": r.free_non_volatile_memory,
                "free_ram_bytes": r.free_volatile_memory,
                "ci_pkids": i.euicc_ci_pkid_list_for_verification,
            })
        }
        Err(x) if x.to_string().contains("cancelled") => {
            return Err(Failure::new("cancelled", "cancelled"))
        }
        Err(_) => Value::Null,
    };
    let profiles = e
        .profiles()
        .map_err(|x| read_err("reading the profiles", x))?;
    let notifications = e
        .list_notifications()
        .map_err(|x| read_err("reading the notifications", x))?;
    Ok(json!({
        "eid": eid,
        "info": info,
        "profiles": profiles.iter().map(profile_json).collect::<Vec<_>>(),
        "notifications": notifications.iter().map(notification_json).collect::<Vec<_>>(),
    }))
}

#[derive(Clone, Copy, PartialEq)]
enum Switch {
    Enable,
    Disable,
}

/// EnableProfile or DisableProfile, with SIM refresh. The card identity, the target's state and
/// class, and which profile is enabled are checked in the same session, right before the write.
fn switch<T: Apdu>(
    card: &mut Card<impl FnMut() -> Result<T, String>>,
    which: Switch,
    iccid: &str,
    expect_eid: &str,
    expect_enabled: Option<&str>,
) -> Result<Value, Failure> {
    const ENABLE_TAGS: &[u16] = &[
        tag::GET_EID,
        tag::PROFILES,
        tag::LIST_NOTIFICATIONS,
        tag::ENABLE,
    ];
    const DISABLE_TAGS: &[u16] = &[
        tag::GET_EID,
        tag::PROFILES,
        tag::LIST_NOTIFICATIONS,
        tag::DISABLE,
    ];
    let (mut e, write_sent) = card.session(if which == Switch::Enable {
        ENABLE_TAGS
    } else {
        DISABLE_TAGS
    })?;
    check_eid(&mut e, expect_eid)?;
    let profiles = e
        .profiles()
        .map_err(|x| read_err("reading the profiles", x))?;
    let target = profiles
        .iter()
        .find(|p| p.iccid == iccid)
        .ok_or_else(|| Failure::new("state_changed", "the profile is no longer on the card"))?;
    let enabled: Vec<&str> = profiles
        .iter()
        .filter(|p| p.profile_state == Some("enabled"))
        .map(|p| p.iccid.as_str())
        .collect();
    if enabled.first().copied() != expect_enabled || enabled.len() > 1 {
        return Err(Failure::new(
            "state_changed",
            "the enabled profile changed since the list was read",
        ));
    }
    match which {
        Switch::Enable => {
            if target.profile_class.unwrap_or("operational") != "operational" {
                return Err(Failure::new(
                    "state_changed",
                    "only operational profiles can be activated",
                ));
            }
            if target.profile_state != Some("disabled") {
                return Err(Failure::new("state_changed", "the profile is not inactive"));
            }
        }
        Switch::Disable => {
            if target.profile_state != Some("enabled") {
                return Err(Failure::new("state_changed", "the profile is not active"));
            }
        }
    }
    let before: Vec<u64> = e
        .list_notifications()
        .map_err(|x| read_err("reading the notifications", x))?
        .iter()
        .map(|n| n.seq_number)
        .collect();
    if cancelled() {
        return Err(Failure::new("cancelled", "cancelled before the change"));
    }
    let (op, result) = match which {
        Switch::Enable => ("enable_profile", e.enable_profile(iccid, true)),
        Switch::Disable => ("disable_profile", e.disable_profile(iccid, true)),
    };
    write_result(
        op,
        result,
        write_sent.get(),
        json!({"notifications_before": before}),
    )
}

/// Maps a write's answer: 0 is applied; another code is the card's refusal (nothing changed);
/// a failure after the command left is an unknown outcome, never retried here.
fn write_result(
    op: &str,
    result: euicc::Result<u64>,
    write_sent: bool,
    data: Value,
) -> Result<Value, Failure> {
    match result {
        Ok(0) => {
            // A switch makes the card reset (SIM refresh), which closes every channel.
            if op == "enable_profile" || op == "disable_profile" {
                CARD_RESET.store(true, Ordering::Relaxed);
            }
            let mut data = data;
            data["result"] = json!("applied");
            Ok(data)
        }
        Ok(code) => Err(Failure {
            kind: "refused",
            message: euicc::es10c::result_reason(op, code),
            extra: json!({"code": code}),
        }),
        Err(x) if write_sent => Err(Failure::new(
            "outcome_unknown",
            format!("{op}: the card's answer was lost ({x})"),
        )),
        Err(x) => Err(read_err(op, x)),
    }
}

/// Up to 64 bytes of UTF-8; no control characters (Cc) or invisible format characters (Cf, such
/// as bidi overrides). Empty clears the nickname.
pub fn valid_nickname(name: &str) -> Result<(), &'static str> {
    if name.len() > NICKNAME_MAX_BYTES {
        return Err("nickname is longer than 64 bytes");
    }
    if name.chars().any(|c| c.is_control() || is_format_char(c)) {
        return Err("nickname contains control or invisible formatting characters");
    }
    Ok(())
}

/// Unicode general category Cf (the ranges that occur in practice).
fn is_format_char(c: char) -> bool {
    matches!(c as u32,
        0x00AD | 0x0600..=0x0605 | 0x061C | 0x06DD | 0x070F | 0x08E2 | 0x180E
        | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F
        | 0xFEFF | 0xFFF9..=0xFFFB | 0x110BD | 0x110CD | 0x13430..=0x1343F
        | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A | 0xE0001 | 0xE0020..=0xE007F)
}

fn nickname<T: Apdu>(
    card: &mut Card<impl FnMut() -> Result<T, String>>,
    iccid: &str,
    expect_eid: &str,
    name: &str,
) -> Result<Value, Failure> {
    const TAGS: &[u16] = &[tag::GET_EID, tag::PROFILES, tag::SET_NICKNAME];
    valid_nickname(name).map_err(|m| Failure::new("invalid_request", m))?;
    let (mut e, write_sent) = card.session(TAGS)?;
    check_eid(&mut e, expect_eid)?;
    let profiles = e
        .profiles()
        .map_err(|x| read_err("reading the profiles", x))?;
    if !profiles.iter().any(|p| p.iccid == iccid) {
        return Err(Failure::new(
            "state_changed",
            "the profile is no longer on the card",
        ));
    }
    if cancelled() {
        return Err(Failure::new("cancelled", "cancelled before the change"));
    }
    let result = e.set_nickname(iccid, name);
    write_result("set_nickname", result, write_sent.get(), json!({}))
}

/// Delivers the named receipts. The card session is closed while HTTPS runs, so a stop at any
/// point cannot strand a channel; a receipt is removed only after its server accepted it.
fn notify<T: Apdu>(
    card: &mut Card<impl FnMut() -> Result<T, String>>,
    expect_eid: &str,
    seqs: &[u64],
    http: &mut impl euicc::es9p::Http,
) -> Result<Value, Failure> {
    const READ_TAGS: &[u16] = &[
        tag::GET_EID,
        tag::INFO2,
        tag::LIST_NOTIFICATIONS,
        tag::RETRIEVE_NOTIFICATION,
    ];
    const REMOVE_TAGS: &[u16] = &[tag::GET_EID, tag::REMOVE_NOTIFICATION];
    let mut pending = Vec::new();
    let mut results = Vec::new();
    {
        let (mut e, _) = card.session(READ_TAGS)?;
        check_eid(&mut e, expect_eid)?;
        let info = e
            .euicc_info2()
            .map_err(|x| read_err("reading the card's CI list", x))?;
        if !info
            .euicc_ci_pkid_list_for_verification
            .iter()
            .any(|k| k.eq_ignore_ascii_case(https::GSMA_CI1_KEY_ID))
        {
            return Err(Failure::new(
                "untrusted_ci",
                "the card does not trust GSMA RSP2 Root CI1",
            ));
        }
        let listed: Vec<u64> = e
            .list_notifications()
            .map_err(|x| read_err("reading the notifications", x))?
            .iter()
            .map(|n| n.seq_number)
            .collect();
        for &seq in seqs {
            if !listed.contains(&seq) {
                results.push(json!({"seq": seq, "delivered": false, "gone": true}));
                continue;
            }
            let p = e
                .retrieve_notification(seq)
                .map_err(|x| read_err("retrieving a receipt", x))?;
            pending.push((seq, p));
        }
    }
    let mut delivered = Vec::new();
    for (seq, p) in pending {
        let address = p.notification_address.trim().to_string();
        let outcome = if cancelled() {
            Err("cancelled".to_string())
        } else if !https::valid_address(&address) {
            Err("invalid server address".to_string())
        } else {
            euicc::es9p::handle_notification(http, &address, &euicc::b64::encode(&p.der)).map_err(
                |e| match e.transport_error {
                    Some(cause) => format!("{} ({cause})", e.message),
                    None => format!("{} ({})", e.message, e.subject_identifier),
                },
            )
        };
        match outcome {
            Ok(()) => delivered.push(seq),
            Err(error) => results.push(json!({
                "seq": seq, "delivered": false, "host": address, "error": error,
            })),
        }
    }
    if !delivered.is_empty() {
        // If a stop prevents the removal, the receipt stays on the card and a later round
        // delivers it again; nothing is lost.
        let removal = (|| {
            let (mut e, _) = card.session(REMOVE_TAGS)?;
            check_eid(&mut e, expect_eid)?;
            let mut removed = Vec::new();
            for &seq in &delivered {
                let ok = matches!(e.remove_notification(seq), Ok(0 | 1));
                removed.push((seq, ok));
            }
            Ok::<_, Failure>(removed)
        })();
        match removal {
            Ok(removed) => {
                for (seq, ok) in removed {
                    results.push(json!({"seq": seq, "delivered": true, "removed": ok}));
                }
            }
            Err(f) => {
                for seq in delivered {
                    results.push(json!({
                        "seq": seq, "delivered": true, "removed": false, "error": f.message,
                    }));
                }
            }
        }
    }
    Ok(json!({"results": results}))
}

/// DeleteProfile. The profile must exist and be inactive, checked in the same session as the
/// write; the receipts already on the card are returned so only the new one is sent.
fn delete<T: Apdu>(
    card: &mut Card<impl FnMut() -> Result<T, String>>,
    iccid: &str,
    expect_eid: &str,
) -> Result<Value, Failure> {
    const TAGS: &[u16] = &[
        tag::GET_EID,
        tag::PROFILES,
        tag::LIST_NOTIFICATIONS,
        tag::DELETE,
    ];
    let (mut e, write_sent) = card.session(TAGS)?;
    check_eid(&mut e, expect_eid)?;
    let profiles = e
        .profiles()
        .map_err(|x| read_err("reading the profiles", x))?;
    let target = profiles
        .iter()
        .find(|p| p.iccid == iccid)
        .ok_or_else(|| Failure::new("state_changed", "the profile is no longer on the card"))?;
    if target.profile_state != Some("disabled") {
        return Err(Failure::new(
            "state_changed",
            "only an inactive profile can be deleted",
        ));
    }
    let before: Vec<u64> = e
        .list_notifications()
        .map_err(|x| read_err("reading the notifications", x))?
        .iter()
        .map(|n| n.seq_number)
        .collect();
    if cancelled() {
        return Err(Failure::new("cancelled", "cancelled before the change"));
    }
    let result = e.delete_profile(iccid);
    write_result(
        "delete_profile",
        result,
        write_sent.get(),
        json!({"notifications_before": before}),
    )
}

/// One progress line for the agent; the last stdout line remains the answer.
fn report_progress(step: &str, data: Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", json!({"progress": step, "data": data}));
    let _ = out.flush();
}

/// Downloads and installs one profile. Decisions arrive on `ctl`; the result carries the new
/// profile's ICCID, the install receipt's seq and the receipts that were on the card before.
#[allow(clippy::too_many_arguments)]
fn download_profile<T: Apdu, H: download::Es9pTransport>(
    card: &mut Card<impl FnMut() -> Result<T, String>>,
    expect_eid: &str,
    code: &download::ActivationCode,
    confirmation_code: Option<&str>,
    ctl: &Receiver<download::DownloadCtl>,
    http: &mut H,
    deadline: Instant,
    progress: &mut dyn FnMut(&str, Value),
) -> Result<Value, Failure> {
    const TAGS: &[u16] = &[
        tag::GET_EID,
        tag::LIST_NOTIFICATIONS,
        tag::CHALLENGE,
        tag::INFO1,
        tag::AUTHENTICATE_SERVER,
        tag::PREPARE_DOWNLOAD,
        tag::CANCEL_SESSION,
        0xBF36,
        0xA0,
        0xA1,
        0x88,
        0xA2,
        0xA3,
        0x86,
    ];
    let (mut e, _) = card.session(TAGS)?;
    check_eid(&mut e, expect_eid)?;
    let before: Vec<u64> = e
        .list_notifications()
        .map_err(|x| read_err("reading the notifications", x))?
        .iter()
        .map(|n| n.seq_number)
        .collect();
    let detail = json!({"notifications_before": before});
    match download::download(
        &mut e,
        http,
        code,
        confirmation_code,
        ctl,
        progress,
        deadline,
    ) {
        Ok(mut data) => {
            data["notifications_before"] = json!(before);
            Ok(data)
        }
        Err((ending, message)) => Err(Failure {
            kind: match ending {
                download::Ending::Cancelled => "download_cancelled",
                download::Ending::Failed => "download_failed",
                download::Ending::Unknown => "outcome_unknown",
            },
            message,
            extra: detail,
        }),
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

struct Request {
    op: String,
    node: u32,
    slot: u8,
    lock_path: String,
    deadline: Duration,
    body: Value,
}

fn parse_request(raw: &[u8]) -> Result<Request, Failure> {
    let bad = |m: &str| Failure::new("invalid_request", m);
    let body: Value = serde_json::from_slice(raw).map_err(|_| bad("request is not JSON"))?;
    if body["protocol"].as_u64() != Some(PROTOCOL) {
        return Err(bad("unsupported protocol version"));
    }
    let op = body["op"]
        .as_str()
        .ok_or_else(|| bad("missing op"))?
        .to_string();
    let node = body["node"].as_u64().unwrap_or(3);
    let slot = body["slot"].as_u64().unwrap_or(1);
    if node > u32::MAX as u64 || !(1..=2).contains(&slot) {
        return Err(bad("invalid modem node or slot"));
    }
    let deadline = body["deadline_ms"]
        .as_u64()
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_DEADLINE)
        .min(MAX_DEADLINE);
    let lock_path = body["lock_path"]
        .as_str()
        .unwrap_or(DEFAULT_LOCK)
        .to_string();
    if !lock_path.starts_with("/tmp/") || lock_path.contains("..") {
        return Err(bad("lock path must be under /tmp"));
    }
    Ok(Request {
        op,
        node: node as u32,
        slot: slot as u8,
        lock_path,
        deadline,
        body,
    })
}

fn iccid_arg(body: &Value, key: &str) -> Result<String, Failure> {
    body[key]
        .as_str()
        .filter(|s| (18..=20).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()))
        .map(str::to_string)
        .ok_or_else(|| Failure::new("invalid_request", format!("invalid {key}")))
}

fn eid_arg(body: &Value) -> Result<String, Failure> {
    body["expect_eid"]
        .as_str()
        .filter(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_string)
        .ok_or_else(|| Failure::new("invalid_request", "invalid expect_eid"))
}

fn dispatch<T: Apdu, H: download::Es9pTransport>(
    req: &Request,
    card: &mut Card<impl FnMut() -> Result<T, String>>,
    http: &mut H,
    ctl: &Receiver<download::DownloadCtl>,
    progress: &mut dyn FnMut(&str, Value),
) -> Result<Value, Failure> {
    let b = &req.body;
    match req.op.as_str() {
        "inventory" => inventory(card),
        "delete" => delete(card, &iccid_arg(b, "iccid")?, &eid_arg(b)?),
        "download" => {
            let bad = |m: &str| Failure::new("invalid_request", m);
            let code = b["activation_code"]
                .as_str()
                .ok_or_else(|| bad("missing activation_code"))?;
            let code = download::parse_activation_code(code).map_err(bad)?;
            let confirmation = match &b["confirmation_code"] {
                Value::Null => None,
                v => Some(
                    v.as_str()
                        .filter(|c| download::valid_confirmation_code(c))
                        .ok_or_else(|| bad("invalid confirmation_code"))?,
                ),
            };
            let deadline = Instant::now() + req.deadline;
            download_profile(
                card,
                &eid_arg(b)?,
                &code,
                confirmation,
                ctl,
                http,
                deadline,
                progress,
            )
        }
        "enable" | "disable" => {
            let which = if req.op == "enable" {
                Switch::Enable
            } else {
                Switch::Disable
            };
            let iccid = iccid_arg(b, "iccid")?;
            let expect_enabled = match &b["expect_enabled"] {
                Value::Null => None,
                _ => Some(iccid_arg(b, "expect_enabled")?),
            };
            switch(card, which, &iccid, &eid_arg(b)?, expect_enabled.as_deref())
        }
        "nickname" => {
            let name = b["nickname"]
                .as_str()
                .ok_or_else(|| Failure::new("invalid_request", "missing nickname"))?;
            nickname(card, &iccid_arg(b, "iccid")?, &eid_arg(b)?, name)
        }
        "notify" => {
            let seqs: Vec<u64> = b["seqs"]
                .as_array()
                .ok_or_else(|| Failure::new("invalid_request", "missing seqs"))?
                .iter()
                .map(|v| v.as_u64())
                .collect::<Option<_>>()
                .filter(|s: &Vec<u64>| !s.is_empty() && s.len() <= 32)
                .ok_or_else(|| Failure::new("invalid_request", "invalid seqs"))?;
            notify(card, &eid_arg(b)?, &seqs, http)
        }
        _ => Err(Failure::new("invalid_request", "unknown op")),
    }
}

fn answer(result: Result<Value, Failure>) -> Value {
    let channel = json!({
        "last": CHANNEL.load(Ordering::Relaxed),
        "leak_suspected": CHANNEL_LEAK.load(Ordering::Relaxed),
    });
    match result {
        Ok(data) => json!({"ok": true, "protocol": PROTOCOL, "data": data, "channel": channel}),
        Err(f) => json!({
            "ok": false,
            "protocol": PROTOCOL,
            "error": {"kind": f.kind, "message": f.message, "detail": f.extra},
            "cancelled": cancelled(),
            "channel": channel,
        }),
    }
}

/// Forwards a download's control lines; stdin closing (the agent went away) means cancel.
fn control_lines(mut reader: impl BufRead + Send + 'static) -> Receiver<download::DownloadCtl> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        loop {
            line.clear();
            match (&mut reader).take(4096).read_line(&mut line) {
                Ok(0) | Err(_) => {
                    let _ = tx.send(download::DownloadCtl::Cancel);
                    return;
                }
                Ok(_) => {
                    if let Some(ctl) = download::DownloadCtl::parse(line.trim()) {
                        if tx.send(ctl).is_err() {
                            return;
                        }
                    }
                }
            }
        }
    });
    rx
}

fn run() -> Result<Value, Failure> {
    // The request is the first line. A download takes its decisions as later lines, so stdin is
    // not read to its end here.
    let mut reader = std::io::BufReader::new(std::io::stdin());
    let mut raw = Vec::new();
    (&mut reader)
        .take(MAX_REQUEST as u64 + 1)
        .read_until(b'\n', &mut raw)
        .map_err(|e| Failure::new("invalid_request", e.to_string()))?;
    if raw.len() > MAX_REQUEST {
        return Err(Failure::new("invalid_request", "request too large"));
    }
    let req = parse_request(&raw)?;
    if req.op == "version" {
        return Ok(json!({"protocol": PROTOCOL, "version": env!("CARGO_PKG_VERSION")}));
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&req.lock_path)
        .map_err(|e| Failure::new("transport", format!("lock file: {e}")))?;
    // SAFETY: flock on a descriptor owned for the rest of this run; the kernel releases it if the
    // process dies.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(Failure::new(
            "lock_busy",
            "another eSIM operation holds the card",
        ));
    }
    let deadline = Instant::now() + req.deadline;
    let (node, slot) = (req.node, req.slot);
    let mut card = Card {
        connect: || qrtr_uim::UimClient::connect(node, slot, QMI_TIMEOUT, deadline),
    };
    let mut http = https::Es9pHttp {
        timeout: HTTP_TIMEOUT,
        deadline,
        max_response: if req.op == "download" {
            https::PACKAGE_RESPONSE
        } else {
            https::SMALL_RESPONSE
        },
    };
    let ctl = if req.op == "download" {
        control_lines(reader)
    } else {
        mpsc::channel().1
    };
    let result = dispatch(&req, &mut card, &mut http, &ctl, &mut report_progress);
    drop(lock);
    result
}

fn main() {
    install_signal_handlers();
    let result = run();
    let ok = result.is_ok();
    println!("{}", answer(result));
    std::process::exit(if ok { 0 } else { 1 });
}

/// Tests that touch the process-wide flags (cancel, install) run one at a time.
#[cfg(test)]
pub(crate) fn test_serial() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests;
