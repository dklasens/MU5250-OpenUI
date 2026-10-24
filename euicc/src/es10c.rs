// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2023-2025 ESTKME TECHNOLOGY LIMITED, Hong Kong
// SPDX-FileCopyrightText: 2026 Jesther Silvestre
//
// Rust translation of lpac euicc/es10c.c, euicc/es10c.h and the profile parts of euicc/tostr.c
// (lpac commit c7c8d4f).
// Changed 2026-10-19: rewritten in Rust returning typed values; eUICC memory reset is not
// translated (it erases every profile and nothing here needs it); result codes are named with
// the SGP.22 ES10c value names (result_reason).

//! ES10c: profile list, enable, disable, delete, EID, nickname.

use crate::der::{self, Node};
use crate::{hex, Apdu, Error, Euicc, Result};
use base64::Engine;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileInfo {
    pub iccid: String,
    pub isdp_aid: String,
    /// `disabled`, `enabled`, or `unknown`; None when the card omitted it.
    pub profile_state: Option<&'static str>,
    pub profile_nickname: Option<String>,
    pub service_provider_name: Option<String>,
    pub profile_name: Option<String>,
    /// `jpeg`, `png` or `unknown`.
    pub icon_type: Option<&'static str>,
    /// Base64 of the icon file.
    pub icon: Option<String>,
    /// `test`, `provisioning`, `operational` or `unknown`.
    pub profile_class: Option<&'static str>,
}

fn text(value: &[u8]) -> String {
    String::from_utf8_lossy(value).into_owned()
}

fn profile_state(value: u64) -> &'static str {
    match value {
        0 => "disabled",
        1 => "enabled",
        _ => "unknown",
    }
}

fn profile_class(value: u64) -> &'static str {
    match value {
        0 => "test",
        1 => "provisioning",
        2 => "operational",
        _ => "unknown",
    }
}

fn icon_type(value: u64) -> &'static str {
    match value {
        0 => "jpeg",
        1 => "png",
        _ => "unknown",
    }
}

/// Parses a ProfileInfoListResponse body (inside BF2D).
pub fn parse_profiles(body: &[u8]) -> Result<Vec<ProfileInfo>> {
    let ok = der::find(body, 0xA0).ok_or(Error::Malformed("profile list error"))?;
    Ok(der::iter(ok.value)
        .filter(|t| t.tag == 0xE3)
        .map(|info| {
            let mut p = ProfileInfo::default();
            for t in der::iter(info.value) {
                match t.tag {
                    0x5A => p.iccid = hex::bcd_to_iccid(t.value),
                    0x4F => p.isdp_aid = hex::encode(t.value),
                    0x9F70 => p.profile_state = Some(profile_state(der::to_u64(t.value))),
                    0x90 => p.profile_nickname = Some(text(t.value)),
                    0x91 => p.service_provider_name = Some(text(t.value)),
                    0x92 => p.profile_name = Some(text(t.value)),
                    0x93 => p.icon_type = Some(icon_type(der::to_u64(t.value))),
                    0x94 => {
                        p.icon = Some(base64::engine::general_purpose::STANDARD.encode(t.value))
                    }
                    0x95 => p.profile_class = Some(profile_class(der::to_u64(t.value))),
                    _ => {}
                }
            }
            p
        })
        .collect())
}

/// A profile identifier: a 32-hex-digit ISD-P AID, or an ICCID.
fn profile_id(id: &str) -> Result<Node> {
    if id.len() == 32 {
        let aid = hex::decode(id).ok_or(Error::Argument("ISD-P AID"))?;
        Ok(Node::Prim(0x4F, aid))
    } else {
        if id.is_empty() || id.len() > 20 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Argument("ICCID"));
        }
        Ok(Node::Prim(
            0x5A,
            hex::iccid_to_bcd(id, 10).ok_or(Error::Argument("ICCID"))?,
        ))
    }
}

