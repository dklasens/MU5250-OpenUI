// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2023-2025 ESTKME TECHNOLOGY LIMITED, Hong Kong
// SPDX-FileCopyrightText: 2026 Jesther Silvestre
//
// Rust translation of lpac euicc/es9p.c, euicc/es9p.h, euicc/es9p_errors.c and
// euicc/es9p_errors.h (lpac commit c7c8d4f).
// Changed 2026-10-19: rewritten in Rust, ES9+ only (the ES11 functions are not translated). The
// HTTP interface is a trait; each function returns its result or the status instead of filling
// ctx->http.status and ctx->http._internal. Instead of linking cJSON, the request is printed and
// the response read by a minimal reimplementation of the behaviour libeuicc relied on from
// cJSON 1.7.18 as pinned by lpac (CoelacanthusHex/cJSON 4818f04, numbers of any length):
// lenient parsing, first case-insensitive member match, C-string truncation at NUL. Its
// containers are walked with an explicit stack. A transport failure also keeps the transport's
// own error text. Text that is not UTF-8 is converted lossily.
//
// The JSON handling (hex4, utf16_literal and the escape loop of Parser::string in particular)
// follows cJSON 1.7.18 as pinned by lpac (CoelacanthusHex/cJSON 4818f04, numbers of any
// length), which is distributed under this notice:
//
//   Copyright (c) 2009-2017 Dave Gamble and cJSON contributors
//
//   Permission is hereby granted, free of charge, to any person obtaining a copy
//   of this software and associated documentation files (the "Software"), to deal
//   in the Software without restriction, including without limitation the rights
//   to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
//   copies of the Software, and to permit persons to whom the Software is
//   furnished to do so, subject to the following conditions:
//
//   The above copyright notice and this permission notice shall be included in
//   all copies or substantial portions of the Software.
//
//   THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
//   IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//   FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//   AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//   LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
//   OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
//   THE SOFTWARE.

//! ES9+: the LPA's HTTPS exchanges with the SM-DP+ during a profile download, and the delivery
//! of notifications (SGP.22 section 6.5).
//!
//! Every request is a JSON object POSTed to `https://<server address><path>` with the three
//! [`HEADERS`]. A response other than 2xx fails. Functions that return data read named string
//! members of the response's top-level object; a response without them fails with the
//! `header.functionExecutionStatus.statusCodeData` the server sent, if any. CancelSession and
//! HandleNotification only need a 2xx answer.

use crate::rsp::{AuthenticateServerParam, PrepareDownloadParam};
use std::fmt;

/// How ES9+ requests reach the server (`struct euicc_http_interface`).
pub trait Http {
    /// POSTs `body` to `url` with `headers` (each `Name: value`) and returns the HTTP status and
    /// the response body. `Err` is a transport failure (DNS, TLS, timeout...) and its text.
    fn post(&mut self, url: &str, headers: &[&str], body: &[u8]) -> Result<(u32, Vec<u8>), String>;
}

/// The headers sent with every ES9+ request (`lpa_header`).
pub const HEADERS: [&str; 3] = [
    "User-Agent: gsma-rsp-lpad",
    "X-Admin-Protocol: gsma/rsp/v2.2.2",
    "Content-Type: application/json",
];

/// Why an ES9+ call failed (`ctx->http.status`).
///
/// The fields keep libeuicc's sizes: the codes at most 8 bytes, the identifier and the message at
/// most 128 bytes (longer server text is cut, as `snprintf` into the C buffers did).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Es9pStatus {
    /// `subjectCode` from the server's statusCodeData, else `0.0.0`.
    pub subject_code: String,
    /// `reasonCode` from the server's statusCodeData, else `0.0.0`.
    pub reason_code: String,
    /// `subjectIdentifier` from the server, else what failed locally: `unknown`, the HTTP status
    /// number, `root`, `header` or `functionExecutionStatus`.
    pub subject_identifier: String,
    /// The server's `message`, the description of its subject and reason codes, or a local
    /// reason such as `HTTP transport failed`, `HTTP status code error`, `Not JSON`,
    /// `Not Object`, `Critical object missing` or `unknown`.
    pub message: String,
    /// For `HTTP transport failed` only: the transport's error text. Not part of libeuicc's
    /// status (its curl driver printed this to stderr).
    pub transport_error: Option<String>,
}

impl fmt::Display for Es9pStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Es9pStatus {}

const PATH_INITIATE_AUTHENTICATION: &str = "/gsma/rsp2/es9plus/initiateAuthentication";
const PATH_AUTHENTICATE_CLIENT: &str = "/gsma/rsp2/es9plus/authenticateClient";
const PATH_GET_BOUND_PROFILE_PACKAGE: &str = "/gsma/rsp2/es9plus/getBoundProfilePackage";
const PATH_CANCEL_SESSION: &str = "/gsma/rsp2/es9plus/cancelSession";
const PATH_HANDLE_NOTIFICATION: &str = "/gsma/rsp2/es9plus/handleNotification";

/// ES9+ InitiateAuthentication. Returns the transactionId and the values for ES10b
/// AuthenticateServer (`es9p_initiate_authentication_r`).
pub fn initiate_authentication(
    http: &mut (impl Http + ?Sized),
    server_address: &str,
    b64_euicc_challenge: &str,
    b64_euicc_info1: &str,
) -> Result<(String, AuthenticateServerParam), Es9pStatus> {
    let [transaction_id, signed1, signature1, ci_pkid, certificate] = exchange_json(
        http,
        server_address,
        PATH_INITIATE_AUTHENTICATION,
        &[
            ("smdpAddress", Some(server_address)),
            ("euiccChallenge", Some(b64_euicc_challenge)),
            ("euiccInfo1", Some(b64_euicc_info1)),
        ],
        [
            "transactionId",
            "serverSigned1",
            "serverSignature1",
            "euiccCiPKIdToBeUsed",
            "serverCertificate",
        ],
    )?;
    Ok((
        text(transaction_id),
        AuthenticateServerParam {
            b64_server_signed1: base64_text(signed1),
            b64_server_signature1: base64_text(signature1),
            b64_euicc_ci_pkid_to_be_used: base64_text(ci_pkid),
            b64_server_certificate: base64_text(certificate),
        },
    ))
}

/// ES9+ AuthenticateClient. Returns the values for the profile preview and ES10b
/// PrepareDownload (`es9p_authenticate_client_r`).
pub fn authenticate_client(
    http: &mut (impl Http + ?Sized),
    server_address: &str,
    transaction_id: &str,
    b64_authenticate_server_response: &str,
) -> Result<PrepareDownloadParam, Es9pStatus> {
    let [metadata, signed2, signature2, certificate] = exchange_json(
        http,
        server_address,
        PATH_AUTHENTICATE_CLIENT,
        &[
            ("transactionId", Some(transaction_id)),
            (
                "authenticateServerResponse",
                Some(b64_authenticate_server_response),
            ),
        ],
        [
            "profileMetadata",
            "smdpSigned2",
            "smdpSignature2",
            "smdpCertificate",
        ],
    )?;
    Ok(PrepareDownloadParam {
        b64_profile_metadata: base64_text(metadata),
        b64_smdp_signed2: base64_text(signed2),
        b64_smdp_signature2: base64_text(signature2),
        b64_smdp_certificate: base64_text(certificate),
    })
}

/// ES9+ GetBoundProfilePackage. Returns the bound profile package as base64
/// (`es9p_get_bound_profile_package_r`).
pub fn get_bound_profile_package(
    http: &mut (impl Http + ?Sized),
    server_address: &str,
    transaction_id: &str,
    b64_prepare_download_response: &str,
) -> Result<String, Es9pStatus> {
    let [bpp] = exchange_json(
        http,
        server_address,
        PATH_GET_BOUND_PROFILE_PACKAGE,
        &[
            ("transactionId", Some(transaction_id)),
            (
                "prepareDownloadResponse",
                Some(b64_prepare_download_response),
            ),
        ],
        ["boundProfilePackage"],
    )?;
    Ok(base64_text(bpp))
}

