// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2023-2025 ESTKME TECHNOLOGY LIMITED, Hong Kong
// SPDX-FileCopyrightText: 2026 Jesther Silvestre
//
// Rust translation of the profile download parts of lpac euicc/es10b.c and euicc/es10b.h
// (es10b_get_euicc_challenge_r, es10b_get_euicc_info_r, es10b_authenticate_server_r,
// es10b_prepare_download_r, es10b_load_bound_profile_package_r and _tx, es10b_cancel_session_r
// with the checks of es10b_cancel_session), of euicc_bppcommandid2str and euicc_errorreason2str
// from euicc/tostr.c and of euicc_hexutil_gsmbcd2bin
// from euicc/hexutil.c (lpac commit c7c8d4f).
// Changed 2026-10-19: rewritten in Rust as stateless methods returning typed values (the caller
// keeps the session state that libeuicc kept in euicc_ctx.http._internal); SHA-256 comes from the
// sha2 crate instead of euicc/sha256.c; a card ErrorResult from LoadBoundProfilePackage is
// returned as Ok with bpp_command_id and error_reason set instead of as a failure code; an IMEI
// of 6 digits or fewer (under 4 BCD bytes) gives a zero-filled TAC instead of uninitialised stack
// bytes; an empty ICCID in an installation result gives an empty string instead of a buffer
// overrun; a transaction id that does not fit CancelSessionRequest is refused instead of having
// its length truncated to 8 bits; a package missing a later part fails without reporting the
// previous segment's installation result; through crate::der, an element whose long-form length
// has more than 4 length bytes is malformed, where libeuicc accepts it (the exact length when the
// extra leading bytes are zero, the low 32 bits otherwise).

//! ES10b profile download: the card side of an SGP.22 download session.
//!
//! The operator's server (ES9+) and these calls pass base64 text back and forth, as in
//! libeuicc: [`Euicc::get_euicc_challenge`] and [`Euicc::get_euicc_info1`] feed
//! InitiateAuthentication; [`Euicc::authenticate_server`] takes its answer and feeds
//! AuthenticateClient; [`Euicc::prepare_download`] takes that answer and feeds
//! GetBoundProfilePackage; [`Euicc::load_bound_profile_package`] installs the package.
//! [`Euicc::cancel_session`] ends a session that will not complete.
//!
//! Base64 from the server is decoded the way libeuicc does it ([`crate::b64::decode`]): decoding stops
//! at the first character outside the standard alphabet (padding, whitespace, `-`, `_`, NUL...),
//! a lone trailing character is dropped and leftover bits are ignored, so decoding never fails.
//! Text arguments (matching ID, IMEI, confirmation code) are read like C strings: up to the first
//! NUL.
//!
//! DER is read with [`crate::der`], which treats a long-form length of more than 4 length bytes
//! as malformed. libeuicc accepts such a length (exactly when the extra leading bytes are zero,
//! keeping the low 32 bits otherwise), so it can get further through a BoundProfilePackage or
//! the server fields carrying one. SM-DP+ output is DER, which never uses that form.

use crate::der::{self, Node, Tlv};
use crate::rsp::{AuthenticateServerParam, CancelSessionReason, PrepareDownloadParam};
use crate::{hex, Apdu, Error, Euicc, Result};
use sha2::{Digest, Sha256};

/// `ES10B_BPP_COMMAND_ID_UNDEFINED`: libeuicc's value for an absent or unknown bppCommandId.
pub const BPP_COMMAND_ID_UNDEFINED: u8 = 0xFF;
/// `ES10B_ERROR_REASON_UNDEFINED`: libeuicc's value for an absent or unknown errorReason.
pub const ERROR_REASON_UNDEFINED: u8 = 0xFF;

/// libeuicc builds CancelSessionRequest in its fixed 255-byte APDU buffer
/// (`request_buffer.body`); longer requests fail to pack.
const REQUEST_BUFFER: usize = 255;

/// TAC that libeuicc sends in DeviceInfo when no IMEI is given.
const DEFAULT_TAC: [u8; 4] = [0x35, 0x29, 0x06, 0x11];

/// Largest IMEI libeuicc converts (`uint8_t imei[8]`).
const IMEI_BYTES: usize = 8;

/// Outcome of [`Euicc::load_bound_profile_package`]
/// (`struct es10b_load_bound_profile_package_result`).
///
/// libeuicc returns 0 or -1 and fills this struct; the mapping is:
///
/// * C returns 0: every segment was sent and the card's last answer was empty or a
///   ProfileInstallationResult with a SuccessResult. Here: `Ok` with `bpp_command_id` and
///   `error_reason` both `None` ([`LoadBppResult::is_success`]). `seq_number` and `iccid` come from
///   the last answer (0 and `None` when it was empty, as libeuicc resets them before each segment).
/// * C returns -1 because the card answered a segment with a ProfileInstallationResult whose
///   finalResult is an ErrorResult, or a CHOICE libeuicc does not know: libeuicc stops there and
///   the struct holds that result. Here: `Ok` with `bpp_command_id` and `error_reason` both `Some`
///   (0xFF where the card omitted the field or sent a value libeuicc does not name, like
///   libeuicc's UNDEFINED), plus `seq_number` and `iccid` of that result. No further segment is
///   sent.
/// * C returns -1 for anything else: undecodable package (no BoundProfilePackage,
///   InitialiseSecureChannelRequest, firstSequenceOf87, sequenceOf88 or sequenceOf86), transport
///   failure, error status word, or an answer that is not a ProfileInstallationResult with
///   ProfileInstallationResultData, NotificationMetadata and finalResult. Here: `Err`. (libeuicc's
///   struct then holds its reset values 0, NULL, 0xFF, 0xFF, or, when a later part of the package
///   is missing, whatever the previous segment's answer left there; an `Err` carries neither.) As
///   in libeuicc, a package that is missing a later part fails only after the parts before it
///   were sent to the card.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadBppResult {
    /// NotificationMetadata seqNumber of the installation result (its notification).
    pub seq_number: u64,
    /// ICCID from the NotificationMetadata, when the card included it.
    pub iccid: Option<String>,
    /// ErrorResult bppCommandId ([`bpp_command_id_str`]); `None` on success.
    pub bpp_command_id: Option<u8>,
    /// ErrorResult errorReason ([`error_reason_str`]); `None` on success.
    pub error_reason: Option<u8>,
}

impl LoadBppResult {
    /// True when libeuicc would have returned 0 (no ErrorResult from the card).
    pub fn is_success(&self) -> bool {
        self.bpp_command_id.is_none() && self.error_reason.is_none()
    }
}

/// `euicc_bppcommandid2str`.
pub fn bpp_command_id_str(id: u8) -> &'static str {
    match id {
        0 => "initialise_secure_channel",
        1 => "configure_isdp",
        2 => "store_metadata",
        3 => "store_metadata2",
        4 => "replace_session_keys",
        5 => "load_profile_elements",
        BPP_COMMAND_ID_UNDEFINED => "unknown",
        _ => "(no_str_available)",
    }
}

/// `euicc_errorreason2str`.
pub fn error_reason_str(reason: u8) -> &'static str {
    match reason {
        1 => "incorrect_input_values",
        2 => "invalid_signature",
        3 => "invalid_transaction_id",
        4 => "unsupported_crt_values",
        5 => "unsupported_remote_operation_type",
        6 => "unsupported_profile_class",
        7 => "scp03t_structure_error",
        8 => "scp03t_security_error",
        9 => "install_failed_due_to_iccid_already_exists_on_euicc",
        10 => "install_failed_due_to_insufficient_memory_for_profile",
        11 => "install_failed_due_to_interruption",
        12 => "install_failed_due_to_pe_processing_error",
        13 => "install_failed_due_to_data_mismatch",
        14 => "test_profile_install_failed_due_to_invalid_naa_key",
        15 => "ppr_not_allowed",
        127 => "install_failed_due_to_unknown_error",
        ERROR_REASON_UNDEFINED => "unknown",
        _ => "(no_str_available)",
    }
}

/// The text a C caller would see: everything before the first NUL.
fn c_str(text: &str) -> &str {
    text.find('\0').map_or(text, |nul| &text[..nul])
}

/// `euicc_hexutil_gsmbcd2bin(imei, 8, text, 0)`: digits to swapped-nibble BCD (an odd count is
/// padded with `f`), refusing non-hex characters and more than 8 bytes.
fn imei_bcd(imei: &str) -> Option<Vec<u8>> {
    let bytes = imei.len().div_ceil(2);
    if bytes > IMEI_BYTES {
        return None;
    }
    hex::iccid_to_bcd(imei, bytes)
}

/// First element in `buf` with `tag`, with its offset in `buf`.
fn find_with_offset(buf: &[u8], tag: u16) -> Option<(usize, Tlv<'_>)> {
    let mut offset = 0;
    for t in der::iter(buf) {
        if t.tag == tag {
            return Some((offset, t));
        }
        offset += t.raw.len();
    }
    None
}

/// Tag and length bytes of an element.
fn header<'a>(t: &Tlv<'a>) -> &'a [u8] {
    &t.raw[..t.raw.len() - t.value.len()]
}