/// EnableProfile / DisableProfile request; `refresh` asks the card to tell the modem to reload.
fn switch_request(tag: u16, id: &str, refresh: bool) -> Result<Node> {
    Ok(Node::Cons(
        tag,
        vec![
            Node::Cons(0xA0, vec![profile_id(id)?]),
            Node::Prim(0x81, vec![if refresh { 0xFF } else { 0x00 }]),
        ],
    ))
}

impl<T: Apdu> Euicc<T> {
    /// GetProfilesInfo (ProfileInfoListRequest, BF2D).
    pub fn profiles(&mut self) -> Result<Vec<ProfileInfo>> {
        let body = self.request(Node::Prim(0xBF2D, Vec::new()))?;
        parse_profiles(&body)
    }

    /// EnableProfile (BF31). Returns the card's result code (0 = ok, 5 = catBusy...).
    pub fn enable_profile(&mut self, id: &str, refresh: bool) -> Result<u64> {
        self.result_code(switch_request(0xBF31, id, refresh)?)
    }

    /// DisableProfile (BF32). Returns the card's result code.
    pub fn disable_profile(&mut self, id: &str, refresh: bool) -> Result<u64> {
        self.result_code(switch_request(0xBF32, id, refresh)?)
    }

    /// DeleteProfile (BF33). Returns the card's result code.
    pub fn delete_profile(&mut self, id: &str) -> Result<u64> {
        self.result_code(Node::Cons(0xBF33, vec![profile_id(id)?]))
    }

    /// GetEID (GetEuiccDataRequest BF3E with tag list 5A), as 32 hex digits.
    pub fn eid(&mut self) -> Result<String> {
        let body = self.request(Node::Cons(0xBF3E, vec![Node::Prim(0x5C, vec![0x5A])]))?;
        der::find(&body, 0x5A)
            .map(|t| hex::encode(t.value))
            .ok_or(Error::Malformed("no EID"))
    }

    /// SetNickname (BF29). Returns the card's result code.
    pub fn set_nickname(&mut self, iccid: &str, nickname: &str) -> Result<u64> {
        let bcd = hex::iccid_to_bcd(iccid, 10).ok_or(Error::Argument("ICCID"))?;
        self.result_code(Node::Cons(
            0xBF29,
            vec![
                Node::Prim(0x5A, bcd),
                Node::Prim(0x90, nickname.as_bytes().to_vec()),
            ],
        ))
    }
}