/// ES9+ CancelSession; any 2xx answer is success (`es9p_cancel_session_r`).
pub fn cancel_session(
    http: &mut (impl Http + ?Sized),
    server_address: &str,
    transaction_id: &str,
    b64_cancel_session_response: &str,
) -> Result<(), Es9pStatus> {
    exchange(
        http,
        server_address,
        PATH_CANCEL_SESSION,
        &[
            ("transactionId", Some(transaction_id)),
            ("cancelSessionResponse", Some(b64_cancel_session_response)),
        ],
    )
    .map(drop)
}

/// ES9+ HandleNotification; any 2xx answer is success (`es9p_handle_notification`).
pub fn handle_notification(
    http: &mut (impl Http + ?Sized),
    server_address: &str,
    b64_pending_notification: &str,
) -> Result<(), Es9pStatus> {
    exchange(
        http,
        server_address,
        PATH_HANDLE_NOTIFICATION,
        &[("pendingNotification", Some(b64_pending_notification))],
    )
    .map(drop)
}

/// The SGP.22 description of a subject code / reason code pair (`es9p_error_message`).
pub fn error_message(subject_code: &str, reason_code: &str) -> Option<&'static str> {
    error_message_bytes(
        c_str(subject_code).as_bytes(),
        c_str(reason_code).as_bytes(),
    )
}

#[rustfmt::skip]
const ERRORS: &[(&str, &str, &str)] = &[
    ("8.1",    "4.8", "eUICC does not have sufficient space for this Profile"),
    ("8.1",    "6.1", "eUICC signature is invalid or serverChallenge is invalid"),
    ("8.1.1",  "2.2", "Indicates that the EID is missing in the context of this order (SM-DS address provided or MatchingID value is empty)"),
    ("8.1.1",  "3.1", "Indicates that a different EID is already associated with this ICCID"),
    ("8.1.1",  "3.8", "EID doesn't match the expected value"),
    ("8.1.2",  "6.1", "EUM Certificate is invalid"),
    ("8.1.2",  "6.3", "EUM Certificate has expired"),
    ("8.1.3",  "6.1", "eUICC Certificate is invalid"),
    ("8.1.3",  "6.3", "eUICC Certificate has expired"),
    ("8.2",    "1.2", "Profile has not yet been released"),
    ("8.2",    "3.7", "BPP is not available for a new binding"),
    ("8.2.1",  "1.2", "Indicates that the function caller is not allowed to perform this function on the target Profile"),
    ("8.2.1",  "3.1", "Indicates that a different EID is associated with this ICCID"),
    ("8.2.1",  "3.3", "Indicates that the Profile identified by the provided ICCID is not available"),
    ("8.2.1",  "3.5", "Indicates that the target Profile cannot be released"),
    ("8.2.1",  "3.9", "Indicates that the Profile Type identified by this Profile Type is unknown to the SM-DP+"),
    ("8.2.5",  "1.2", "Indicates that the function caller is not allowed to perform this function on the Profile Type"),
    ("8.2.5",  "3.7", "No more Profile available for the requested Profile Type"),
    ("8.2.5",  "3.8", "Indicates that the Profile Type identified by this Profile Type is not aligned with the Profile Type of Profile identified by the ICCID"),
    ("8.2.5",  "3.9", "Indicates that the Profile Type identified by this Profile Type is unknown to the SM-DP+"),
    ("8.2.5",  "4.3", "No eligible Profile for this eUICC/Device"),
    ("8.2.6",  "3.1", "Indicates that a different MatchingID is associated with this ICCID"),
    ("8.2.6",  "3.3", "Conflicting MatchingID value"),
    ("8.2.6",  "3.8", "MatchingID (AC_Token or EventID) is refused"),
    ("8.2.7",  "2.2", "Confirmation Code is missing"),
    ("8.2.7",  "3.8", "Confirmation Code is refused"),
    ("8.2.7",  "6.4", "The maximum number of retries for the Confirmation Code has been exceeded"),
    ("8.8",    "3.1", "The provided SM-DP+ OID is invalid"),
    ("8.8.1",  "3.8", "Invalid SM-DP+ Address"),
    ("8.8.2",  "3.1", "None of the proposed Public Key Identifiers is supported by the SM-DP+"),
    ("8.8.3",  "3.1", "The Specification Version Number indicated by the eUICC is not supported by the SM-DP+"),
    ("8.8.4",  "3.7", "The SM-DP+ has no CERT.DPauth.ECDSA signed by one of the CI Public Key supported by the eUICC"),
    ("8.8.5",  "4.1", "The Download order has expired"),
    ("8.8.5",  "6.4", "The maximum number of retries for the Profile download order has been exceeded"),
    ("8.9",    "4.2", "Root SM-DS has raised an error"),
    ("8.9",    "5.1", "Root SM-DS was unavailable"),
    ("8.9.1",  "3.8", "Invalid SM-DS Address"),
    ("8.9.2",  "3.1", "None of the proposed Public Key Identifiers is supported by the SM-DS"),
    ("8.9.3",  "3.1", "The Specification Version Number indicated by the eUICC is not supported by the SM-DS"),
    ("8.9.4",  "3.7", "The SM-DS has no CERT.DS.ECDSA signed by one of the GSMA CI Public Key supported by the eUICC"),
    ("8.9.5",  "3.3", "The Event Record already exist in the SM-DS (EventID duplicated)"),
    ("8.9.5",  "3.9", "No Event identified by the Event ID for the EID exists"),
    ("8.10.1", "3.9", "The RSP session identified by the TransactionID is unknown"),
    ("8.11.1", "3.9", "Unknown CI Public Key. The CI used by the EUM Certificate is not a trusted root."),
];

fn error_message_bytes(subject_code: &[u8], reason_code: &[u8]) -> Option<&'static str> {
    ERRORS
        .iter()
        .find(|(subject, reason, _)| {
            subject.as_bytes() == subject_code && reason.as_bytes() == reason_code
        })
        .map(|(_, _, description)| *description)
}

/// Capacity (without the NUL) of `subjectCode` and `reasonCode` in `ctx->http.status`.
const CODE_LEN: usize = 8;
/// Capacity (without the NUL) of `subjectIdentifier` and `message` in `ctx->http.status`.
const TEXT_LEN: usize = 128;

/// `ctx->http.status` while it is being filled, as the bytes C would hold.
#[derive(Clone)]
struct Status {
    subject_code: Vec<u8>,
    reason_code: Vec<u8>,
    subject_identifier: Vec<u8>,
    message: Vec<u8>,
}

impl Status {
    fn new(subject_identifier: &[u8], message: &[u8]) -> Self {
        Status {
            subject_code: b"0.0.0".to_vec(),
            reason_code: b"0.0.0".to_vec(),
            subject_identifier: field(subject_identifier, TEXT_LEN),
            message: field(message, TEXT_LEN),
        }
    }

    /// Takes what the server reported in `statusCodeData` (which may be any JSON value).
    fn apply(&mut self, data: &Json) {
        if let Some(Json::Str(v)) = data.get("reasonCode") {
            self.reason_code = field(v, CODE_LEN);
        }
        if let Some(Json::Str(v)) = data.get("subjectCode") {
            self.subject_code = field(v, CODE_LEN);
        }
        if let Some(Json::Str(v)) = data.get("subjectIdentifier") {
            self.subject_identifier = field(v, TEXT_LEN);
        }
        self.message = match data.get("message") {
            Some(Json::Str(v)) => field(v, TEXT_LEN),
            _ => match error_message_bytes(&self.subject_code, &self.reason_code) {
                Some(description) => field(description.as_bytes(), TEXT_LEN),
                None => {
                    let mut m = b"subject-code: ".to_vec();
                    m.extend_from_slice(&self.subject_code);
                    m.extend_from_slice(b", reason-code: ");
                    m.extend_from_slice(&self.reason_code);
                    field(&m, TEXT_LEN)
                }
            },
        };
    }