/// AuthenticateServerRequest (BF38) and the transactionId inside serverSigned1.
fn authenticate_server_request(
    param: &AuthenticateServerParam,
    matching_id: Option<&str>,
    imei: Option<&str>,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let signed1 = crate::b64::decode(&param.b64_server_signed1);
    let signature1 = crate::b64::decode(&param.b64_server_signature1);
    let ci_pkid = crate::b64::decode(&param.b64_euicc_ci_pkid_to_be_used);
    let certificate = crate::b64::decode(&param.b64_server_certificate);

    let signed1 = der::find(&signed1, 0x30).ok_or(Error::Argument("no serverSigned1"))?;
    let transaction_id = der::find(signed1.value, 0x80)
        .ok_or(Error::Argument("no transactionId in serverSigned1"))?;
    let signature1 =
        der::find(&signature1, 0x5F37).ok_or(Error::Argument("no serverSignature1"))?;
    let ci_pkid = der::find(&ci_pkid, 0x04).ok_or(Error::Argument("no euiccCiPKIdToBeUsed"))?;
    let certificate =
        der::find(&certificate, 0x30).ok_or(Error::Argument("no serverCertificate"))?;

    // DeviceInfo: tac [0], deviceCapabilities [1] (empty), imei [2] OPTIONAL
    let device_info = match imei.map(c_str) {
        Some(imei) => {
            let bcd = imei_bcd(imei).ok_or(Error::Argument("IMEI"))?;
            // The TAC is the first 4 IMEI bytes. libeuicc reads them from an uninitialised buffer,
            // so for an IMEI of 6 digits or fewer (under 4 BCD bytes) the rest is undefined; here
            // it is zero. 7 digits fill 4 bytes (the last nibble is the `f` pad), as in libeuicc.
            let mut tac = [0u8; 4];
            let n = bcd.len().min(tac.len());
            tac[..n].copy_from_slice(&bcd[..n]);
            vec![
                Node::Prim(0x80, tac.to_vec()),
                Node::Prim(0xA1, Vec::new()),
                Node::Prim(0x82, bcd),
            ]
        }
        None => vec![
            Node::Prim(0x80, DEFAULT_TAC.to_vec()),
            Node::Prim(0xA1, Vec::new()),
        ],
    };

    // CtxParams1 (ctxParamsForCommonAuthentication [0]): matchingId [0] OPTIONAL, deviceInfo [1]
    let mut ctx_params = Vec::with_capacity(2);
    if let Some(matching_id) = matching_id.map(c_str) {
        ctx_params.push(Node::Prim(0x80, matching_id.as_bytes().to_vec()));
    }
    ctx_params.push(Node::Cons(0xA1, device_info));

    let request = Node::Cons(
        0xBF38,
        vec![
            Node::Prim(0x30, signed1.value.to_vec()),
            Node::Prim(0x5F37, signature1.value.to_vec()),
            Node::Prim(0x04, ci_pkid.value.to_vec()),
            Node::Prim(0x30, certificate.value.to_vec()),
            Node::Cons(0xA0, ctx_params),
        ],
    );
    Ok((request.encode(), transaction_id.value.to_vec()))
}

/// PrepareDownloadRequest (BF21).
fn prepare_download_request(
    param: &PrepareDownloadParam,
    confirmation_code: Option<&str>,
) -> Result<Vec<u8>> {
    let signed2 = crate::b64::decode(&param.b64_smdp_signed2);
    let signature2 = crate::b64::decode(&param.b64_smdp_signature2);
    let certificate = crate::b64::decode(&param.b64_smdp_certificate);

    let signed2 = der::find(&signed2, 0x30).ok_or(Error::Argument("no smdpSigned2"))?;
    let signature2 = der::find(&signature2, 0x5F37).ok_or(Error::Argument("no smdpSignature2"))?;
    let certificate = der::find(&certificate, 0x30).ok_or(Error::Argument("no smdpCertificate"))?;
    let transaction_id =
        der::find(signed2.value, 0x80).ok_or(Error::Argument("no transactionId in smdpSigned2"))?;
    let cc_required = der::find(signed2.value, 0x01)
        .ok_or(Error::Argument("no ccRequiredFlag in smdpSigned2"))?;

    let mut children = vec![
        Node::Prim(0x30, signed2.value.to_vec()),
        Node::Prim(0x5F37, signature2.value.to_vec()),
    ];
    // libeuicc reads the BOOLEAN as a big-endian integer (its low 64 bits) and tests for non-zero.
    if der::to_u64(cc_required.value) != 0 {
        let code = confirmation_code
            .map(c_str)
            .filter(|code| !code.is_empty())
            .ok_or(Error::Argument("confirmation code required"))?;
        // hashCc = SHA-256(SHA-256(confirmation code) | transactionId of smdpSigned2)
        let mut hash = Sha256::new();
        hash.update(Sha256::digest(code.as_bytes()));
        hash.update(transaction_id.value);
        children.push(Node::Prim(0x04, hash.finalize().to_vec()));
    }
    children.push(Node::Prim(0x30, certificate.value.to_vec()));
    Ok(Node::Cons(0xBF21, children).encode())
}

/// The ES10b.LoadBoundProfilePackage segments libeuicc sends for a BoundProfilePackage, in
/// order, each as its own STORE DATA sequence:
///
/// 1. from the BoundProfilePackage tag through the end of InitialiseSecureChannelRequest (with
///    whatever precedes it inside the package),
/// 2. firstSequenceOf87 (A0) whole,
/// 3. the tag and length of sequenceOf88 (A1), then each of its elements,
/// 4. secondSequenceOf87 (A2) whole, if present,
/// 5. the tag and length of sequenceOf86 (A3), then each of its elements.
///
/// Each part is looked up from the start of the package; element iteration stops at the first
/// malformed element. `Err` means nothing can be sent; an `Err` item inside the plan is where
/// libeuicc stops after sending the segments before it.
fn bpp_segments(bpp: &[u8]) -> Result<Vec<Result<&[u8]>>> {
    let package = der::find(bpp, 0xBF36).ok_or(Error::Argument("no BoundProfilePackage"))?;
    let parts = package.value;
    let (offset, init) = find_with_offset(parts, 0xBF23)
        .ok_or(Error::Argument("no InitialiseSecureChannelRequest"))?;
    let first_len = header(&package).len() + offset + init.raw.len();
    let mut plan = vec![package
        .raw
        .get(..first_len)
        .ok_or(Error::Argument("BoundProfilePackage"))];

    let Some(first87) = der::find(parts, 0xA0) else {
        plan.push(Err(Error::Argument("no firstSequenceOf87")));
        return Ok(plan);
    };
    plan.push(Ok(first87.raw));

    let Some(seq88) = der::find(parts, 0xA1) else {
        plan.push(Err(Error::Argument("no sequenceOf88")));
        return Ok(plan);
    };
    plan.push(Ok(header(&seq88)));
    plan.extend(der::iter(seq88.value).map(|t| Ok(t.raw)));

    if let Some(second87) = der::find(parts, 0xA2) {
        plan.push(Ok(second87.raw));
    }

    let Some(seq86) = der::find(parts, 0xA3) else {
        plan.push(Err(Error::Argument("no sequenceOf86")));
        return Ok(plan);
    };
    plan.push(Ok(header(&seq86)));
    plan.extend(der::iter(seq86.value).map(|t| Ok(t.raw)));
    Ok(plan)
}

fn known_bpp_command_id(value: u64) -> u8 {
    match value {
        0..=5 => value as u8,
        _ => BPP_COMMAND_ID_UNDEFINED,
    }
}

fn known_error_reason(value: u64) -> u8 {
    match value {
        1..=15 | 127 => value as u8,
        _ => ERROR_REASON_UNDEFINED,
    }
}

/// Parses the card's answer to one LoadBoundProfilePackage segment
/// (`es10b_load_bound_profile_package_tx`). An empty answer means "continue".
pub fn parse_load_bpp_response(resp: &[u8]) -> Result<LoadBppResult> {
    let mut result = LoadBppResult::default();
    if resp.is_empty() {
        return Ok(result);
    }
    let installation =
        der::find(resp, 0xBF37).ok_or(Error::Malformed("no ProfileInstallationResult"))?;
    let data = der::find(installation.value, 0xBF27)
        .ok_or(Error::Malformed("no ProfileInstallationResultData"))?;
    let metadata =
        der::find(data.value, 0xBF2F).ok_or(Error::Malformed("no NotificationMetadata"))?;
    let final_result = der::find(data.value, 0xA2).ok_or(Error::Malformed("no finalResult"))?;
    let outcome = der::parse(final_result.value).ok_or(Error::Malformed("empty finalResult"))?;

    if let Some(seq) = der::find(metadata.value, 0x80) {
        result.seq_number = der::to_u64(seq.value);
    }
    if let Some(iccid) = der::find(metadata.value, 0x5A) {
        result.iccid = Some(hex::bcd_to_iccid(iccid.value));
    }
    match outcome.tag {
        // SuccessResult
        0xA0 => {}
        // ErrorResult: the last bppCommandId / errorReason wins, as in libeuicc's loop
        0xA1 => {
            result.bpp_command_id = Some(BPP_COMMAND_ID_UNDEFINED);
            result.error_reason = Some(ERROR_REASON_UNDEFINED);
            for t in der::iter(outcome.value) {
                match t.tag {
                    0x80 => {
                        result.bpp_command_id = Some(known_bpp_command_id(der::to_u64(t.value)))
                    }
                    0x81 => result.error_reason = Some(known_error_reason(der::to_u64(t.value))),
                    _ => {}
                }
            }
        }
        _ => {
            result.bpp_command_id = Some(BPP_COMMAND_ID_UNDEFINED);
            result.error_reason = Some(ERROR_REASON_UNDEFINED);
        }
    }
    Ok(result)
}