/// The SGP.22 name of an EnableProfile / DisableProfile / DeleteProfile / SetNickname result code
/// (the INTEGER named values in the ES10c response definitions).
pub fn result_reason(op: &str, code: u64) -> String {
    let named = match (op, code) {
        (_, 1) if op == "set_nickname" => "iccidNotFound",
        (_, 1) => "iccidOrAidNotFound",
        ("enable_profile" | "delete_profile", 2) => "profileNotInDisabledState",
        ("disable_profile", 2) => "profileNotInEnabledState",
        (_, 3) if op != "set_nickname" => "disallowedByPolicy",
        ("enable_profile", 4) => "wrongProfileReenabling",
        ("enable_profile" | "disable_profile", 5) => "catBusy",
        (_, 127) => "undefinedError",
        _ => "",
    };
    if named.is_empty() {
        format!("unknown result {code}")
    } else {
        named.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Script;

    #[test]
    fn profile_list_decodes_every_field() {
        // BF2D { A0 { E3 {...} E3 {...} } }
        let p1 = Node::Cons(
            0xE3,
            vec![
                Node::Prim(0x5A, hex::iccid_to_bcd("8963000000000000001", 10).unwrap()),
                Node::Prim(0x4F, vec![0xA0, 0x00, 0x00, 0x05, 0x59, 0x10, 0x10, 0x01]),
                Node::Prim(0x9F70, vec![1]),
                Node::Prim(0x90, b"Smart".to_vec()),
                Node::Prim(0x91, b"SMART".to_vec()),
                Node::Prim(0x92, b"Operational".to_vec()),
                Node::Prim(0x95, vec![2]),
            ],
        );
        let p2 = Node::Cons(
            0xE3,
            vec![
                Node::Prim(0x5A, hex::iccid_to_bcd("89630000000000000027", 10).unwrap()),
                Node::Prim(0x9F70, vec![0]),
                Node::Prim(0x93, vec![1]),
                Node::Prim(0x94, vec![1, 2, 3]),
                Node::Prim(0x95, vec![9]),
            ],
        );
        let resp = Node::Cons(0xBF2D, vec![Node::Cons(0xA0, vec![p1, p2])]).encode();
        let mut sw = resp.clone();
        sw.extend([0x90, 0x00]);
        let mut card = Script::new(1, &[&sw]);
        let list = Euicc::open(&mut card).unwrap().profiles().unwrap();
        assert_eq!(
            card.sent[0],
            [0x81, 0xE2, 0x91, 0x00, 0x03, 0xBF, 0x2D, 0x00]
        );
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].iccid, "8963000000000000001");
        assert_eq!(list[0].isdp_aid, "a000000559101001");
        assert_eq!(
            (list[0].profile_state, list[0].profile_class),
            (Some("enabled"), Some("operational"))
        );
        assert_eq!(list[0].profile_nickname.as_deref(), Some("Smart"));
        assert_eq!(list[0].service_provider_name.as_deref(), Some("SMART"));
        assert_eq!(list[1].iccid, "89630000000000000027");
        assert_eq!(list[1].profile_nickname, None);
        assert_eq!(
            (list[1].icon_type, list[1].icon.as_deref()),
            (Some("png"), Some("AQID"))
        );
        assert_eq!(list[1].profile_class, Some("unknown"));
    }

    #[test]
    fn enable_request_matches_libeuicc_and_reads_the_result() {
        let mut card = Script::new(1, &[&[0xBF, 0x31, 0x03, 0x80, 0x01, 0x05, 0x90, 0x00]]);
        let code = Euicc::open(&mut card)
            .unwrap()
            .enable_profile("8963000000000000001", true)
            .unwrap();
        assert_eq!(code, 5);
        let mut expected = vec![
            0x81, 0xE2, 0x91, 0x00, 0x14, 0xBF, 0x31, 0x11, 0xA0, 0x0C, 0x5A, 0x0A,
        ];
        expected.extend(hex::iccid_to_bcd("8963000000000000001", 10).unwrap());
        expected.extend([0x81, 0x01, 0xFF]);
        assert_eq!(card.sent[0], expected);
        assert_eq!(result_reason("enable_profile", 5), "catBusy");
        assert_eq!(result_reason("enable_profile", 6), "unknown result 6");
        assert_eq!(
            result_reason("delete_profile", 2),
            "profileNotInDisabledState"
        );
        assert_eq!(result_reason("set_nickname", 1), "iccidNotFound");
    }

    #[test]
    fn delete_eid_and_nickname_requests() {
        let mut card = Script::new(
            1,
            &[
                &[0xBF, 0x33, 0x03, 0x80, 0x01, 0x00, 0x90, 0x00],
                &[
                    0xBF, 0x3E, 0x06, 0x5A, 0x04, 0x89, 0x00, 0x00, 0x01, 0x90, 0x00,
                ],
                &[0xBF, 0x29, 0x03, 0x80, 0x01, 0x00, 0x90, 0x00],
            ],
        );
        let mut e = Euicc::open(&mut card).unwrap();
        assert_eq!(e.delete_profile("8963000000000000001").unwrap(), 0);
        assert_eq!(e.eid().unwrap(), "89000001");
        assert_eq!(e.set_nickname("8963000000000000001", "Travel").unwrap(), 0);
        drop(e);
        assert_eq!(
            &card.sent[0][5..9],
            &[0xBF, 0x33, 0x0C, 0x5A],
            "delete has no A0 wrapper"
        );
        assert_eq!(&card.sent[1][5..], &[0xBF, 0x3E, 0x03, 0x5C, 0x01, 0x5A]);
        assert_eq!(&card.sent[2][card.sent[2].len() - 8..], b"\x90\x06Travel");
        assert!(Euicc::open(&mut Script::new(1, &[]))
            .unwrap()
            .enable_profile("not-an-iccid", true)
            .is_err());
    }
}