    fn into_error(self) -> Es9pStatus {
        Es9pStatus {
            subject_code: text(self.subject_code),
            reason_code: text(self.reason_code),
            subject_identifier: text(self.subject_identifier),
            message: text(self.message),
            transport_error: None,
        }
    }
}

/// `snprintf(buf, cap + 1, "%s", value)`: the C string in `value`, cut to `cap` bytes.
fn field(value: &[u8], cap: usize) -> Vec<u8> {
    let v = c_bytes(value);
    v[..v.len().min(cap)].to_vec()
}

/// The bytes before the first NUL, as C sees a string.
fn c_bytes(b: &[u8]) -> &[u8] {
    b.iter().position(|&c| c == 0).map_or(b, |n| &b[..n])
}

/// The text before the first NUL, as C sees a string.
fn c_str(s: &str) -> &str {
    s.find('\0').map_or(s, |n| &s[..n])
}

fn text(v: Vec<u8>) -> String {
    String::from_utf8(v).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// `es9p_base64_trim`: drops line breaks, spaces and tabs.
fn base64_text(mut v: Vec<u8>) -> String {
    v.retain(|b| !matches!(b, b'\n' | b'\r' | b' ' | b'\t'));
    text(v)
}

/// POSTs the request object and returns the body of a 2xx answer (`es9p_trans_ex` and the
/// transport half of `es9p_trans_json`).
fn exchange(
    http: &mut (impl Http + ?Sized),
    server_address: &str,
    path: &str,
    request: &[(&str, Option<&str>)],
) -> Result<Vec<u8>, Es9pStatus> {
    let url = format!("https://{}{}", c_str(server_address), path);
    let (code, body) = http
        .post(&url, &HEADERS, &print_object(request))
        .map_err(|e| {
            let mut status = Status::new(b"unknown", b"HTTP transport failed").into_error();
            status.transport_error = Some(e);
            status
        })?;
    if code / 100 != 2 {
        // C prints the uint32_t status with "%d".
        let shown = (code as i32).to_string();
        return Err(Status::new(shown.as_bytes(), b"HTTP status code error").into_error());
    }
    Ok(body)
}

/// [`exchange`], then reads the named string members of the response object (the rest of
/// `es9p_trans_json`).
fn exchange_json<const N: usize>(
    http: &mut (impl Http + ?Sized),
    server_address: &str,
    path: &str,
    request: &[(&str, Option<&str>)],
    keys: [&str; N],
) -> Result<[Vec<u8>; N], Es9pStatus> {
    let body = exchange(http, server_address, path, request)?;
    read_response(&body, keys).map_err(Status::into_error)
}

fn read_response<const N: usize>(body: &[u8], keys: [&str; N]) -> Result<[Vec<u8>; N], Status> {
    let root = parse(body).ok_or_else(|| Status::new(b"root", b"Not JSON"))?;
    if !matches!(root, Json::Obj(_)) {
        return Err(Status::new(b"root", b"Not Object"));
    }
    let header = root
        .get("header")
        .ok_or_else(|| Status::new(b"header", b"Critical object missing"))?;
    let execution = header
        .get("functionExecutionStatus")
        .ok_or_else(|| Status::new(b"functionExecutionStatus", b"Critical object missing"))?;
    let mut status = Status::new(b"unknown", b"unknown");
    if let Some(data) = execution.get("statusCodeData") {
        status.apply(data);
    }
    let mut out: [Vec<u8>; N] = std::array::from_fn(|_| Vec::new());
    for (slot, key) in out.iter_mut().zip(keys) {
        match root.get(key) {
            Some(Json::Str(v)) => *slot = c_bytes(v).to_vec(),
            _ => return Err(status),
        }
    }
    Ok(out)
}

/// `cJSON_PrintUnformatted` of an object of strings (`None` prints as `null`), members in order.
fn print_object(members: &[(&str, Option<&str>)]) -> Vec<u8> {
    let mut out = vec![b'{'];
    for (i, (name, value)) in members.iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        print_string(&mut out, c_str(name).as_bytes());
        out.push(b':');
        match value {
            Some(v) => print_string(&mut out, c_str(v).as_bytes()),
            None => out.extend_from_slice(b"null"),
        }
    }
    out.push(b'}');
    out
}