impl<T: Apdu> Euicc<T> {
    /// GetEuiccChallenge (BF2E): the card's euiccChallenge, base64, for InitiateAuthentication.
    pub fn get_euicc_challenge(&mut self) -> Result<String> {
        let body = self.request(Node::Prim(0xBF2E, Vec::new()))?;
        let challenge = der::find(&body, 0x80).ok_or(Error::Malformed("no euiccChallenge"))?;
        Ok(crate::b64::encode(challenge.value))
    }

    /// GetEuiccInfo1 (BF20): the whole EUICCInfo1 element, base64, for InitiateAuthentication.
    pub fn get_euicc_info1(&mut self) -> Result<String> {
        let resp = self.command(&Node::Prim(0xBF20, Vec::new()).encode())?;
        let info = der::find(&resp, 0xBF20).ok_or(Error::Malformed("no EUICCInfo1"))?;
        Ok(crate::b64::encode(info.raw))
    }

    /// AuthenticateServer (BF38) with the server's InitiateAuthentication answer. `matching_id`
    /// comes from the activation code (`Some("")` sends an empty matchingId, as libeuicc does);
    /// `imei` is up to 16 hex digits (`None` sends libeuicc's default TAC 35290611 and no IMEI).
    ///
    /// Returns the transactionId from serverSigned1 (keep it for [`Euicc::cancel_session`]) and
    /// the card's whole answer in base64 for AuthenticateClient. As in libeuicc, the answer is
    /// not checked here (an empty answer gives an empty string): the server judges it.
    pub fn authenticate_server(
        &mut self,
        param: &AuthenticateServerParam,
        matching_id: Option<&str>,
        imei: Option<&str>,
    ) -> Result<(Vec<u8>, String)> {
        let (request, transaction_id) = authenticate_server_request(param, matching_id, imei)?;
        let resp = self.command(&request)?;
        Ok((transaction_id, crate::b64::encode(&resp)))
    }

    /// PrepareDownload (BF21) with the server's AuthenticateClient answer. When smdpSigned2 sets
    /// ccRequiredFlag a non-empty `confirmation_code` is required and hashCc is sent.
    ///
    /// `_transaction_id` is not used: like libeuicc, hashCc binds the transactionId inside
    /// smdpSigned2, which is the one the card checks against its session.
    /// `b64_profile_metadata` is not used either. Returns the card's whole answer in base64 for
    /// GetBoundProfilePackage, unchecked as in libeuicc (a card-side error goes to the server).
    pub fn prepare_download(
        &mut self,
        param: &PrepareDownloadParam,
        _transaction_id: &[u8],
        confirmation_code: Option<&str>,
    ) -> Result<String> {
        let request = prepare_download_request(param, confirmation_code)?;
        let resp = self.command(&request)?;
        Ok(crate::b64::encode(&resp))
    }

    /// LoadBoundProfilePackage: sends the server's BoundProfilePackage (base64) segment by
    /// segment and reads the ProfileInstallationResult. See [`LoadBppResult`] for how success, a
    /// card-reported installation error and other failures are told apart.
    pub fn load_bound_profile_package(&mut self, b64_bpp: &str) -> Result<LoadBppResult> {
        let bpp = crate::b64::decode(b64_bpp);
        let mut result = LoadBppResult::default();
        for segment in bpp_segments(&bpp)? {
            let resp = self.command(segment?)?;
            result = parse_load_bpp_response(&resp)?;
            if !result.is_success() {
                break;
            }
        }
        Ok(result)
    }