/// cJSON's `print_string_ptr`: escapes `"`, `\` and control characters; copies other bytes.
fn print_string(out: &mut Vec<u8>, s: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push(b'"');
    for &b in s {
        match b {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x0C => out.extend_from_slice(b"\\f"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            0..=0x1F => out.extend_from_slice(&[
                b'\\',
                b'u',
                b'0',
                b'0',
                HEX[usize::from(b >> 4)],
                HEX[usize::from(b & 15)],
            ]),
            _ => out.push(b),
        }
    }
    out.push(b'"');
}

/// A parsed JSON value, keeping only what ES9+ reads: strings and shallow objects.
#[derive(Debug, PartialEq)]
enum Json {
    Str(Vec<u8>),
    /// Members in document order, duplicates included.
    Obj(Vec<(Vec<u8>, Json)>),
    /// null, booleans, numbers, arrays, and objects nested deeper than [`KEEP_DEPTH`].
    Other,
}

impl Json {
    /// `cJSON_GetObjectItem`: the first member whose name, read as a C string, equals `name`
    /// ignoring ASCII case. Values other than objects have no members.
    fn get(&self, name: &str) -> Option<&Json> {
        match self {
            Json::Obj(members) => members
                .iter()
                .find(|(n, _)| c_bytes(n).eq_ignore_ascii_case(name.as_bytes()))
                .map(|(_, v)| v),
            _ => None,
        }
    }
}

/// `CJSON_NESTING_LIMIT`: arrays and objects open at once.
const NESTING_LIMIT: usize = 1000;
/// Objects are kept down to the root's great-grandchildren
/// (`header.functionExecutionStatus.statusCodeData`); deeper ones are checked and dropped.
const KEEP_DEPTH: usize = 4;

enum Frame {
    Array,
    /// Members so far (`None` when this object is not kept) and the name of the member whose
    /// value is being parsed.
    Object(Option<Vec<(Vec<u8>, Json)>>, Vec<u8>),
}

/// `cJSON_Parse`: input ends at the first NUL; a UTF-8 BOM is skipped; every byte up to 0x20 is
/// whitespace; anything after the first complete value is ignored.
fn parse(input: &[u8]) -> Option<Json> {
    let mut p = Parser {
        s: c_bytes(input),
        i: 0,
    };
    if p.s.starts_with(b"\xEF\xBB\xBF") {
        p.i = 3;
    }
    p.skip_whitespace();
    let mut stack: Vec<Frame> = Vec::new();
    'value: loop {
        // One value starts at p.i (whitespace already skipped).
        let mut value = match p.peek()? {
            b'n' if p.rest().starts_with(b"null") => {
                p.i += 4;
                Json::Other
            }
            b'f' if p.rest().starts_with(b"false") => {
                p.i += 5;
                Json::Other
            }
            b't' if p.rest().starts_with(b"true") => {
                p.i += 4;
                Json::Other
            }
            b'"' => Json::Str(p.string()?),
            b'-' | b'0'..=b'9' => {
                p.number()?;
                Json::Other
            }
            b'[' => {
                if stack.len() >= NESTING_LIMIT {
                    return None;
                }
                p.i += 1;
                p.skip_whitespace();
                if p.peek() == Some(b']') {
                    p.i += 1;
                    Json::Other
                } else {
                    stack.push(Frame::Array);
                    continue 'value;
                }
            }
            b'{' => {
                if stack.len() >= NESTING_LIMIT {
                    return None;
                }
                let keep = stack.len() < KEEP_DEPTH;
                p.i += 1;
                p.skip_whitespace();
                if p.peek() == Some(b'}') {
                    p.i += 1;
                    if keep {
                        Json::Obj(Vec::new())
                    } else {
                        Json::Other
                    }
                } else {
                    let name = p.member_name()?;
                    stack.push(Frame::Object(keep.then(Vec::new), name));
                    continue 'value;
                }
            }
            _ => return None,
        };
        // Hand the finished value to its container; close containers that end here.
        loop {
            let Some(frame) = stack.last_mut() else {
                return Some(value);
            };
            if let Frame::Object(Some(members), name) = frame {
                members.push((std::mem::take(name), value));
            }
            p.skip_whitespace();
            let close = match frame {
                Frame::Array => b']',
                Frame::Object(..) => b'}',
            };
            match p.peek() {
                Some(b',') => {
                    p.i += 1;
                    p.skip_whitespace();
                    if let Frame::Object(_, name) = frame {
                        *name = p.member_name()?;
                    }
                    continue 'value;
                }
                Some(c) if c == close => {
                    p.i += 1;
                    value = match stack.pop() {
                        Some(Frame::Object(Some(members), _)) => Json::Obj(members),
                        _ => Json::Other,
                    };
                }
                _ => return None,
            }
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn rest(&self) -> &[u8] {
        self.s.get(self.i..).unwrap_or_default()
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(|b| b <= 32) {
            self.i += 1;
        }
    }

    /// A member name, the `:` after it and the whitespace before the value.
    fn member_name(&mut self) -> Option<Vec<u8>> {
        if self.peek() != Some(b'"') {
            return None;
        }
        let name = self.string()?;
        self.skip_whitespace();
        if self.peek() != Some(b':') {
            return None;
        }
        self.i += 1;
        self.skip_whitespace();
        Some(name)
    }

    /// cJSON's `parse_string`, at the opening quote. Bytes are copied as they are (control
    /// characters and invalid UTF-8 included); `\u0000` and `\u` with bad hex digits give NUL.
    fn string(&mut self) -> Option<Vec<u8>> {
        let s = self.s;
        let start = self.i + 1;
        // Find the closing quote, stepping over the byte after each backslash.
        let mut end = start;
        loop {
            match *s.get(end)? {
                b'"' => break,
                b'\\' => end += 2,
                _ => end += 1,
            }
        }
        let mut out = Vec::with_capacity(end - start);
        let mut at = start;
        while at < end {
            let b = s[at];
            if b != b'\\' {
                out.push(b);
                at += 1;
                continue;
            }
            // The byte after a backslash may be the closing quote itself when a preceding \u
            // sequence swallowed the backslash's partner; cJSON then reads it as \".
            at += match *s.get(at + 1)? {
                b'b' => {
                    out.push(0x08);
                    2
                }
                b'f' => {
                    out.push(0x0C);
                    2
                }
                b'n' => {
                    out.push(b'\n');
                    2
                }
                b'r' => {
                    out.push(b'\r');
                    2
                }
                b't' => {
                    out.push(b'\t');
                    2
                }
                c @ (b'"' | b'\\' | b'/') => {
                    out.push(c);
                    2
                }
                b'u' => utf16_literal(s.get(at..end)?, &mut out)?,
                _ => return None,
            };
        }
        self.i = end + 1;
        Some(out)
    }

    /// cJSON's `parse_number`: the run of `0-9 + - e E .` is handed to `strtod`, and parsing
    /// continues after what `strtod` used. The value itself is not needed. The run may be of any
    /// length: lpac's pinned cJSON fork copies it whole, whereas upstream 1.7.18 copies at most
    /// 63 bytes and so rejects longer numbers. Follow the fork, which is what lpac links.
    fn number(&mut self) -> Option<()> {
        let run = self.rest();
        let run = &run[..run
            .iter()
            .position(|b| !matches!(b, b'0'..=b'9' | b'+' | b'-' | b'e' | b'E' | b'.'))
            .unwrap_or(run.len())];
        match strtod_len(run) {
            0 => None,
            n => {
                self.i += n;
                Some(())
            }
        }
    }
}

/// How many bytes of `s` (made only of `0-9 + - e E .`) C's `strtod` would convert; 0 if none.
fn strtod_len(s: &[u8]) -> usize {
    let digits = |from: usize| {
        s.get(from..)
            .unwrap_or_default()
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count()
    };
    let mut i = usize::from(matches!(s.first(), Some(b'+' | b'-')));
    let int_digits = digits(i);
    i += int_digits;
    let mut frac_digits = 0;
    if s.get(i) == Some(&b'.') {
        frac_digits = digits(i + 1);
        if int_digits + frac_digits > 0 {
            i += 1 + frac_digits;
        }
    }
    if int_digits + frac_digits == 0 {
        return 0;
    }
    if matches!(s.get(i), Some(b'e' | b'E')) {
        let mut e = i + 1;
        if matches!(s.get(e), Some(b'+' | b'-')) {
            e += 1;
        }
        let exp_digits = digits(e);
        if exp_digits > 0 {
            i = e + exp_digits;
        }
    }
    i
}

/// cJSON's `parse_hex4`: four hex digits, or 0 if any is not one.
fn hex4(s: &[u8]) -> u32 {
    let mut h = 0;
    for &c in s.iter().take(4) {
        let d = match c {
            b'0'..=b'9' => c - b'0',
            b'A'..=b'F' => c - b'A' + 10,
            b'a'..=b'f' => c - b'a' + 10,
            _ => return 0,
        };
        h = (h << 4) | u32::from(d);
    }
    h
}

/// cJSON's `utf16_literal_to_utf8`. `s` runs from the backslash to the closing quote
/// (exclusive). Appends the UTF-8 and returns how many input bytes were used.
fn utf16_literal(s: &[u8], out: &mut Vec<u8>) -> Option<usize> {
    if s.len() < 6 {
        return None;
    }
    let first = hex4(&s[2..6]);
    if (0xDC00..=0xDFFF).contains(&first) {
        return None;
    }
    let (codepoint, used) = if (0xD800..=0xDBFF).contains(&first) {
        let second = s.get(6..12)?;
        if !second.starts_with(b"\\u") {
            return None;
        }
        let second = hex4(&second[2..]);
        if !(0xDC00..=0xDFFF).contains(&second) {
            return None;
        }
        (0x10000 + (((first & 0x3FF) << 10) | (second & 0x3FF)), 12)
    } else {
        (first, 6)
    };
    // Surrogates were excluded above, so this is always a char; '\0' gives a NUL byte.
    let c = char::from_u32(codepoint)?;
    out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
    Some(used)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers every request with `reply` and records what was sent.
    struct Server {
        reply: Result<(u32, Vec<u8>), String>,
        sent: Vec<(String, Vec<String>, Vec<u8>)>,
    }

    impl Server {
        fn ok(body: &str) -> Self {
            Self::reply(200, body.as_bytes())
        }
        fn reply(code: u32, body: &[u8]) -> Self {
            Server {
                reply: Ok((code, body.to_vec())),
                sent: Vec::new(),
            }
        }
        fn body(&self) -> String {
            String::from_utf8(self.sent[0].2.clone()).unwrap()
        }
    }

    impl Http for Server {
        fn post(
            &mut self,
            url: &str,
            headers: &[&str],
            body: &[u8],
        ) -> Result<(u32, Vec<u8>), String> {
            self.sent.push((
                url.to_string(),
                headers.iter().map(|h| h.to_string()).collect(),
                body.to_vec(),
            ));
            self.reply.clone()
        }
    }

    const HDR: &str = r#""header":{"functionExecutionStatus":{"status":"Executed-Success"}}"#;

    fn status(sc: &str, rc: &str, si: &str, msg: &str) -> Es9pStatus {
        Es9pStatus {
            subject_code: sc.into(),
            reason_code: rc.into(),
            subject_identifier: si.into(),
            message: msg.into(),
            transport_error: None,
        }
    }

    fn failure(sc: &str, rc: &str, msg: Option<&str>) -> String {
        let mut data = format!(r#""subjectCode":"{sc}","reasonCode":"{rc}""#);
        if let Some(m) = msg {
            data += &format!(r#","message":"{m}""#);
        }
        format!(
            r#"{{"header":{{"functionExecutionStatus":{{"status":"Failed","statusCodeData":{{{data}}}}}}}}}"#
        )
    }

    /// The status `initiate_authentication` fails with for a 200 answer carrying `body`.
    fn ia_status(body: &[u8]) -> Es9pStatus {
        let mut s = Server::reply(200, body);
        initiate_authentication(&mut s, "a", "b", "c").unwrap_err()
    }

    fn bpp(body: &[u8]) -> Result<String, Es9pStatus> {
        get_bound_profile_package(&mut Server::reply(200, body), "a", "t", "p")
    }

    // ---- requests ----

    #[test]
    fn initiate_authentication_request_and_response() {
        let mut s = Server::ok(&format!(
            r#"{{{HDR},"transactionId":" 0A1B ","serverSigned1":"MAo\nB\r\n","serverSignature1":"X\tY Z","euiccCiPKIdToBeUsed":"BBQ=","serverCertificate":"MII"}}"#
        ));
        let (tid, p) =
            initiate_authentication(&mut s, "smdp.example.com", "Y2hhbA==", "aW5mbzE=").unwrap();
        assert_eq!(tid, " 0A1B ", "the transactionId is not trimmed");
        assert_eq!(
            p,
            AuthenticateServerParam {
                b64_server_signed1: "MAoB".into(),
                b64_server_signature1: "XYZ".into(),
                b64_euicc_ci_pkid_to_be_used: "BBQ=".into(),
                b64_server_certificate: "MII".into(),
            }
        );
        assert_eq!(s.sent.len(), 1);
        assert_eq!(
            s.sent[0].0,
            "https://smdp.example.com/gsma/rsp2/es9plus/initiateAuthentication"
        );
        assert_eq!(
            s.sent[0].1,
            [
                "User-Agent: gsma-rsp-lpad",
                "X-Admin-Protocol: gsma/rsp/v2.2.2",
                "Content-Type: application/json"
            ]
        );
        assert_eq!(
            s.body(),
            r#"{"smdpAddress":"smdp.example.com","euiccChallenge":"Y2hhbA==","euiccInfo1":"aW5mbzE="}"#
        );
    }

    #[test]
    fn authenticate_client_request_and_response() {
        let mut s = Server::ok(&format!(
            r#"{{{HDR},"profileMetadata":"vyU=","smdpSigned2":"MA==\n","smdpSignature2":"XzdA","smdpCertificate":"MIIC"}}"#
        ));
        let p = authenticate_client(&mut s, "dp.example", "TID1", "vzhA").unwrap();
        assert_eq!(
            p,
            PrepareDownloadParam {
                b64_profile_metadata: "vyU=".into(),
                b64_smdp_signed2: "MA==".into(),
                b64_smdp_signature2: "XzdA".into(),
                b64_smdp_certificate: "MIIC".into(),
            }
        );
        assert_eq!(
            s.sent[0].0,
            "https://dp.example/gsma/rsp2/es9plus/authenticateClient"
        );
        assert_eq!(
            s.body(),
            r#"{"transactionId":"TID1","authenticateServerResponse":"vzhA"}"#
        );
    }

    #[test]
    fn get_bound_profile_package_request_and_response() {
        let mut s = Server::ok(&format!(
            r#"{{{HDR},"boundProfilePackage":"vzZ\r\nC  A\t="}}"#
        ));
        assert_eq!(
            get_bound_profile_package(&mut s, "dp", "T", "vyE=").unwrap(),
            "vzZCA="
        );
        assert_eq!(
            s.sent[0].0,
            "https://dp/gsma/rsp2/es9plus/getBoundProfilePackage"
        );
        assert_eq!(
            s.body(),
            r#"{"transactionId":"T","prepareDownloadResponse":"vyE="}"#
        );
    }

    #[test]
    fn cancel_session_and_handle_notification_need_only_2xx() {
        for code in [200, 204, 250, 299] {
            let mut s = Server::reply(code, b"\xFFnot json at all");
            cancel_session(&mut s, "dp", "T", "vyI=").unwrap();
            assert_eq!(s.sent[0].0, "https://dp/gsma/rsp2/es9plus/cancelSession");
            assert_eq!(
                s.body(),
                r#"{"transactionId":"T","cancelSessionResponse":"vyI="}"#
            );

            let mut s = Server::reply(code, b"");
            handle_notification(&mut s, "rsp.example:8443", "vzc=").unwrap();
            assert_eq!(
                s.sent[0].0,
                "https://rsp.example:8443/gsma/rsp2/es9plus/handleNotification"
            );
            assert_eq!(s.body(), r#"{"pendingNotification":"vzc="}"#);
        }
    }

    #[test]
    fn requests_escape_like_cjson() {
        let mut s = Server::ok("{}");
        let arg = "q\"b\\s/\u{8}\u{c}\n\r\t\u{1}\u{1f}\u{7f}é\u{1F600}";
        handle_notification(&mut s, "x", arg).unwrap();
        assert_eq!(
            s.body(),
            "{\"pendingNotification\":\"q\\\"b\\\\s/\\b\\f\\n\\r\\t\\u0001\\u001f\u{7f}é\u{1F600}\"}",
            "solidus, DEL and non-ASCII are copied; other control characters use lowercase \\u00xx"
        );
        assert_eq!(
            print_object(&[("k", None), ("e", Some(""))]),
            br#"{"k":null,"e":""}"#
        );
        assert_eq!(print_object(&[]), b"{}");
    }

    #[test]
    fn arguments_end_at_nul_like_c_strings() {
        let mut s = Server::ok("{}");
        cancel_session(&mut s, "host\0.evil", "T\0X", "\0abc").unwrap();
        assert_eq!(s.sent[0].0, "https://host/gsma/rsp2/es9plus/cancelSession");
        assert_eq!(
            s.body(),
            r#"{"transactionId":"T","cancelSessionResponse":""}"#
        );
    }

    #[test]
    fn works_through_a_trait_object() {
        let mut server = Server::ok("{}");
        let http: &mut dyn Http = &mut server;
        handle_notification(http, "x", "y").unwrap();
        cancel_session(http, "x", "t", "y").unwrap();
        assert_eq!(server.sent.len(), 2);
    }

    #[test]
    fn empty_arguments_are_sent_as_empty_strings() {
        let mut s = Server::ok("{}");
        handle_notification(&mut s, "", "").unwrap();
        assert_eq!(s.sent[0].0, "https:///gsma/rsp2/es9plus/handleNotification");
        assert_eq!(s.body(), r#"{"pendingNotification":""}"#);
    }

    #[test]
    fn long_arguments_are_sent_whole() {
        let big = "A".repeat(300_000);
        let mut s = Server::ok("{}");
        handle_notification(&mut s, "x", &big).unwrap();
        assert_eq!(
            s.sent[0].2.len(),
            big.len() + r#"{"pendingNotification":""}"#.len()
        );
    }

    // ---- transport and HTTP status ----

    #[test]
    fn transport_failure_keeps_the_cause() {
        let mut s = Server {
            reply: Err("tls: certificate expired".into()),
            sent: Vec::new(),
        };
        let e = handle_notification(&mut s, "x", "y").unwrap_err();
        assert_eq!(e.message, "HTTP transport failed");
        assert_eq!(e.to_string(), "HTTP transport failed");
        assert_eq!(
            (e.subject_code.as_str(), e.reason_code.as_str()),
            ("0.0.0", "0.0.0")
        );
        assert_eq!(e.subject_identifier, "unknown");
        assert_eq!(
            e.transport_error.as_deref(),
            Some("tls: certificate expired")
        );
        let mut s = Server {
            reply: Err(String::new()),
            sent: Vec::new(),
        };
        assert_eq!(
            initiate_authentication(&mut s, "x", "y", "z")
                .unwrap_err()
                .message,
            "HTTP transport failed"
        );
    }

    #[test]
    fn non_2xx_fails_with_the_status_number() {
        for (code, shown) in [
            (0u32, "0"),
            (100, "100"),
            (199, "199"),
            (300, "300"),
            (404, "404"),
            (500, "500"),
            (2_147_483_648, "-2147483648"),
            (u32::MAX, "-1"),
        ] {
            let ok = format!(r#"{{{HDR},"boundProfilePackage":"QQ=="}}"#);
            let e =
                get_bound_profile_package(&mut Server::reply(code, ok.as_bytes()), "a", "t", "p")
                    .unwrap_err();
            assert_eq!(
                e,
                status("0.0.0", "0.0.0", shown, "HTTP status code error"),
                "{code}"
            );
            let e = cancel_session(&mut Server::reply(code, b""), "a", "t", "p").unwrap_err();
            assert_eq!(e.message, "HTTP status code error");
        }
        let ok = format!(r#"{{{HDR},"boundProfilePackage":"QQ=="}}"#);
        assert_eq!(
            get_bound_profile_package(&mut Server::reply(299, ok.as_bytes()), "a", "t", "p")
                .unwrap(),
            "QQ=="
        );
    }

    // ---- response structure ----

    #[test]
    fn malformed_responses() {
        let not_json = status("0.0.0", "0.0.0", "root", "Not JSON");
        let not_object = status("0.0.0", "0.0.0", "root", "Not Object");
        let no_header = status("0.0.0", "0.0.0", "header", "Critical object missing");
        let no_fes = status(
            "0.0.0",
            "0.0.0",
            "functionExecutionStatus",
            "Critical object missing",
        );
        let unknown = status("0.0.0", "0.0.0", "unknown", "unknown");
        let cases: &[(&[u8], &Es9pStatus)] = &[
            (b"", &not_json),
            (b"   \n\t", &not_json),
            (b"\0{}", &not_json),
            (b"{", &not_json),
            (b"{\"header\"", &not_json),
            (b"{\"header\":}", &not_json),
            (b"{,}", &not_json),
            (b"{\"a\":1,}", &not_json),
            (b"{'a':1}", &not_json),
            (b"{a:1}", &not_json),
            (b"<html>502</html>", &not_json),
            (b"nul", &not_json),
            (b"\xEF\xBB\xBF", &not_json),
            (b"[]", &not_object),
            (b"[{\"header\":{}}]", &not_object),
            (b"\"str\"", &not_object),
            (b"123", &not_object),
            (b"-1e5x", &not_object),
            (b"null", &not_object),
            (b"true", &not_object),
            (b"falsey", &not_object),
            (b"{}", &no_header),
            (b"{\"Header \":{}}", &no_header),
            (b"{\"header\":{}}", &no_fes),
            (b"{\"header\":null}", &no_fes),
            (b"{\"header\":[{\"functionExecutionStatus\":{}}]}", &no_fes),
            (b"{\"header\":\"functionExecutionStatus\"}", &no_fes),
            (b"{\"header\":{\"functionExecutionStatus\":null}}", &unknown),
            (b"{\"header\":{\"functionExecutionStatus\":{}}}", &unknown),
            (b"{\"header\":{\"functionExecutionStatus\":{}},\"boundProfilePackage\":null}", &unknown),
            (b"{\"header\":{\"functionExecutionStatus\":{}},\"boundProfilePackage\":5}", &unknown),
            (b"{\"header\":{\"functionExecutionStatus\":{}},\"boundProfilePackage\":[\"QQ==\"]}", &unknown),
            (b"{\"header\":{\"functionExecutionStatus\":{}},\"boundProfilePackage\":{\"a\":\"b\"}}", &unknown),
            (b"{\"header\":{\"functionExecutionStatus\":{\"boundProfilePackage\":\"QQ==\"}}}", &unknown),
        ];
        for (body, want) in cases {
            assert_eq!(
                &&bpp(body).unwrap_err(),
                want,
                "{}",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[test]
    fn present_members_succeed_even_with_a_failure_status() {
        // libeuicc does not look at functionExecutionStatus.status; the members decide.
        let body = r#"{"header":{"functionExecutionStatus":{"status":"Failed","statusCodeData":{"subjectCode":"8.1"}}},"boundProfilePackage":"QQ=="}"#;
        assert_eq!(bpp(body.as_bytes()).unwrap(), "QQ==");
    }

    #[test]
    fn members_match_ignoring_case_and_the_first_one_wins() {
        let body = r#"{"HEADER":{"FunctionExecutionStatus":{}},"boundprofilepackage":"first","boundProfilePackage":"second"}"#;
        assert_eq!(bpp(body.as_bytes()).unwrap(), "first");
        // A non-string first match fails even if a later duplicate is a string.
        let body = r#"{"header":{"functionExecutionStatus":{}},"boundProfilePackage":1,"boundProfilePackage":"QQ=="}"#;
        assert!(bpp(body.as_bytes()).is_err());
        // Member names are C strings: "\u0000" ends them.
        let body = r#"{"header\u0000x":{"functionExecutionStatus":{}},"boundProfilePackage\u0000junk":"QQ=="}"#;
        assert_eq!(bpp(body.as_bytes()).unwrap(), "QQ==");
        // Non-ASCII letters are not folded.
        let body = "{\"header\":{\"functionExecutionStatus\":{}},\"boundProfilePackagé\":\"QQ==\"}";
        assert!(bpp(body.as_bytes()).is_err());
    }

    // ---- statusCodeData ----

    #[test]
    fn status_code_data_with_message() {
        let e = ia_status(failure("8.8.5", "4.1", Some("Order expired")).as_bytes());
        assert_eq!(e, status("8.8.5", "4.1", "unknown", "Order expired"));
        let body = r#"{"header":{"functionExecutionStatus":{"statusCodeData":{"subjectCode":"8.1","reasonCode":"6.1","subjectIdentifier":"EUICC","message":""}}}}"#;
        assert_eq!(
            ia_status(body.as_bytes()),
            status("8.1", "6.1", "EUICC", "")
        );
    }

    #[test]
    fn status_code_data_without_message_uses_the_table() {
        let e = ia_status(failure("8.2.6", "3.8", None).as_bytes());
        assert_eq!(e.message, "MatchingID (AC_Token or EventID) is refused");
        assert_eq!(e.subject_identifier, "unknown");
        // A non-string message counts as missing.
        let body = r#"{"header":{"functionExecutionStatus":{"statusCodeData":{"subjectCode":"8.1","reasonCode":"4.8","message":7}}}}"#;
        assert_eq!(
            ia_status(body.as_bytes()).message,
            "eUICC does not have sufficient space for this Profile"
        );
        // The one table entry longer than the C buffer is cut to 128 bytes.
        let e = ia_status(failure("8.2.5", "3.8", None).as_bytes());
        assert_eq!(e.message.len(), 128);
        assert_eq!(
            e.message,
            "Indicates that the Profile Type identified by this Profile Type is not aligned with the Profile Type of Profile identified by th"
        );
    }

    #[test]
    fn every_table_entry_is_found() {
        for (sc, rc, description) in ERRORS {
            assert_eq!(error_message(sc, rc), Some(*description));
            let e = ia_status(failure(sc, rc, None).as_bytes());
            assert_eq!(e.message, &description[..description.len().min(128)]);
        }
        assert_eq!(error_message("8.1", "9.9"), None);
        assert_eq!(error_message("8.1\0x", "4.8"), error_message("8.1", "4.8"));
        assert_eq!(error_message("8.1 ", "4.8"), None, "exact match only");
    }

    #[test]
    fn unknown_codes_are_spelled_out() {
        let e = ia_status(failure("1.2", "3.4", None).as_bytes());
        assert_eq!(e.message, "subject-code: 1.2, reason-code: 3.4");
        // statusCodeData of any type triggers the message, with the defaults when nothing is read.
        for data in [
            "null",
            "1",
            "\"x\"",
            "[]",
            "{}",
            r#"[{"subjectCode":"8.1"}]"#,
            r#"{"subjectCode":5}"#,
        ] {
            let body = format!(
                r#"{{"header":{{"functionExecutionStatus":{{"statusCodeData":{data}}}}}}}"#
            );
            assert_eq!(
                ia_status(body.as_bytes()),
                status(
                    "0.0.0",
                    "0.0.0",
                    "unknown",
                    "subject-code: 0.0.0, reason-code: 0.0.0"
                ),
                "{data}"
            );
        }
        // Only subjectCode given: reasonCode stays 0.0.0.
        let body =
            r#"{"header":{"functionExecutionStatus":{"statusCodeData":{"subjectcode":"8.8"}}}}"#;
        assert_eq!(
            ia_status(body.as_bytes()).message,
            "subject-code: 8.8, reason-code: 0.0.0"
        );
    }

    #[test]
    fn status_fields_are_cut_to_the_c_buffer_sizes() {
        let long = "x".repeat(300);
        let body = format!(
            r#"{{"header":{{"functionExecutionStatus":{{"statusCodeData":{{"subjectCode":"8.2.1.999","reasonCode":"3.3.3.3.3","subjectIdentifier":"{long}","message":"{long}"}}}}}}}}"#
        );
        let e = ia_status(body.as_bytes());
        assert_eq!(e.subject_code, "8.2.1.99");
        assert_eq!(e.reason_code, "3.3.3.3.");
        assert_eq!(e.subject_identifier.len(), 128);
        assert_eq!(e.message.len(), 128);
        // The cut codes are what the table and the fallback text see.
        let body = r#"{"header":{"functionExecutionStatus":{"statusCodeData":{"subjectCode":"8.2.1\u00009","reasonCode":"3.3456789"}}}}"#;
        assert_eq!(
            ia_status(body.as_bytes()).message,
            "subject-code: 8.2.1, reason-code: 3.345678"
        );
        let body = r#"{"header":{"functionExecutionStatus":{"statusCodeData":{"subjectCode":"8.2.1\u0000xx","reasonCode":"3.3"}}}}"#;
        assert_eq!(
            ia_status(body.as_bytes()).message,
            "Indicates that the Profile identified by the provided ICCID is not available"
        );
        // Cutting inside a multi-byte character leaves a replacement character.
        let msg = format!("{}é", "a".repeat(127));
        let body = format!(
            r#"{{"header":{{"functionExecutionStatus":{{"statusCodeData":{{"message":"{msg}"}}}}}}}}"#
        );
        assert_eq!(
            ia_status(body.as_bytes()).message,
            format!("{}\u{FFFD}", "a".repeat(127))
        );
    }

    #[test]
    fn status_from_a_failure_with_partial_members() {
        let body = r#"{"header":{"functionExecutionStatus":{"status":"Failed","statusCodeData":{"subjectCode":"8.10.1","reasonCode":"3.9","subjectIdentifier":"TransactionId"}}},"transactionId":"AB"}"#;
        let e = ia_status(body.as_bytes());
        assert_eq!(
            e,
            status(
                "8.10.1",
                "3.9",
                "TransactionId",
                "The RSP session identified by the TransactionID is unknown"
            )
        );
    }

    // ---- JSON reading ----

    #[test]
    fn lenient_json_like_cjson() {
        let ok = |body: &[u8]| bpp(body).unwrap();
        let v = r#""header":{"functionExecutionStatus":{}},"boundProfilePackage":"QQ==""#;
        assert_eq!(ok(format!("\u{FEFF}{{{v}}}").as_bytes()), "QQ==", "BOM");
        assert_eq!(
            ok(format!("{{{v}}} trailing <garbage>").as_bytes()),
            "QQ==",
            "trailing bytes"
        );
        assert_eq!(
            ok(format!("{{{v}}}\0\u{1}garbage").as_bytes()),
            "QQ==",
            "NUL ends the input"
        );
        let ws = format!(
            "\u{1}{{\u{1f}{}\u{b}}}\u{7}",
            v.replace(':', "\u{c}:\u{2} ").replace(',', "\u{3},\u{14}")
        );
        assert_eq!(
            ok(ws.as_bytes()),
            "QQ==",
            "all control bytes are whitespace"
        );
        assert_eq!(
            ok(format!(r#"{{"n":[1,-2.5e+3,0.0,01,1.,-.5,1E5,null,true,false,[],{{}},[[{{"a":[]}}]]],{v}}}"#)
                .as_bytes()),
            "QQ==",
            "strtod numbers"
        );
        // Invalid UTF-8 and raw control characters in strings are taken as they are.
        let mut body =
            br#"{"header":{"functionExecutionStatus":{}},"boundProfilePackage":"Q"#.to_vec();
        body.extend_from_slice(b"\xff\x01\x7fQ\"}");
        assert_eq!(ok(&body), "Q\u{FFFD}\u{1}\u{7f}Q");
    }

    #[test]
    fn numbers_are_cut_where_strtod_stops() {
        let v = r#""header":{"functionExecutionStatus":{}},"boundProfilePackage":"QQ==""#;
        for good in [
            "0",
            "-0",
            "123",
            "1.5",
            "1.",
            "-.5",
            "1e5",
            "1E+5",
            "1e-5",
            "007",
            "1.e3",
            "99999999999999999999999999999999e999",
        ] {
            let body = format!(r#"{{"n":{good},{v}}}"#);
            assert!(bpp(body.as_bytes()).is_ok(), "{good}");
        }
        for bad in [
            "-", "+1", ".5", "1e", "1e+", "1-2", "1..2", "--1", "1.2.3", "0x10", "1ee5", "-e",
            "Infinity", "NaN",
        ] {
            let body = format!(r#"{{"n":{bad},{v}}}"#);
            assert_eq!(
                bpp(body.as_bytes()).unwrap_err().message,
                "Not JSON",
                "{bad}"
            );
        }
        assert_eq!(strtod_len(b"1e"), 1);
        assert_eq!(strtod_len(b"1e+"), 1);
        assert_eq!(strtod_len(b"-.e5"), 0);
        assert_eq!(strtod_len(b"12.34e-5-"), 8);
        assert_eq!(strtod_len(b"."), 0);
        assert_eq!(strtod_len(b""), 0);
    }

    /// lpac links CoelacanthusHex/cJSON 4818f04, whose parse_number copies the whole run; upstream
    /// 1.7.18 copies 63 bytes and rejects every case below from 64 digits on. Each expectation was
    /// checked against pristine es9p.c linked with the fork.
    #[test]
    fn numbers_of_any_length_like_lpacs_cjson_fork() {
        let v = r#""header":{"functionExecutionStatus":{}},"boundProfilePackage":"QQ==""#;
        let n = |c: &str, k: usize| c.repeat(k);
        for good in [
            n("1", 63),
            n("1", 64),
            n("1", 65),
            n("1", 1000),
            n("9", 100_000),
            format!("-{}", n("1", 80)),
            format!("{}.{}", n("1", 40), n("2", 40)),
            format!("{}e+{}", n("1", 70), n("3", 30)),
            n("0", 200),
            format!("-{}.", n("0", 100)),
            format!("{}e{}", n("1", 64), n("9", 64)),
            format!("[{}]", vec![n("7", 90); 5].join(",")),
        ] {
            let body = format!(r#"{{"n":{good},{v}}}"#);
            assert_eq!(bpp(body.as_bytes()), Ok("QQ==".into()), "{}", good.len());
        }
        // strtod stops early, or the value does not start a number: what is left is not JSON,
        // however long the run.
        for bad in [
            format!(".{}", n("1", 70)),
            format!("{}-", n("1", 70)),
            format!("{}e", n("1", 70)),
            format!("{}..", n("1", 70)),
        ] {
            let body = format!(r#"{{"n":{bad},{v}}}"#);
            assert_eq!(
                bpp(body.as_bytes()).unwrap_err().message,
                "Not JSON",
                "{bad}"
            );
        }
        assert_eq!(strtod_len(&[b'1'; 100_000]), 100_000);
        assert_eq!(strtod_len(format!("{}-", n("1", 70)).as_bytes()), 70);
    }

    /// The header must keep cJSON's MIT notice (hex4, utf16_literal and the string escapes follow
    /// cJSON closely) and must name the cJSON lpac really links, not plain upstream 1.7.18.
    #[test]
    fn header_keeps_the_cjson_notice_and_names_lpacs_fork() {
        let header: Vec<&str> = include_str!("es9p.rs")
            .lines()
            .take_while(|l| l.starts_with("//") && !l.starts_with("//!"))
            .map(|l| l.trim_start_matches('/'))
            .collect();
        let header = header
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            header.starts_with("SPDX-License-Identifier: LGPL-2.1-only "),
            "file license stays LGPL"
        );
        let mit = "Copyright (c) 2009-2017 Dave Gamble and cJSON contributors \
            Permission is hereby granted, free of charge, to any person obtaining a copy of this \
            software and associated documentation files (the \"Software\"), to deal in the Software \
            without restriction, including without limitation the rights to use, copy, modify, \
            merge, publish, distribute, sublicense, and/or sell copies of the Software, and to \
            permit persons to whom the Software is furnished to do so, subject to the following \
            conditions: The above copyright notice and this permission notice shall be included in \
            all copies or substantial portions of the Software. THE SOFTWARE IS PROVIDED \"AS IS\", \
            WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE \
            WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN \
            NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR \
            OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT \
            OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.";
        assert!(
            header.contains(mit),
            "cJSON MIT notice missing from the header"
        );
        let mentions: Vec<_> = header
            .match_indices("cJSON 1.7.18")
            .map(|(i, _)| &header[i..])
            .collect();
        assert!(!mentions.is_empty());
        for m in mentions {
            assert!(
                m.starts_with("cJSON 1.7.18 as pinned by lpac (CoelacanthusHex/cJSON 4818f04, numbers of any length)"),
                "{m}"
            );
        }
    }

    #[test]
    fn string_escapes_like_cjson() {
        let read = |raw: &str| -> Option<Vec<u8>> {
            match parse(format!(r#"{{"k":"{raw}"}}"#).as_bytes())? {
                Json::Obj(m) => match m.into_iter().next()?.1 {
                    Json::Str(s) => Some(s),
                    _ => None,
                },
                _ => None,
            }
        };
        assert_eq!(
            read(r#"a\"b\\c\/d\b\f\n\r\t"#).unwrap(),
            b"a\"b\\c/d\x08\x0c\n\r\t"
        );
        assert_eq!(read(r"\u0041\u00e9\u20AC").unwrap(), "Aé€".as_bytes());
        assert_eq!(read(r"\ud83d\ude00").unwrap(), "\u{1F600}".as_bytes());
        assert_eq!(read(r"\uDBFF\uDFFF").unwrap(), "\u{10FFFF}".as_bytes());
        assert_eq!(read(r"x\u0000y").unwrap(), b"x\0y");
        assert_eq!(read(r"\uZZZZ").unwrap(), b"\0", "bad hex gives 0");
        assert_eq!(read(r"\u12G4").unwrap(), b"\0");
        assert_eq!(
            read(r#"\u00\"0"#).unwrap(),
            b"\x000",
            "\\u swallows the escaped quote"
        );
        assert_eq!(
            read(r"\u123\\").unwrap(),
            b"\0\"",
            "then a lone backslash escapes the closing quote"
        );
        for bad in [
            r"\x",
            r"\a",
            r"\u12",
            r"\u",
            r"\ud800",
            r"\ud800x",
            r"\ud800A",
            r"\udc00",
            r"\ud800\ud800",
            r"\U0041",
            "\\",
        ] {
            assert_eq!(read(bad), None, "{bad}");
        }
        let body = br#"{"header":{"functionExecutionStatus":{}},"boundProfilePackage":"QQ\u0000==","x":1}"#;
        assert_eq!(bpp(body).unwrap(), "QQ", "values end at NUL");
    }

    #[test]
    fn nesting_limit_matches_cjson() {
        let nest = |n: usize| {
            format!(
                r#"{{"header":{{"functionExecutionStatus":{{}}}},"boundProfilePackage":"QQ==","deep":{}{}}}"#,
                "[".repeat(n),
                "]".repeat(n)
            )
        };
        assert!(
            bpp(nest(999).as_bytes()).is_ok(),
            "root + 999 arrays = 1000 containers"
        );
        assert_eq!(bpp(nest(1000).as_bytes()).unwrap_err().message, "Not JSON");
        // Very deep input neither overflows the stack nor builds a deep tree.
        assert_eq!(
            bpp(nest(1_000_000).as_bytes()).unwrap_err().message,
            "Not JSON"
        );
        let objects = format!(
            "{}{}",
            r#"{"a":"#.repeat(999),
            "1".to_owned() + &"}".repeat(999)
        );
        assert_eq!(
            bpp(objects.as_bytes()).unwrap_err().message,
            "Critical object missing"
        );
        let objects = format!(
            "{}{}",
            r#"{"a":"#.repeat(1001),
            "1".to_owned() + &"}".repeat(1001)
        );
        assert_eq!(bpp(objects.as_bytes()).unwrap_err().message, "Not JSON");
    }

    #[test]
    fn deep_objects_are_not_kept() {
        let v = parse(br#"{"a":{"b":{"c":{"d":{"e":"x"},"s":"y"}}}}"#).unwrap();
        let d = v
            .get("a")
            .and_then(|v| v.get("b"))
            .and_then(|v| v.get("c"))
            .unwrap();
        assert_eq!(d.get("s"), Some(&Json::Str(b"y".to_vec())));
        assert_eq!(d.get("d"), Some(&Json::Other));
    }

    #[test]
    fn statuscodedata_strings_are_read_at_depth_four() {
        let body = r#"{"header":{"functionExecutionStatus":{"statusCodeData":{"subjectCode":"8.1","reasonCode":"4.8","x":{"y":{"z":1}}}}}}"#;
        assert_eq!(
            ia_status(body.as_bytes()).message,
            "eUICC does not have sufficient space for this Profile"
        );
    }

    #[test]
    fn hostile_bytes_never_panic() {
        // A small xorshift generator: deterministic, no dependencies.
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let alphabet =
            b"{}[]:,\"\\u0123456789abcdefABCDEF-+.eEnulltruefalse \t\n\x00\x01\xff\xc3\xa9/header";
        let base = format!(
            r#"{{"header":{{"functionExecutionStatus":{{"statusCodeData":{{"subjectCode":"8.1","reasonCode":"4.8"}}}}}},{}}}"#,
            r#""transactionId":"T","serverSigned1":"A","serverSignature1":"B","euiccCiPKIdToBeUsed":"C","serverCertificate":"D""#
        );
        for round in 0..20_000 {
            let mut body = base.as_bytes().to_vec();
            let edits = (next() % 6) as usize;
            for _ in 0..=edits {
                let at = (next() as usize) % (body.len() + 1);
                let c = alphabet[(next() as usize) % alphabet.len()];
                match next() % 3 {
                    0 if at < body.len() => body[at] = c,
                    1 if at < body.len() => {
                        body.remove(at);
                    }
                    _ => body.insert(at, c),
                }
            }
            if round % 3 == 0 {
                body.truncate((next() as usize) % (body.len() + 1));
            }
            let _ = initiate_authentication(&mut Server::reply(200, &body), "a", "b", "c");
            let random: Vec<u8> = (0..(next() % 64))
                .map(|_| alphabet[(next() as usize) % alphabet.len()])
                .collect();
            let _ = parse(&random);
        }
    }
}