    /// CancelSession (BF41) for the session `transaction_id` (from
    /// [`Euicc::authenticate_server`]). Returns the whole CancelSessionResponse in base64 for ES9+
    /// CancelSession. An empty transaction id is refused (as `es10b_cancel_session` does), and so
    /// is one too long for libeuicc's 255-byte request buffer.
    pub fn cancel_session(
        &mut self,
        transaction_id: &[u8],
        reason: CancelSessionReason,
    ) -> Result<String> {
        if transaction_id.is_empty() {
            return Err(Error::Argument("empty transaction id"));
        }
        let request = Node::Cons(
            0xBF41,
            vec![
                Node::Prim(0x80, transaction_id.to_vec()),
                Node::Prim(0x81, der::from_u64(reason as u64)),
            ],
        )
        .encode();
        if request.len() > REQUEST_BUFFER {
            return Err(Error::Argument("transaction id too long"));
        }
        let resp = self.command(&request)?;
        let answer =
            der::find(&resp, 0xBF41).ok_or(Error::Malformed("no CancelSessionResponse"))?;
        Ok(crate::b64::encode(answer.raw))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Script;

    const SW_OK: [u8; 2] = [0x90, 0x00];

    fn b64(bin: &[u8]) -> String {
        crate::b64::encode(bin)
    }

    fn with_sw(bin: &[u8]) -> Vec<u8> {
        let mut v = bin.to_vec();
        v.extend(SW_OK);
        v
    }

    fn unhex(text: &str) -> Vec<u8> {
        hex::decode(&text.replace(' ', "")).unwrap()
    }

    /// STORE DATA blocks of one segment as libeuicc sends them on channel 1.
    fn store_data(segment: &[u8]) -> Vec<Vec<u8>> {
        let blocks: Vec<&[u8]> = segment.chunks(120).collect();
        blocks
            .iter()
            .enumerate()
            .map(|(i, block)| {
                let p1 = if i == blocks.len() - 1 { 0x91 } else { 0x11 };
                let mut apdu = vec![0x81, 0xE2, p1, i as u8, block.len() as u8];
                apdu.extend_from_slice(block);
                apdu
            })
            .collect()
    }

    fn run<R>(
        responses: &[&[u8]],
        f: impl FnOnce(&mut Euicc<&mut Script>) -> R,
    ) -> (R, Vec<Vec<u8>>) {
        let mut card = Script::new(1, responses);
        let r = {
            let mut e = Euicc::open(&mut card).unwrap();
            f(&mut e)
        };
        assert_eq!(card.closed, [1]);
        (r, card.sent)
    }

    // ---- base64 and helpers -----------------------------------------------------------------

    #[test]
    fn base64_decoding_is_as_tolerant_as_libeuicc() {
        assert_eq!(crate::b64::decode(""), b"");
        assert_eq!(crate::b64::decode("QUJD"), b"ABC");
        assert_eq!(crate::b64::decode("QUJDRA=="), b"ABCD");
        assert_eq!(crate::b64::decode("QUJDRA"), b"ABCD", "padding is optional");
        assert_eq!(crate::b64::decode("QUJDREU"), b"ABCDE");
        assert_eq!(
            crate::b64::decode("QUJDRA==QUJD"),
            b"ABCD",
            "stops at the first '='"
        );
        assert_eq!(
            crate::b64::decode("QUJD\nREVG"),
            b"ABC",
            "stops at whitespace"
        );
        assert_eq!(
            crate::b64::decode(" QUJD"),
            b"",
            "leading whitespace stops at once"
        );
        assert_eq!(crate::b64::decode("Q"), b"", "a lone character is dropped");
        assert_eq!(
            crate::b64::decode("QUJDR"),
            b"ABC",
            "a lone trailing character is dropped"
        );
        assert_eq!(
            crate::b64::decode("QR=="),
            b"A",
            "leftover bits are ignored"
        );
        assert_eq!(
            crate::b64::decode("_-8"),
            b"",
            "URL-safe characters are not in the alphabet"
        );
        assert_eq!(crate::b64::decode("+/8="), [0xFB, 0xFF]);
        assert_eq!(
            crate::b64::decode("QUJD\0REVG"),
            b"ABC",
            "a NUL ends the C string"
        );
        assert_eq!(crate::b64::decode("QUJDé"), b"ABC");
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(crate::b64::decode(&b64(&all)), all);
        assert_eq!(b64(&[]), "");
        assert_eq!(b64(b"A"), "QQ==");
        assert_eq!(b64(b"AB"), "QUI=");
    }

    #[test]
    fn imeis_become_swapped_nibble_bcd_of_at_most_8_bytes() {
        assert_eq!(
            imei_bcd("356938035643809").unwrap(),
            unhex("53 96 83 30 65 34 08 F9")
        );
        assert_eq!(
            imei_bcd("3569380356438091").unwrap(),
            unhex("53 96 83 30 65 34 08 19")
        );
        assert_eq!(
            imei_bcd("35693803564380912"),
            None,
            "17 digits do not fit 8 bytes"
        );
        assert_eq!(imei_bcd("ABCDEFab").unwrap(), unhex("BA DC FE ba"));
        assert_eq!(imei_bcd("").unwrap(), b"");
        assert_eq!(imei_bcd("1").unwrap(), [0xF1]);
        assert_eq!(imei_bcd("35693803564380x"), None);
        assert_eq!(imei_bcd("3569 380"), None);
        assert_eq!(imei_bcd("35é9"), None);
        assert_eq!(c_str("12\u{0}34"), "12");
        assert_eq!(c_str("1234"), "1234");
        assert_eq!(c_str("\u{0}"), "");
    }

    #[test]
    fn names_match_tostr() {
        let bpp: Vec<&str> = (0..=6)
            .chain([0xFE, 0xFF])
            .map(bpp_command_id_str)
            .collect();
        assert_eq!(
            bpp,
            [
                "initialise_secure_channel",
                "configure_isdp",
                "store_metadata",
                "store_metadata2",
                "replace_session_keys",
                "load_profile_elements",
                "(no_str_available)",
                "(no_str_available)",
                "unknown"
            ]
        );
        assert_eq!(error_reason_str(0), "(no_str_available)");
        assert_eq!(error_reason_str(1), "incorrect_input_values");
        assert_eq!(error_reason_str(8), "scp03t_security_error");
        assert_eq!(error_reason_str(13), "install_failed_due_to_data_mismatch");
        assert_eq!(error_reason_str(15), "ppr_not_allowed");
        assert_eq!(error_reason_str(16), "(no_str_available)");
        assert_eq!(error_reason_str(127), "install_failed_due_to_unknown_error");
        assert_eq!(error_reason_str(128), "(no_str_available)");
        assert_eq!(error_reason_str(0xFF), "unknown");
        for v in 0..=255u8 {
            assert!(!bpp_command_id_str(v).is_empty() && !error_reason_str(v).is_empty());
        }
    }

    // ---- challenge, info1 --------------------------------------------------------------------

    #[test]
    fn challenge_is_the_80_inside_bf2e() {
        let challenge: Vec<u8> = (1..=16).collect();
        let resp = with_sw(&Node::Cons(0xBF2E, vec![Node::Prim(0x80, challenge.clone())]).encode());
        let (r, sent) = run(&[&resp], |e| e.get_euicc_challenge());
        assert_eq!(r.unwrap(), b64(&challenge));
        assert_eq!(sent, [unhex("81 E2 91 00 03 BF 2E 00")]);

        let (r, _) = run(&[&unhex("BF2E 02 8000 9000")], |e| e.get_euicc_challenge());
        assert_eq!(r.unwrap(), "", "an empty challenge is passed on");
        for bad in [
            "9000",
            "BF2E 00 9000",
            "BF2E 03 810100 9000",
            "BF20 03 800100 9000",
            "BF2E 05 8001 9000",
        ] {
            let (r, _) = run(&[&unhex(bad)], |e| e.get_euicc_challenge());
            assert!(r.is_err(), "{bad}");
        }
        let (r, _) = run(&[&unhex("6A88")], |e| e.get_euicc_challenge());
        assert_eq!(r, Err(Error::Status(0x6A, 0x88)));
        let (r, _) = run(&[], |e| e.get_euicc_challenge());
        assert!(matches!(r, Err(Error::Transport(_))));
    }

    #[test]
    fn info1_is_the_whole_bf20_element() {
        let info1 = unhex("BF20 0B 82 03 020300 A9 04 04 02 F5 4A");
        let mut resp = unhex("5A 00"); // an element before it is skipped
        resp.extend(&info1);
        resp.extend(unhex("FF FF")); // trailing bytes are ignored
        let (r, sent) = run(&[&with_sw(&resp)], |e| e.get_euicc_info1());
        assert_eq!(r.unwrap(), b64(&info1));
        assert_eq!(sent, [unhex("81 E2 91 00 03 BF 20 00")]);
        let (r, _) = run(&[&unhex("BF20 00 9000")], |e| e.get_euicc_info1());
        assert_eq!(r.unwrap(), b64(&[0xBF, 0x20, 0x00]));
        for bad in ["9000", "BF2E 00 9000", "BF20 05 00 9000"] {
            let (r, _) = run(&[&unhex(bad)], |e| e.get_euicc_info1());
            assert!(r.is_err(), "{bad}");
        }
        // 61xx is followed with GET RESPONSE and the parts are joined
        let (r, sent) = run(&[&unhex("BF20 61 03"), &unhex("03 820102 9000")], |e| {
            e.get_euicc_info1()
        });
        assert_eq!(r.unwrap(), b64(&unhex("BF20 03 820102")));
        assert_eq!(sent[1], unhex("81 C0 00 00 03"));
    }

    // ---- authenticate server -----------------------------------------------------------------

    fn auth_param() -> AuthenticateServerParam {
        AuthenticateServerParam {
            b64_server_signed1: b64(&unhex("30 06 80 01 AA 81 01 BB")),
            b64_server_signature1: b64(&unhex("5F37 02 CCDD")),
            b64_euicc_ci_pkid_to_be_used: b64(&unhex("04 01 EE")),
            b64_server_certificate: b64(&unhex("30 01 FF")),
        }
    }

    #[test]
    fn authenticate_server_request_matches_libeuicc() {
        let answer = unhex("BF38 03 A0 01 00");
        let (r, sent) = run(&[&with_sw(&answer)], |e| {
            e.authenticate_server(&auth_param(), Some("AB"), None)
        });
        let (tid, resp) = r.unwrap();
        assert_eq!(tid, [0xAA]);
        assert_eq!(resp, b64(&answer));
        let request =
            unhex("BF38 23 3006800 1AA8101BB 5F3702CCDD 0401EE 3001FF A0 0E 80024142 A1 08 80043529 0611 A100");
        let mut apdu = unhex("81 E2 91 00 26");
        apdu.extend(&request);
        assert_eq!(sent, [apdu]);
    }

    #[test]
    fn authenticate_server_device_info_variants() {
        let request = |m: Option<&str>, i: Option<&str>| {
            let (r, sent) = run(&[&SW_OK], |e| e.authenticate_server(&auth_param(), m, i));
            r.map(|(tid, b)| (tid, b, sent.concat()))
        };
        // APDU header, BF38 tag and length, then serverSigned1, serverSignature1, CI PKId, certificate
        let ctx_params = |sent: Vec<u8>| sent[5 + 3 + 8 + 5 + 3 + 3..].to_vec();

        let (_, resp, sent) = request(None, None).unwrap();
        assert_eq!(resp, "", "an empty answer is passed on, as libeuicc does");
        assert_eq!(ctx_params(sent), unhex("A0 0A A1 08 80 04 35290611 A1 00"));

        let (_, _, sent) = request(Some(""), Some("356938035643809")).unwrap();
        assert_eq!(
            ctx_params(sent),
            unhex("A0 16 8000 A1 12 80 04 53968330 A1 00 82 08 5396833065 3408F9"),
            "TAC from the IMEI, then the IMEI"
        );
        let (_, _, sent) = request(None, Some("123")).unwrap();
        assert_eq!(
            ctx_params(sent),
            unhex("A0 0E A1 0C 80 04 21F30000 A100 82 02 21F3"),
            "short IMEI: zero TAC"
        );
        let (_, _, sent) = request(None, Some("")).unwrap();
        assert_eq!(
            ctx_params(sent),
            unhex("A0 0C A1 0A 80 04 00000000 A100 8200")
        );
        let (_, _, sent) = request(Some("A\u{0}B"), Some("12345678\u{0}x")).unwrap();
        assert_eq!(
            ctx_params(sent),
            unhex("A0 13 80 01 41 A1 0E 80 04 21436587 A100 82 04 21436587")
        );

        for bad in ["35693803564380912", "35693803564380x", "-1", " 1"] {
            let (r, sent) = run(&[&SW_OK], |e| {
                e.authenticate_server(&auth_param(), None, Some(bad))
            });
            assert_eq!(r, Err(Error::Argument("IMEI")), "{bad}");
            assert!(sent.is_empty(), "nothing is sent for a bad IMEI");
        }
    }

    #[test]
    fn tac_is_zero_filled_only_below_4_bcd_bytes() {
        let param = AuthenticateServerParam {
            b64_server_signed1: "MAOAAQE=".into(),
            b64_server_signature1: "XzcA".into(),
            b64_euicc_ci_pkid_to_be_used: "BAA=".into(),
            b64_server_certificate: "MAA=".into(),
        };
        let sent_for = |imei: &str| {
            let (r, sent) = run(&[&SW_OK], |e| {
                e.authenticate_server(&param, None, Some(imei))
            });
            assert_eq!(r.unwrap().0, [0x01]);
            sent.concat()
        };
        // 7 digits pad to 4 BCD bytes, so libeuicc fills the whole TAC from them: the same APDU
        // as libeuicc sends (captured from the C build)
        assert_eq!(
            sent_for("1234567"),
            unhex("81e2910021bf381e30038001015f370004003000a010a10e8004214365f7a1008204214365f7")
        );
        // the TAC is zero-filled (libeuicc: uninitialised) only for 0 to 6 digits
        let digits = "1234567890123456";
        for n in 0..=digits.len() {
            let bcd = imei_bcd(&digits[..n]).unwrap();
            let sent = sent_for(&digits[..n]);
            let tac = &sent[26..30];
            assert_eq!(
                &sent[22..26],
                &[0xA1, 10 + bcd.len() as u8, 0x80, 0x04],
                "{n} digits"
            );
            if n <= 6 {
                assert!(bcd.len() < 4, "{n} digits");
                let mut zero_filled = bcd.clone();
                zero_filled.resize(4, 0);
                assert_eq!(tac, zero_filled, "{n} digits: zero-filled");
            } else {
                assert!(bcd.len() >= 4, "{n} digits");
                assert_eq!(
                    tac,
                    &bcd[..4],
                    "{n} digits: the first 4 IMEI bytes, as in libeuicc"
                );
            }
        }
        assert_eq!(
            &sent_for("123456")[26..30],
            unhex("21436500"),
            "6 digits: the deviation"
        );
        assert_eq!(&sent_for("12345678")[26..30], unhex("21436587"));
    }

    #[test]
    fn authenticate_server_reencodes_found_elements_and_splits_long_requests() {
        // serverSigned1 after another element, with a long-form length that is not minimal
        let mut signed1 = unhex("04 00 30 81 05 80 03 01 02 03");
        signed1.extend(unhex("00 00"));
        let cert_value = vec![0x5A; 300];
        let cert = Node::Prim(0x30, cert_value.clone()).encode();
        let param = AuthenticateServerParam {
            b64_server_signed1: b64(&signed1) + "== trailing",
            b64_server_signature1: b64(&unhex("5F37 00")),
            b64_euicc_ci_pkid_to_be_used: b64(&unhex("02 00 04 00")),
            b64_server_certificate: b64(&cert),
        };
        let (r, sent) = run(&[&SW_OK, &SW_OK, &with_sw(&[0xAB])], |e| {
            e.authenticate_server(&param, Some("MATCHING-ID-1"), None)
        });
        let (tid, resp) = r.unwrap();
        assert_eq!(tid, [1, 2, 3]);
        assert_eq!(resp, "qw==");
        let expected = Node::Cons(
            0xBF38,
            vec![
                Node::Prim(0x30, unhex("80 03 01 02 03")),
                Node::Prim(0x5F37, vec![]),
                Node::Prim(0x04, vec![]),
                Node::Prim(0x30, cert_value),
                Node::Cons(
                    0xA0,
                    vec![
                        Node::Prim(0x80, b"MATCHING-ID-1".to_vec()),
                        Node::Cons(
                            0xA1,
                            vec![
                                Node::Prim(0x80, DEFAULT_TAC.to_vec()),
                                Node::Prim(0xA1, vec![]),
                            ],
                        ),
                    ],
                ),
            ],
        )
        .encode();
        assert_eq!(&expected[..4], &[0xBF, 0x38, 0x82, 0x01]);
        assert_eq!(sent, store_data(&expected));
        assert_eq!(sent.len(), 3, "348 bytes take three blocks");
    }

    #[test]
    fn authenticate_server_refuses_missing_parts_without_sending() {
        let mut cases = Vec::new();
        let mut p = auth_param();
        p.b64_server_signed1 = b64(&unhex("30 03 81 01 BB"));
        cases.push(p);
        let mut p = auth_param();
        p.b64_server_signed1 = b64(&unhex("31 03 80 01 BB"));
        cases.push(p);
        let mut p = auth_param();
        p.b64_server_signed1 = b64(&unhex("30 05 80 01 BB"));
        cases.push(p);
        let mut p = auth_param();
        p.b64_server_signature1 = b64(&unhex("5F 36 00"));
        cases.push(p);
        let mut p = auth_param();
        p.b64_euicc_ci_pkid_to_be_used = String::new();
        cases.push(p);
        let mut p = auth_param();
        p.b64_server_certificate = "-".into();
        cases.push(p);
        let mut p = auth_param();
        p.b64_server_certificate = " MAH/".into();
        cases.push(p);
        for p in cases {
            let (r, sent) = run(&[&SW_OK], |e| e.authenticate_server(&p, None, None));
            assert!(matches!(r, Err(Error::Argument(_))), "{p:?}");
            assert!(sent.is_empty());
        }
        // the transactionId may be empty: libeuicc keeps it as is
        let mut p = auth_param();
        p.b64_server_signed1 = b64(&unhex("30 02 80 00"));
        let (r, _) = run(&[&SW_OK], |e| e.authenticate_server(&p, None, None));
        assert_eq!(r.unwrap().0, b"");
        // card errors fail the call
        let (r, _) = run(&[&unhex("6985")], |e| {
            e.authenticate_server(&auth_param(), None, None)
        });
        assert_eq!(r, Err(Error::Status(0x69, 0x85)));
        let (r, _) = run(&[&[0x90]], |e| {
            e.authenticate_server(&auth_param(), None, None)
        });
        assert!(matches!(r, Err(Error::Malformed(_))));
    }

    // ---- prepare download --------------------------------------------------------------------

    fn prepare_param(cc_flag: &[u8]) -> PrepareDownloadParam {
        let tid: Vec<u8> = (1..=16).collect();
        let signed2 = Node::Cons(
            0x30,
            vec![Node::Prim(0x80, tid), Node::Prim(0x01, cc_flag.to_vec())],
        )
        .encode();
        PrepareDownloadParam {
            b64_profile_metadata: "not used".into(),
            b64_smdp_signed2: b64(&signed2),
            b64_smdp_signature2: b64(&unhex("5F37 02 1122")),
            b64_smdp_certificate: b64(&unhex("30 01 33")),
        }
    }

    #[test]
    fn prepare_download_without_confirmation_code() {
        let answer = unhex("BF21 03 A0 01 00");
        let (r, sent) = run(&[&with_sw(&answer)], |e| {
            e.prepare_download(&prepare_param(&[0x00]), &[9], Some("1234"))
        });
        assert_eq!(r.unwrap(), b64(&answer));
        let mut apdu = unhex("81 E2 91 00 22 BF21 1F 30 15 80 10");
        apdu.extend(1..=16u8);
        apdu.extend(unhex("01 01 00 5F37 02 1122 30 01 33"));
        assert_eq!(sent, [apdu]);
        // an empty BOOLEAN reads as false, a 9-byte one keeps only its low 64 bits (zero)
        for flag in [&[][..], &[1, 0, 0, 0, 0, 0, 0, 0, 0][..]] {
            let (r, sent) = run(&[&SW_OK], |e| {
                e.prepare_download(&prepare_param(flag), &[], None)
            });
            assert_eq!(r.unwrap(), "");
            assert!(!sent[0].windows(2).any(|w| w == [0x04, 0x20]), "no hashCc");
        }
    }

    #[test]
    fn prepare_download_with_confirmation_code_sends_hash_cc() {
        // SHA-256(SHA-256("1234") | 01..10), computed independently
        let hash_cc = unhex("94e57b8cdfe3eb0a8510f18898add0b1e67b48fafd39892796d6f65f9ce3cc40");
        for flag in [&[0xFF][..], &[0x01][..], &[0x00, 0x01][..]] {
            let (r, sent) = run(&[&SW_OK], |e| {
                e.prepare_download(&prepare_param(flag), &[], Some("1234\u{0}ignored"))
            });
            assert_eq!(r.unwrap(), "");
            let request = sent.concat();
            let mut tail = unhex("5F37 02 1122 04 20");
            tail.extend(&hash_cc);
            tail.extend(unhex("30 01 33"));
            assert!(request.ends_with(&tail), "{}", hex::encode(&request));
        }
        for code in [None, Some(""), Some("\u{0}1234")] {
            let (r, sent) = run(&[&SW_OK], |e| {
                e.prepare_download(&prepare_param(&[0xFF]), &[], code)
            });
            assert_eq!(
                r,
                Err(Error::Argument("confirmation code required")),
                "{code:?}"
            );
            assert!(sent.is_empty());
        }
    }

    #[test]
    fn prepare_download_hash_uses_the_signed2_transaction_id() {
        let signed2 = unhex("30 05 01 01 FF 80 00");
        let param = PrepareDownloadParam {
            b64_smdp_signed2: b64(&signed2),
            b64_smdp_signature2: b64(&unhex("5F37 00")),
            b64_smdp_certificate: b64(&unhex("30 00")),
            ..Default::default()
        };
        let (r, sent) = run(&[&SW_OK], |e| {
            e.prepare_download(&param, &[1, 2, 3], Some("x"))
        });
        assert!(r.is_ok());
        let mut expected = unhex("BF21 2E 30 05 01 01 FF 80 00 5F37 00 04 20");
        expected.extend(unhex(
            "0a325ca303eb3014c43ae004970f343634db176fa1697bcc8c9efac94626488d",
        ));
        expected.extend(unhex("30 00"));
        assert_eq!(sent, store_data(&expected));
    }

    #[test]
    fn prepare_download_refuses_missing_parts() {
        let mut cases = Vec::new();
        let mut p = prepare_param(&[0]);
        p.b64_smdp_signed2 = b64(&unhex("30 03 01 01 00"));
        cases.push(p);
        let mut p = prepare_param(&[0]);
        p.b64_smdp_signed2 = b64(&unhex("30 02 80 00"));
        cases.push(p);
        let mut p = prepare_param(&[0]);
        p.b64_smdp_signature2 = b64(&unhex("5F38 00"));
        cases.push(p);
        let mut p = prepare_param(&[0]);
        p.b64_smdp_certificate = b64(&unhex("30 02 00"));
        cases.push(p);
        let mut p = prepare_param(&[0]);
        p.b64_smdp_signed2 = String::new();
        cases.push(p);
        for p in cases {
            let (r, sent) = run(&[&SW_OK], |e| e.prepare_download(&p, &[], Some("1")));
            assert!(matches!(r, Err(Error::Argument(_))), "{p:?}");
            assert!(sent.is_empty());
        }
    }

    // ---- load bound profile package ----------------------------------------------------------

    fn installation_result(seq: u8, iccid: Option<&[u8]>, final_result: Node) -> Vec<u8> {
        let mut metadata = vec![
            Node::Prim(0x80, vec![seq]),
            Node::Prim(0x81, vec![0x07, 0x80]),
        ];
        metadata.push(Node::Prim(0x0C, b"smdp.example.com".to_vec()));
        if let Some(iccid) = iccid {
            metadata.push(Node::Prim(0x5A, iccid.to_vec()));
        }
        Node::Cons(
            0xBF37,
            vec![
                Node::Cons(
                    0xBF27,
                    vec![
                        Node::Prim(0x80, vec![0xAA; 16]),
                        Node::Cons(0xBF2F, metadata),
                        Node::Prim(0x06, vec![0x2B, 0x06]),
                        Node::Cons(0xA2, vec![final_result]),
                    ],
                ),
                Node::Prim(0x5F37, vec![0x55; 64]),
            ],
        )
        .encode()
    }

    fn success() -> Node {
        Node::Cons(
            0xA0,
            vec![Node::Prim(0x4F, vec![0xA0, 0x00, 0x00, 0x05, 0x59])],
        )
    }

    fn error_result(children: Vec<Node>) -> Node {
        Node::Cons(0xA1, children)
    }

    const ICCID: [u8; 10] = [0x98, 0x36, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF1];

    struct Bpp {
        init: Node,
        first87: Option<Node>,
        seq88: Option<Node>,
        second87: Option<Node>,
        seq86: Option<Node>,
    }

    impl Bpp {
        fn standard() -> Self {
            Bpp {
                init: Node::Cons(0xBF23, vec![Node::Prim(0x82, vec![0x01])]),
                first87: Some(Node::Cons(0xA0, vec![Node::Prim(0x87, vec![0x01, 0x02])])),
                seq88: Some(Node::Cons(
                    0xA1,
                    vec![
                        Node::Prim(0x88, vec![0xAA, 0xBB, 0xCC]),
                        Node::Prim(0x88, vec![0xDD]),
                    ],
                )),
                second87: Some(Node::Cons(0xA2, vec![Node::Prim(0x87, vec![0xEE])])),
                seq86: Some(Node::Cons(
                    0xA3,
                    vec![
                        Node::Prim(0x86, (0..300).map(|i| i as u8).collect()),
                        Node::Prim(0x86, vec![0xFF]),
                    ],
                )),
            }
        }

        fn encode(&self) -> Vec<u8> {
            let mut children = vec![self.init.clone()];
            children.extend(
                [&self.first87, &self.seq88, &self.second87, &self.seq86]
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
            Node::Cons(0xBF36, children).encode()
        }
    }

    #[test]
    fn bpp_is_sent_in_libeuicc_segments() {
        let bpp = Bpp::standard().encode();
        let segments = bpp_segments(&bpp).unwrap();
        let segments: Vec<Vec<u8>> = segments.into_iter().map(|s| s.unwrap().to_vec()).collect();
        let expected: Vec<Vec<u8>> = vec![
            unhex("BF36 82 0152 BF23 03 820101"),
            unhex("A0 04 8702 0102"),
            unhex("A1 08"),
            unhex("88 03 AABBCC"),
            unhex("88 01 DD"),
            unhex("A2 03 8701EE"),
            unhex("A3 82 0133"),
            Node::Prim(0x86, (0..300).map(|i| i as u8).collect()).encode(),
            unhex("86 01 FF"),
        ];
        assert_eq!(segments, expected);
        assert_eq!(bpp.len(), 5 + 0x152);

        let done = with_sw(&installation_result(5, Some(&ICCID), success()));
        let mut responses: Vec<&[u8]> = vec![&SW_OK; 10];
        responses.push(&done);
        let (r, sent) = run(&responses, |e| e.load_bound_profile_package(&b64(&bpp)));
        let r = r.unwrap();
        assert!(r.is_success());
        assert_eq!(
            r,
            LoadBppResult {
                seq_number: 5,
                iccid: Some("8963000000000000001".into()),
                bpp_command_id: None,
                error_reason: None
            }
        );
        let apdus: Vec<Vec<u8>> = expected.iter().flat_map(|s| store_data(s)).collect();
        assert_eq!(sent, apdus);
        assert_eq!(
            sent.len(),
            11,
            "the 304-byte element takes three blocks numbered from 0"
        );
        assert_eq!(&sent[7][..5], &[0x81, 0xE2, 0x11, 0x00, 120]);
        assert_eq!(&sent[9][..5], &[0x81, 0xE2, 0x91, 0x02, 64]);
    }

    #[test]
    fn bpp_optional_and_unusual_layouts() {
        // no secondSequenceOf87; empty sequences; something before BF23 travels with it
        let mut b = Bpp::standard();
        b.second87 = None;
        b.seq88 = Some(Node::Cons(0xA1, vec![]));
        b.seq86 = Some(Node::Cons(0xA3, vec![]));
        let mut bpp = Node::Cons(
            0xBF36,
            vec![
                Node::Prim(0x99, vec![1]),
                b.init.clone(),
                b.first87.clone().unwrap(),
                b.seq88.clone().unwrap(),
                b.seq86.clone().unwrap(),
            ],
        )
        .encode();
        bpp.extend(unhex("DEAD")); // after the package: ignored
        let segments: Vec<Vec<u8>> = bpp_segments(&bpp)
            .unwrap()
            .into_iter()
            .map(|s| s.unwrap().to_vec())
            .collect();
        assert_eq!(
            segments,
            [
                unhex("BF36 13 990101 BF23 03 820101"),
                unhex("A0 04 87020102"),
                unhex("A1 00"),
                unhex("A3 00")
            ]
        );

        // the package after other top-level elements; trailing junk inside A3 ends its elements
        let mut b = Bpp::standard();
        b.seq86 = Some(Node::Prim(0xA3, unhex("86 01 01 86 05 01")));
        let mut bpp = unhex("30 00");
        bpp.extend(b.encode());
        let segments: Vec<Vec<u8>> = bpp_segments(&bpp)
            .unwrap()
            .into_iter()
            .map(|s| s.unwrap().to_vec())
            .collect();
        assert_eq!(segments.last().unwrap(), &unhex("86 01 01"));
        assert_eq!(segments[segments.len() - 2], unhex("A3 06"));
    }

    #[test]
    fn bpp_missing_parts_fail_where_libeuicc_stops() {
        // nothing sent without BF36 or BF23
        for bpp in [
            vec![],
            unhex("BF37 00"),
            unhex("BF36 03 A0 01 00"),
            unhex("BF36 05 BF23 03 82"),
        ] {
            let (r, sent) = run(&[&SW_OK], |e| e.load_bound_profile_package(&b64(&bpp)));
            assert!(
                matches!(r, Err(Error::Argument(_))),
                "{}",
                hex::encode(&bpp)
            );
            assert!(sent.is_empty());
        }
        let (r, sent) = run(&[], |e| e.load_bound_profile_package("not base64!"));
        assert!(r.is_err() && sent.is_empty());

        // a missing mandatory part fails after the parts before it went out
        type Strip = fn(&mut Bpp);
        let cases: [(Strip, usize, &str); 3] = [
            (|b| b.first87 = None, 1, "no firstSequenceOf87"),
            (|b| b.seq88 = None, 2, "no sequenceOf88"),
            (|b| b.seq86 = None, 6, "no sequenceOf86"),
        ];
        for (strip, sent_count, what) in cases {
            let mut b = Bpp::standard();
            strip(&mut b);
            let responses: Vec<&[u8]> = vec![&SW_OK; 12];
            let (r, sent) = run(&responses, |e| {
                e.load_bound_profile_package(&b64(&b.encode()))
            });
            assert_eq!(r, Err(Error::Argument(what)));
            assert_eq!(sent.len(), sent_count, "{what}");
        }
    }

    #[test]
    fn bpp_error_result_stops_and_reports() {
        let bpp = Bpp::standard().encode();
        let failed = with_sw(&installation_result(
            9,
            Some(&ICCID),
            error_result(vec![
                Node::Prim(0x80, vec![0x01]),
                Node::Prim(0x81, vec![0x08]),
            ]),
        ));
        let (r, sent) = run(&[&SW_OK, &failed, &SW_OK], |e| {
            e.load_bound_profile_package(&b64(&bpp))
        });
        let r = r.unwrap();
        assert!(!r.is_success());
        assert_eq!(
            r,
            LoadBppResult {
                seq_number: 9,
                iccid: Some("8963000000000000001".into()),
                bpp_command_id: Some(1),
                error_reason: Some(8)
            }
        );
        assert_eq!(sent.len(), 2, "nothing more is sent after an ErrorResult");
        assert_eq!(
            (bpp_command_id_str(1), error_reason_str(8)),
            ("configure_isdp", "scp03t_security_error")
        );
    }

    #[test]
    fn installation_results_parse_like_libeuicc() {
        let parse = |final_result: Node| {
            parse_load_bpp_response(&installation_result(3, None, final_result)).unwrap()
        };
        let undefined = (Some(BPP_COMMAND_ID_UNDEFINED), Some(ERROR_REASON_UNDEFINED));
        // ErrorResult without fields, with unknown values, with repeated fields
        let r = parse(error_result(vec![]));
        assert_eq!((r.bpp_command_id, r.error_reason), undefined);
        assert_eq!((r.seq_number, r.iccid), (3, None));
        let r = parse(error_result(vec![
            Node::Prim(0x80, vec![6]),
            Node::Prim(0x81, vec![16]),
        ]));
        assert_eq!((r.bpp_command_id, r.error_reason), undefined);
        let r = parse(error_result(vec![
            Node::Prim(0x81, vec![0]),
            Node::Prim(0x80, vec![0xFF]),
        ]));
        assert_eq!((r.bpp_command_id, r.error_reason), undefined);
        let r = parse(error_result(vec![
            Node::Prim(0x80, vec![2]),
            Node::Prim(0x81, vec![0x7F]),
            Node::Prim(0x80, vec![0, 0, 5]),
            Node::Prim(0x82, vec![1]),
        ]));
        assert_eq!((r.bpp_command_id, r.error_reason), (Some(5), Some(127)));
        let r = parse(error_result(vec![
            Node::Prim(0x80, vec![]),
            Node::Prim(0x81, vec![0x01, 0, 0, 0, 0, 0, 0, 0, 13]),
        ]));
        assert_eq!(
            (r.bpp_command_id, r.error_reason),
            (Some(0), Some(13)),
            "low 64 bits, as C's long"
        );
        // unknown CHOICE: a failure with undefined fields
        let r = parse(Node::Prim(0x80, vec![]));
        assert_eq!((r.bpp_command_id, r.error_reason), undefined);
        // success
        let r = parse(Node::Prim(0xA0, vec![]));
        assert!(r.is_success());

        // empty answer: carry on with reset values
        assert_eq!(
            parse_load_bpp_response(&[]).unwrap(),
            LoadBppResult::default()
        );

        // ICCIDs: trailing f trimmed, empty, odd
        let iccid = |v: &[u8]| {
            parse_load_bpp_response(&installation_result(1, Some(v), success()))
                .unwrap()
                .iccid
        };
        assert_eq!(iccid(&ICCID).as_deref(), Some("8963000000000000001"));
        assert_eq!(
            iccid(&[]).as_deref(),
            Some(""),
            "libeuicc overruns its buffer here"
        );
        assert_eq!(iccid(&[0xFF, 0xFF]).as_deref(), Some("f"));
        assert_eq!(iccid(&[0x21, 0x43]).as_deref(), Some("1234"));

        // malformed answers
        for bad in [
            "00",
            "BF37 00",
            "BF37 03 BF27 00",
            "BF37 05 BF27 02 A200",
            "BF37 07 BF27 05 BF2F00 A200",
            "BF37 06 BF27 03 A2 01 A0",
            "BF37 08 BF27 05 BF2F00 A2 01",
            "BF27 07 BF2F00 A2 02 A000",
        ] {
            assert!(
                matches!(
                    parse_load_bpp_response(&unhex(bad)),
                    Err(Error::Malformed(_))
                ),
                "{bad}"
            );
        }
        // minimal success: metadata without seqNumber
        let r = parse_load_bpp_response(&unhex("BF37 0A BF27 07 BF2F00 A2 02 A000")).unwrap();
        assert_eq!(r, LoadBppResult::default());
        // the sequence number keeps its low 64 bits
        let r = parse_load_bpp_response(&unhex(
            "BF37 15 BF27 12 BF2F 0B 80 09 010000000000000007 A2 02 A000",
        ))
        .unwrap();
        assert_eq!(r.seq_number, 7);
    }

    #[test]
    fn bpp_transport_and_card_errors_fail() {
        let bpp = Bpp::standard().encode();
        let (r, sent) = run(&[&SW_OK, &unhex("6A80")], |e| {
            e.load_bound_profile_package(&b64(&bpp))
        });
        assert_eq!(r, Err(Error::Status(0x6A, 0x80)));
        assert_eq!(sent.len(), 2);
        let (r, _) = run(&[&SW_OK, &with_sw(&[0x01])], |e| {
            e.load_bound_profile_package(&b64(&bpp))
        });
        assert!(
            matches!(r, Err(Error::Malformed(_))),
            "an answer that is not a result"
        );
        let (r, sent) = run(&[&SW_OK, &SW_OK], |e| {
            e.load_bound_profile_package(&b64(&bpp))
        });
        assert!(matches!(r, Err(Error::Transport(_))));
        assert_eq!(sent.len(), 3);
        // a success on an early segment is replaced by later (empty) answers
        let early = with_sw(&installation_result(4, Some(&ICCID), success()));
        let mut responses: Vec<&[u8]> = vec![&early];
        responses.extend(vec![&SW_OK[..]; 10]);
        let (r, _) = run(&responses, |e| e.load_bound_profile_package(&b64(&bpp)));
        assert_eq!(r.unwrap(), LoadBppResult::default());
    }

    // ---- long-form lengths (inherited from crate::der) ----------------------------------------

    #[test]
    fn lengths_of_more_than_4_bytes_are_malformed_unlike_libeuicc() {
        // BF36 { BF23 03 820101, A0 00, A1 86 000000000000, A3 00 }. libeuicc reads the 6-byte
        // zero length exactly and sends 4 segments (..., A1 86 000000000000, A3 00), returning 0.
        // Here A1 is malformed, so the search for it stops: 2 segments, then "no sequenceOf88".
        let bpp = "vzYSvyMDggEBoAChhgAAAAAAAKMA";
        assert_eq!(
            crate::b64::decode(bpp),
            unhex("BF36 12 BF23 03 820101 A000 A1 86 000000000000 A300")
        );
        let (r, sent) = run(&[&SW_OK[..]; 4], |e| e.load_bound_profile_package(bpp));
        assert_eq!(r, Err(Error::Argument("no sequenceOf88")));
        assert_eq!(
            sent,
            [
                unhex("81e2910009 BF36 12 BF23 03 820101"),
                unhex("81e2910002 A000")
            ]
        );
        // with 4 length bytes both accept it and send the same 4 segments
        let bpp = unhex("BF36 10 BF23 03 820101 A000 A1 84 00000000 A300");
        let (r, sent) = run(&[&SW_OK[..]; 4], |e| {
            e.load_bound_profile_package(&b64(&bpp))
        });
        assert_eq!(r.unwrap(), LoadBppResult::default());
        assert_eq!(
            sent,
            [
                unhex("81e2910009 BF36 10 BF23 03 820101"),
                unhex("81e2910002 A000"),
                unhex("81e2910006 A1 84 00000000"),
                unhex("81e2910002 A300")
            ]
        );

        // serverSigned1 30 86 000000000003 8001AA: libeuicc sends AuthenticateServer with
        // transactionId AA; here it is not found and nothing is sent.
        let mut param = AuthenticateServerParam {
            b64_server_signed1: "MIYAAAAAAAOAAao=".into(),
            b64_server_signature1: "XzcA".into(),
            b64_euicc_ci_pkid_to_be_used: "BAA=".into(),
            b64_server_certificate: "MAA=".into(),
        };
        let (r, sent) = run(&[&SW_OK], |e| e.authenticate_server(&param, None, None));
        assert_eq!(r, Err(Error::Argument("no serverSigned1")));
        assert!(sent.is_empty());
        // with 4 length bytes: the APDU libeuicc sends for the 6-byte form
        param.b64_server_signed1 = b64(&unhex("30 84 00000003 8001AA"));
        let (r, sent) = run(&[&SW_OK], |e| e.authenticate_server(&param, None, None));
        assert_eq!(r.unwrap().0, [0xAA]);
        assert_eq!(
            sent,
            [unhex(
                "81e291001b bf3818 30038001aa 5f3700 0400 3000 a00a a108 800435290611 a100"
            )]
        );

        // the same holds for the other server fields: libeuicc reads each as an empty element and
        // sends the request
        for (field, value) in [
            (1, "5F37 85 0000000000"),
            (2, "04 85 0000000000"),
            (3, "30 85 0000000000"),
        ] {
            let mut p = auth_param();
            let text = b64(&unhex(value));
            match field {
                1 => p.b64_server_signature1 = text,
                2 => p.b64_euicc_ci_pkid_to_be_used = text,
                _ => p.b64_server_certificate = text,
            }
            let (r, sent) = run(&[&SW_OK], |e| e.authenticate_server(&p, None, None));
            assert!(matches!(r, Err(Error::Argument(_))), "{value}");
            assert!(sent.is_empty(), "{value}");
        }
        let mut p = prepare_param(&[0]);
        p.b64_smdp_signed2 = b64(&unhex("30 85 0000000006 8001AA 010100"));
        let (r, sent) = run(&[&SW_OK], |e| e.prepare_download(&p, &[], None));
        assert_eq!(r, Err(Error::Argument("no smdpSigned2")));
        assert!(sent.is_empty());
    }

    // ---- cancel session ----------------------------------------------------------------------

    #[test]
    fn cancel_session_request_and_answer() {
        let answer = unhex("BF41 03 81 01 00");
        let mut resp = unhex("30 00");
        resp.extend(&answer);
        let tid: Vec<u8> = (1..=16).collect();
        let (r, sent) = run(&[&with_sw(&resp)], |e| {
            e.cancel_session(&tid, CancelSessionReason::Postponed)
        });
        assert_eq!(r.unwrap(), b64(&answer));
        let mut apdu = unhex("81 E2 91 00 18 BF41 15 80 10");
        apdu.extend(&tid);
        apdu.extend(unhex("81 01 01"));
        assert_eq!(sent, [apdu]);

        for (reason, byte) in [
            (CancelSessionReason::EndUserRejection, 0u8),
            (CancelSessionReason::Timeout, 2),
            (CancelSessionReason::PprNotAllowed, 3),
            (CancelSessionReason::MetadataMismatch, 4),
            (CancelSessionReason::LoadBppExecutionError, 5),
            (CancelSessionReason::Undefined, 0x7F),
        ] {
            let (_, sent) = run(&[&with_sw(&answer)], |e| e.cancel_session(&[0xAB], reason));
            assert_eq!(
                sent[0],
                [
                    0x81, 0xE2, 0x91, 0x00, 0x09, 0xBF, 0x41, 0x06, 0x80, 0x01, 0xAB, 0x81, 0x01,
                    byte
                ]
            );
        }
    }

    #[test]
    fn cancel_session_limits_and_errors() {
        let answer = with_sw(&unhex("BF41 00"));
        let (r, sent) = run(&[&answer], |e| {
            e.cancel_session(&[], CancelSessionReason::Timeout)
        });
        assert_eq!(r, Err(Error::Argument("empty transaction id")));
        assert!(sent.is_empty());
        // 245 bytes is the longest id that fits libeuicc's 255-byte request buffer
        let (r, sent) = run(&[&SW_OK, &SW_OK, &answer], |e| {
            e.cancel_session(&[7; 245], CancelSessionReason::Timeout)
        });
        assert_eq!(r.unwrap(), "v0EA");
        assert_eq!(sent.concat().len(), 255 + 3 * 5);
        for len in [246, 255, 256, 300] {
            let (r, sent) = run(&[&answer], |e| {
                e.cancel_session(&vec![7; len], CancelSessionReason::Timeout)
            });
            assert_eq!(r, Err(Error::Argument("transaction id too long")), "{len}");
            assert!(sent.is_empty());
        }
        for bad in ["9000", "BF40 00 9000", "BF41 02 00 9000"] {
            let (r, _) = run(&[&unhex(bad)], |e| {
                e.cancel_session(&[1], CancelSessionReason::Timeout)
            });
            assert!(matches!(r, Err(Error::Malformed(_))), "{bad}");
        }
        let (r, _) = run(&[&unhex("6F00")], |e| {
            e.cancel_session(&[1], CancelSessionReason::Timeout)
        });
        assert_eq!(r, Err(Error::Status(0x6F, 0x00)));
    }

    // ---- hostile input ----------------------------------------------------------------------

    /// xorshift64: a small deterministic generator for the hostile-input test.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
        fn bytes(&mut self, max: usize) -> Vec<u8> {
            let n = self.below(max + 1);
            (0..n).map(|_| self.next() as u8).collect()
        }
        fn text(&mut self, max: usize) -> String {
            const CHARS: &[char] = &[
                '0', '9', 'a', 'F', 'g', ' ', '\0', '-', '=', '+', '/', 'é', '日', 'Z',
            ];
            (0..self.below(max + 1))
                .map(|_| CHARS[self.below(CHARS.len())])
                .collect()
        }
    }

    /// Nested TLVs with the tags the parsers look for, sometimes cut short or with a byte changed.
    fn random_der(rng: &mut Rng, depth: u32) -> Vec<u8> {
        const TAGS: [u16; 21] = [
            0x30, 0x80, 0x81, 0x82, 0x01, 0x04, 0x5A, 0x5F37, 0xA0, 0xA1, 0xA2, 0xA3, 0x86, 0x87,
            0x88, 0xBF36, 0xBF23, 0xBF37, 0xBF27, 0xBF2F, 0xBF41,
        ];
        let mut out = Vec::new();
        for _ in 0..=rng.below(3) {
            let tag = TAGS[rng.below(TAGS.len())];
            let value = if depth > 0 && rng.below(3) != 0 {
                random_der(rng, depth - 1)
            } else {
                rng.bytes(8)
            };
            out.extend(Node::Prim(tag, value).encode());
        }
        if rng.below(5) == 0 {
            let cut = rng.below(out.len() + 1);
            out.truncate(cut);
        }
        if rng.below(5) == 0 && !out.is_empty() {
            let i = rng.below(out.len());
            out[i] = rng.next() as u8;
        }
        out
    }

    #[test]
    fn hostile_inputs_never_panic() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for _ in 0..4000 {
            let b64_der = |rng: &mut Rng| {
                let mut text = b64(&random_der(rng, 4));
                if rng.below(6) == 0 {
                    let at = rng.below(text.len() + 1);
                    text.insert(at, [' ', '=', '\0', '_'][rng.below(4)]);
                }
                text
            };
            let auth = AuthenticateServerParam {
                b64_server_signed1: b64_der(&mut rng),
                b64_server_signature1: b64_der(&mut rng),
                b64_euicc_ci_pkid_to_be_used: b64_der(&mut rng),
                b64_server_certificate: b64_der(&mut rng),
            };
            let prepare = PrepareDownloadParam {
                b64_profile_metadata: String::new(),
                b64_smdp_signed2: b64_der(&mut rng),
                b64_smdp_signature2: b64_der(&mut rng),
                b64_smdp_certificate: b64_der(&mut rng),
            };
            let bpp = b64_der(&mut rng);
            let text = [rng.text(20), rng.text(20)];
            let tid = rng.bytes(300);
            let responses: Vec<Vec<u8>> = (0..rng.below(14))
                .map(|_| {
                    let mut r = if rng.below(2) == 0 {
                        random_der(&mut rng, 4)
                    } else {
                        Vec::new()
                    };
                    let sw: &[u8] = [
                        &[0x90, 0x00][..],
                        &[0x61, 0x05],
                        &[0x61, 0x00],
                        &[0x6A, 0x80],
                        &[0x91, 0x00],
                        &[],
                    ][rng.below(6)];
                    r.extend_from_slice(sw);
                    r
                })
                .collect();
            let refs: Vec<&[u8]> = responses.iter().map(|r| r.as_slice()).collect();
            let pick = |rng: &mut Rng, i: usize| (rng.below(3) != 0).then_some(text[i].as_str());
            let (matching_id, imei, code) =
                (pick(&mut rng, 0), pick(&mut rng, 1), pick(&mut rng, 0));
            let _ = run(&refs, |e| e.get_euicc_challenge());
            let _ = run(&refs, |e| e.get_euicc_info1());
            let _ = run(&refs, |e| e.authenticate_server(&auth, matching_id, imei));
            let _ = run(&refs, |e| e.prepare_download(&prepare, &tid, code));
            let _ = run(&refs, |e| e.load_bound_profile_package(&bpp));
            let _ = run(&refs, |e| {
                e.cancel_session(&tid, CancelSessionReason::Undefined)
            });
            for r in &responses {
                let _ = parse_load_bpp_response(r);
            }
            let _ = crate::b64::decode(&rng.text(30));
        }
    }

    #[test]
    fn block_numbers_wrap_within_one_segment() {
        // a 30721-byte element needs 257 STORE DATA blocks: P2 runs 00..FF then 00 again
        let big = Node::Prim(0x86, vec![0x5A; 30_717]).encode();
        assert_eq!(big.len(), 30_721);
        let bpp = Node::Cons(
            0xBF36,
            vec![
                Node::Cons(0xBF23, vec![]),
                Node::Prim(0xA0, vec![]),
                Node::Prim(0xA1, vec![]),
                Node::Prim(0xA3, big.clone()),
            ],
        )
        .encode();
        let done = with_sw(&installation_result(1, None, success()));
        let responses: Vec<&[u8]> = [vec![&SW_OK[..]; 4 + 256], vec![&done[..]]].concat();
        let (r, sent) = run(&responses, |e| e.load_bound_profile_package(&b64(&bpp)));
        assert!(r.unwrap().is_success());
        let blocks = &sent[4..];
        assert_eq!(blocks.len(), 257);
        assert_eq!(&blocks[255][..4], &[0x81, 0xE2, 0x11, 0xFF]);
        assert_eq!(
            &blocks[256][..5],
            &[0x81, 0xE2, 0x91, 0x00, 1],
            "block 256 is numbered 0"
        );
        assert_eq!(
            blocks
                .iter()
                .map(|b| b[5..].to_vec())
                .collect::<Vec<_>>()
                .concat(),
            big
        );
    }
}
