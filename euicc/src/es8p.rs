// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2023-2025 ESTKME TECHNOLOGY LIMITED, Hong Kong
// SPDX-FileCopyrightText: 2026 Jesther Silvestre
//
// Rust translation of lpac euicc/es8p.c and euicc/es8p.h
// (lpac commit c7c8d4f).
// Changed 2026-10-19: rewritten in Rust returning an owned struct (a Vec of access rules instead
// of a linked list, enum values as the strings of euicc/tostr.c); a zero-length ICCID is read as
// an empty ICCID instead of overrunning the ICCID buffer; text fields with invalid UTF-8 get
// U+FFFD replacement characters.

//! ES8+ profile metadata: the StoreMetadataRequest an SM-DP+ returns with AuthenticateClient
//! (`profileMetadata`), decoded for the download preview before the user accepts the profile.

use crate::der;
use crate::hex;
use crate::{Error, Result};
use base64::Engine;

/// `notificationConfigurationInfo` of `struct es8p_metadata`. libeuicc never decodes this (tag
/// B6 is only logged), so it is always empty; kept for parity with the C struct.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotificationConfigurationInfo {
    pub profile_management_operation: Vec<&'static str>,
    pub notification_address: Option<String>,
}

/// `profileOwner` of `struct es8p_metadata`. libeuicc never decodes this (tag B7 is only
/// logged), so it is always empty; kept for parity with the C struct.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileOwner {
    pub mccmnc: Option<String>,
    pub gid1: Option<String>,
    pub gid2: Option<String>,
}

/// `dpProprietaryData` of `struct es8p_metadata`. libeuicc never decodes this, so it is always
/// empty; kept for parity with the C struct.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DpProprietaryData {
    pub dp_oid: Option<String>,
}

/// One Android access rule (`struct es8p_metadata_access_rule`): a REF-AR-DO from the
/// metadata's BF76 extension.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccessRule {
    /// Lowercase hex of the REF-DO's DeviceAppID-REF-DO (C1). Always present in a parsed rule
    /// (a REF-DO without one fails the parse); may be empty.
    pub certificate_hash: Option<String>,
    /// The REF-DO's PKG-REF-DO (CA), when present.
    pub package_name: Option<String>,
}

/// Decoded StoreMetadataRequest (`struct es8p_metadata`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileMetadata {
    /// ICCID digits (trailing `f` padding removed); empty when the metadata has none.
    pub iccid: String,
    pub service_provider_name: Option<String>,
    pub profile_name: Option<String>,
    /// `jpeg`, `png` or `unknown`.
    pub icon_type: Option<&'static str>,
    /// Base64 of the icon file.
    pub icon: Option<String>,
    /// `test`, `provisioning`, `operational` or `unknown`.
    pub profile_class: Option<&'static str>,
    /// Never filled by libeuicc; always empty.
    pub notification_configuration_info: NotificationConfigurationInfo,
    /// Never filled by libeuicc; always empty.
    pub profile_owner: ProfileOwner,
    /// Never filled by libeuicc; always empty.
    pub dp_proprietary_data: DpProprietaryData,
    /// Never filled by libeuicc (tag 99 is only logged); always empty.
    pub profile_policy_rules: Vec<&'static str>,
    /// Android access rules (BF76), in order.
    pub access_rules: Vec<AccessRule>,
}

/// A text field as C sees it: the bytes up to the first NUL (where the C string ends), with
/// invalid UTF-8 replaced.
fn text(value: &[u8]) -> String {
    let end = value.iter().position(|&b| b == 0).unwrap_or(value.len());
    String::from_utf8_lossy(&value[..end]).into_owned()
}

/// An enumerated INTEGER as libeuicc reads it: `euicc_derutil_convert_bin2long` stored into an
/// `int`, i.e. the low 32 bits of the big-endian contents (empty contents read as 0).
fn enumerated(value: &[u8]) -> u32 {
    der::to_u64(value) as u32
}

fn icon_type(value: &[u8]) -> &'static str {
    match enumerated(value) {
        0 => "jpeg",
        1 => "png",
        _ => "unknown",
    }
}

fn profile_class(value: &[u8]) -> &'static str {
    match enumerated(value) {
        0 => "test",
        1 => "provisioning",
        2 => "operational",
        _ => "unknown",
    }
}

/// The REF-AR-DO (E2) entries inside the BF76 extension (`es8p_metadata_parse_access_rules`).
/// Other entries are skipped; any rule that is not exactly one REF-DO (E1) holding exactly one
/// certificate hash (C1) and at most one package name (CA) fails the whole metadata.
fn parse_access_rules(value: &[u8]) -> Result<Vec<AccessRule>> {
    let mut rules = Vec::new();
    for entry in der::iter(value).filter(|t| t.tag == 0xE2) {
        let mut ref_dos = der::iter(entry.value).filter(|t| t.tag == 0xE1);
        let ref_do = ref_dos
            .next()
            .ok_or(Error::Malformed("access rule without REF-DO"))?;
        if ref_dos.next().is_some() {
            return Err(Error::Malformed("access rule with several REF-DOs"));
        }
        let mut rule = AccessRule::default();
        for t in der::iter(ref_do.value) {
            match t.tag {
                0xC1 => {
                    if rule.certificate_hash.is_some() {
                        return Err(Error::Malformed(
                            "access rule with several certificate hashes",
                        ));
                    }
                    rule.certificate_hash = Some(hex::encode(t.value));
                }
                0xCA => {
                    if rule.package_name.is_some() {
                        return Err(Error::Malformed("access rule with several package names"));
                    }
                    rule.package_name = Some(text(t.value));
                }
                _ => {}
            }
        }
        if rule.certificate_hash.is_none() {
            return Err(Error::Malformed("access rule without certificate hash"));
        }
        rules.push(rule);
    }
    Ok(rules)
}

/// Decodes the StoreMetadataRequest (BF25) found among the top-level elements of `buf`.
fn parse_store_metadata(buf: &[u8]) -> Result<ProfileMetadata> {
    let request = der::find(buf, 0xBF25).ok_or(Error::Malformed("no StoreMetadataRequest"))?;
    let mut m = ProfileMetadata::default();
    // Like libeuicc, a repeated element replaces the earlier one, and decoding stops quietly
    // at the first malformed element, keeping what came before it.
    for t in der::iter(request.value) {
        match t.tag {
            // An ICCID over 10 bytes does not fit libeuicc's buffer and is skipped.
            0x5A if t.value.len() <= 10 => m.iccid = hex::bcd_to_iccid(t.value),
            0x91 => m.service_provider_name = Some(text(t.value)),
            0x92 => m.profile_name = Some(text(t.value)),
            0x93 => m.icon_type = Some(icon_type(t.value)),
            0x94 => m.icon = Some(base64::engine::general_purpose::STANDARD.encode(t.value)),
            0x95 => m.profile_class = Some(profile_class(t.value)),
            // Android extension: access rules (AR-DO) in the profile metadata.
            0xBF76 => m.access_rules = parse_access_rules(t.value)?,
            // notificationConfigurationInfo (B6), profileOwner (B7), profilePolicyRules (99)
            // and everything else are not decoded.
            _ => {}
        }
    }
    Ok(m)
}

/// Decodes the base64 `profileMetadata` from ES9+ AuthenticateClient (`es8p_metadata_parse`).
///
/// Fails when the text holds no complete StoreMetadataRequest (BF25) or when its access rules
/// are invalid; anything else malformed only ends decoding early, as in libeuicc. One inherited
/// difference: an element whose length takes more than 4 octets is malformed here (`der::parse`)
/// where libeuicc keeps the low 32 bits of the length.
pub fn parse_metadata(b64_metadata: &str) -> Result<ProfileMetadata> {
    parse_store_metadata(&crate::b64::decode(b64_metadata))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::der::Node;
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};

    fn b64(der: &[u8]) -> String {
        STANDARD.encode(der)
    }

    fn metadata(children: Vec<Node>) -> Vec<u8> {
        Node::Cons(0xBF25, children).encode()
    }

    fn parse(children: Vec<Node>) -> Result<ProfileMetadata> {
        parse_metadata(&b64(&metadata(children)))
    }

    fn rule(children: Vec<Node>) -> Node {
        Node::Cons(0xE2, children)
    }

    fn ref_do(children: Vec<Node>) -> Node {
        Node::Cons(0xE1, children)
    }

    fn rules(entries: Vec<Node>) -> Result<Vec<AccessRule>> {
        parse(vec![Node::Cons(0xBF76, entries)]).map(|m| m.access_rules)
    }

    fn ar(hash: &str, pkg: Option<&str>) -> AccessRule {
        AccessRule {
            certificate_hash: Some(hash.into()),
            package_name: pkg.map(Into::into),
        }
    }

    /// Deterministic xorshift for the no-panic sweeps (no extra dependencies).
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
    }

    #[test]
    fn decodes_every_field() {
        let m = parse(vec![
            Node::Prim(0x5A, hex::iccid_to_bcd("8963000000000000001", 10).unwrap()),
            Node::Prim(0x91, b"Smart".to_vec()),
            Node::Prim(0x92, b"Smart Prepaid".to_vec()),
            Node::Prim(0x93, vec![1]),
            Node::Prim(0x94, vec![0x89, b'P', b'N', b'G']),
            Node::Prim(0x95, vec![2]),
            Node::Cons(
                0xB6,
                vec![
                    Node::Prim(0x81, vec![0x04, 0xF0]),
                    Node::Prim(0x80, b"dp.example".to_vec()),
                ],
            ),
            Node::Cons(0xB7, vec![Node::Prim(0x80, vec![0x15, 0xF2, 0x30])]),
            Node::Prim(0x99, vec![0x06, 0x40]),
            Node::Cons(0xB8, vec![Node::Prim(0x80, vec![0x2A, 0x03])]),
            Node::Cons(
                0xBF76,
                vec![
                    rule(vec![
                        ref_do(vec![
                            Node::Prim(0xC1, vec![0xAB; 32]),
                            Node::Prim(0xCA, b"com.example.app".to_vec()),
                        ]),
                        Node::Cons(0xE3, vec![Node::Prim(0xDB, vec![0x01])]),
                    ]),
                    rule(vec![ref_do(vec![Node::Prim(0xC1, vec![0x01, 0x23])])]),
                ],
            ),
        ])
        .unwrap();
        assert_eq!(m.iccid, "8963000000000000001");
        assert_eq!(m.service_provider_name.as_deref(), Some("Smart"));
        assert_eq!(m.profile_name.as_deref(), Some("Smart Prepaid"));
        assert_eq!(m.icon_type, Some("png"));
        assert_eq!(m.icon.as_deref(), Some("iVBORw=="));
        assert_eq!(m.profile_class, Some("operational"));
        assert_eq!(
            m.access_rules,
            [
                ar(&"ab".repeat(32), Some("com.example.app")),
                ar("0123", None)
            ]
        );
        // libeuicc only logs B6/B7/99 and ignores B8: those fields stay empty.
        assert_eq!(
            m.notification_configuration_info,
            NotificationConfigurationInfo::default()
        );
        assert_eq!(m.profile_owner, ProfileOwner::default());
        assert_eq!(m.dp_proprietary_data, DpProprietaryData::default());
        assert!(m.profile_policy_rules.is_empty());
    }

    #[test]
    fn absent_fields_stay_empty() {
        assert_eq!(parse(vec![]).unwrap(), ProfileMetadata::default());
        let m = parse(vec![Node::Prim(0x91, b"GOMO".to_vec())]).unwrap();
        assert_eq!(
            (
                m.iccid.as_str(),
                m.profile_name,
                m.icon_type,
                m.icon,
                m.profile_class
            ),
            ("", None, None, None, None)
        );
        assert!(m.access_rules.is_empty());
    }

    #[test]
    fn base64_is_as_tolerant_as_libeuicc() {
        let der = metadata(vec![Node::Prim(0x91, b"DITO".to_vec())]);
        let want = parse_metadata(&b64(&der)).unwrap();
        assert_eq!(want.service_provider_name.as_deref(), Some("DITO"));
        // Padding is optional, and everything from the first non-alphabet character is ignored.
        assert_eq!(parse_metadata(&STANDARD_NO_PAD.encode(&der)).unwrap(), want);
        assert_eq!(parse_metadata(&format!("{}\r\n", b64(&der))).unwrap(), want);
        assert_eq!(
            parse_metadata(&format!("{}=garbage!", STANDARD_NO_PAD.encode(&der))).unwrap(),
            want
        );
        // A break inside the text truncates the data: BF25 is then incomplete.
        let text = b64(&der);
        assert!(parse_metadata(&format!("{}\n{}", &text[..4], &text[4..])).is_err());
        assert!(parse_metadata(&format!(" {text}")).is_err());
        // URL-safe characters end the input like any other non-alphabet character.
        assert_eq!(crate::b64::decode("QUJD-QUJD"), b"ABC");
        assert_eq!(crate::b64::decode("QUJD_"), b"ABC");
        // Groups of 2 and 3 characters give 1 and 2 bytes; a lone final character is dropped.
        assert_eq!(crate::b64::decode("QQ"), b"A");
        assert_eq!(crate::b64::decode("QUI"), b"AB");
        assert_eq!(crate::b64::decode("QUJDR"), b"ABC");
        assert_eq!(crate::b64::decode("Q"), b"");
        // Unused low bits are not checked.
        assert_eq!(crate::b64::decode("QR"), b"A");
        assert_eq!(crate::b64::decode("QUJ"), b"AB");
        assert_eq!(crate::b64::decode(""), b"");
        assert_eq!(crate::b64::decode("===="), b"");
        assert_eq!(crate::b64::decode("QUJDé"), b"ABC");
        assert_eq!(crate::b64::decode("QU\0JD"), b"A");
        assert_eq!(crate::b64::decode("+/+/"), [0xFB, 0xFF, 0xBF]);
    }

    #[test]
    fn base64_matches_the_strict_decoder_on_canonical_text() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for len in 0..200 {
            let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
            assert_eq!(crate::b64::decode(&STANDARD.encode(&bytes)), bytes);
            assert_eq!(crate::b64::decode(&STANDARD_NO_PAD.encode(&bytes)), bytes);
        }
    }

    #[test]
    fn needs_a_store_metadata_request() {
        assert_eq!(
            parse_metadata(""),
            Err(Error::Malformed("no StoreMetadataRequest"))
        );
        assert!(parse_metadata("!!!!").is_err());
        assert!(
            parse_metadata(&b64(&Node::Prim(0xBF24, vec![]).encode())).is_err(),
            "other tag"
        );
        assert!(parse_metadata(&b64(&[0xBF])).is_err(), "short tag");
        assert!(parse_metadata(&b64(&[0xBF, 0x25])).is_err(), "no length");
        assert!(
            parse_metadata(&b64(&[0xBF, 0x25, 0x03, 0x91, 0x01])).is_err(),
            "truncated"
        );
        assert!(
            parse_metadata(&b64(&[0xBF, 0x25, 0x82, 0x01])).is_err(),
            "truncated length"
        );
        // Well-formed elements before it are skipped; a malformed one hides it.
        let mut buf = Node::Prim(0x30, vec![1, 2, 3]).encode();
        buf.extend(metadata(vec![Node::Prim(0x92, b"x".to_vec())]));
        assert_eq!(
            parse_metadata(&b64(&buf)).unwrap().profile_name.as_deref(),
            Some("x")
        );
        let mut buf = vec![0x30, 0x05, 0x00];
        buf.extend(metadata(vec![]));
        assert!(parse_metadata(&b64(&buf)).is_err());
        // Trailing bytes are ignored; the first BF25 wins.
        let mut buf = metadata(vec![Node::Prim(0x92, b"first".to_vec())]);
        buf.extend(metadata(vec![Node::Prim(0x92, b"second".to_vec())]));
        buf.extend([0xFF, 0xFF]);
        assert_eq!(
            parse_metadata(&b64(&buf)).unwrap().profile_name.as_deref(),
            Some("first")
        );
        // The empty request and the indefinite-length octet (read as length 0) both parse.
        assert_eq!(
            parse_metadata(&b64(&[0xBF, 0x25, 0x00])).unwrap(),
            ProfileMetadata::default()
        );
        assert_eq!(
            parse_metadata(&b64(&[0xBF, 0x25, 0x80, 0x91])).unwrap(),
            ProfileMetadata::default()
        );
    }

    #[test]
    fn stops_quietly_at_a_malformed_child() {
        let mut body = Node::Prim(0x91, b"kept".to_vec()).encode();
        body.extend([0x92, 0x05, b'x']); // truncated: claims 5 bytes, 1 present
        let mut buf = vec![0xBF, 0x25, body.len() as u8];
        buf.extend(&body);
        let m = parse_metadata(&b64(&buf)).unwrap();
        assert_eq!(
            (m.service_provider_name.as_deref(), m.profile_name),
            (Some("kept"), None)
        );
        // A broken BF76 that is itself the malformed child is never looked at.
        let mut body = Node::Prim(0x91, b"kept".to_vec()).encode();
        body.extend([0xBF, 0x76, 0x7F, 0xE2, 0x00]);
        let mut buf = vec![0xBF, 0x25, body.len() as u8];
        buf.extend(&body);
        assert!(parse_metadata(&b64(&buf)).unwrap().access_rules.is_empty());
    }

    #[test]
    fn iccids_follow_libeuicc() {
        let iccid = |v: &[u8]| parse(vec![Node::Prim(0x5A, v.to_vec())]).unwrap().iccid;
        assert_eq!(
            iccid(&hex::iccid_to_bcd("89630000000000000027", 10).unwrap()),
            "89630000000000000027"
        );
        assert_eq!(iccid(&[0x98, 0x36]), "8963");
        assert_eq!(iccid(&[0xF8]), "8");
        assert_eq!(iccid(&[0xFF; 10]), "f", "the first digit is never trimmed");
        assert_eq!(
            iccid(&[0xAB, 0xCD]),
            "badc",
            "non-decimal nibbles pass through"
        );
        assert_eq!(
            iccid(&[]),
            "",
            "libeuicc overruns its buffer here; read as empty"
        );
        assert_eq!(iccid(&[0x98; 11]), "", "over 10 bytes is skipped");
        // A later oversized ICCID keeps the earlier one; a later valid one replaces it.
        let m = parse(vec![
            Node::Prim(0x5A, vec![0x98, 0x36]),
            Node::Prim(0x5A, vec![0x11; 11]),
        ])
        .unwrap();
        assert_eq!(m.iccid, "8963");
        let m = parse(vec![
            Node::Prim(0x5A, vec![0x98; 10]),
            Node::Prim(0x5A, vec![0x21]),
        ])
        .unwrap();
        assert_eq!(m.iccid, "12");
        let m = parse(vec![
            Node::Prim(0x5A, vec![0x98; 10]),
            Node::Prim(0x5A, vec![]),
        ])
        .unwrap();
        assert_eq!(m.iccid, "");
    }

    #[test]
    fn text_ends_at_nul_and_survives_bad_utf8() {
        let m = parse(vec![
            Node::Prim(0x91, b"Smart\0hidden".to_vec()),
            Node::Prim(0x92, vec![b'a', 0xFF, b'b']),
        ])
        .unwrap();
        assert_eq!(m.service_provider_name.as_deref(), Some("Smart"));
        assert_eq!(m.profile_name.as_deref(), Some("a\u{FFFD}b"));
        let m = parse(vec![
            Node::Prim(0x91, vec![]),
            Node::Prim(0x92, vec![0, b'x']),
        ])
        .unwrap();
        assert_eq!(
            (
                m.service_provider_name.as_deref(),
                m.profile_name.as_deref()
            ),
            (Some(""), Some(""))
        );
        let m = parse(vec![Node::Prim(0x92, "Globe 日本".as_bytes().to_vec())]).unwrap();
        assert_eq!(m.profile_name.as_deref(), Some("Globe 日本"));
        // Repeats replace earlier values.
        let m = parse(vec![
            Node::Prim(0x91, b"one".to_vec()),
            Node::Prim(0x91, b"two".to_vec()),
        ])
        .unwrap();
        assert_eq!(m.service_provider_name.as_deref(), Some("two"));
        // Long-form lengths.
        let long = "n".repeat(300);
        let m = parse(vec![Node::Prim(0x92, long.clone().into_bytes())]).unwrap();
        assert_eq!(m.profile_name, Some(long));
    }

    #[test]
    fn enumerations_read_like_libeuicc() {
        let get = |tag: u16, v: &[u8]| {
            let m = parse(vec![Node::Prim(tag, v.to_vec())]).unwrap();
            if tag == 0x93 {
                m.icon_type
            } else {
                m.profile_class
            }
        };
        assert_eq!(get(0x93, &[0]), Some("jpeg"));
        assert_eq!(get(0x93, &[1]), Some("png"));
        assert_eq!(get(0x93, &[2]), Some("unknown"));
        assert_eq!(get(0x93, &[0xFF]), Some("unknown"));
        assert_eq!(get(0x93, &[]), Some("jpeg"), "empty contents read as 0");
        assert_eq!(get(0x93, &[0, 0, 1]), Some("png"));
        assert_eq!(get(0x95, &[0]), Some("test"));
        assert_eq!(get(0x95, &[1]), Some("provisioning"));
        assert_eq!(get(0x95, &[2]), Some("operational"));
        assert_eq!(get(0x95, &[3]), Some("unknown"));
        assert_eq!(get(0x95, &[]), Some("test"));
        assert_eq!(get(0x95, &[0x01, 0x00]), Some("unknown"));
        // Only the low 32 bits count (libeuicc stores the value in an int).
        assert_eq!(get(0x95, &[0x01, 0, 0, 0, 2]), Some("operational"));
        assert_eq!(get(0x93, &[0xFF; 9]), Some("unknown"));
        assert_eq!(
            get(0x93, &[0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]),
            Some("png")
        );
        let m = parse(vec![Node::Prim(0x95, vec![2]), Node::Prim(0x95, vec![0])]).unwrap();
        assert_eq!(m.profile_class, Some("test"), "last one wins");
    }

    #[test]
    fn icons_are_reencoded() {
        let icon = |v: &[u8]| parse(vec![Node::Prim(0x94, v.to_vec())]).unwrap().icon;
        assert_eq!(icon(&[]).as_deref(), Some(""));
        assert_eq!(icon(&[0xFF]).as_deref(), Some("/w=="));
        assert_eq!(icon(&[0xFB, 0xEF]).as_deref(), Some("++8="));
        let big: Vec<u8> = (0..70_000u32).map(|i| i as u8).collect();
        assert_eq!(
            icon(&big),
            Some(STANDARD.encode(&big)),
            "three-octet length"
        );
    }

    #[test]
    fn access_rules_are_validated() {
        let hash = || Node::Prim(0xC1, vec![0x5A, 0xA5]);
        let pkg = |p: &[u8]| Node::Prim(0xCA, p.to_vec());
        // Not exactly one REF-DO.
        assert!(rules(vec![rule(vec![])]).is_err());
        assert!(rules(vec![rule(vec![Node::Cons(0xE3, vec![])])]).is_err());
        assert!(rules(vec![rule(vec![ref_do(vec![hash()]), ref_do(vec![hash()])])]).is_err());
        // Not exactly one certificate hash, or two package names.
        assert!(rules(vec![rule(vec![ref_do(vec![])])]).is_err());
        assert!(rules(vec![rule(vec![ref_do(vec![pkg(b"a")])])]).is_err());
        assert!(rules(vec![rule(vec![ref_do(vec![hash(), hash()])])]).is_err());
        assert!(rules(vec![rule(vec![ref_do(vec![
            Node::Prim(0xC1, vec![]),
            hash()
        ])])])
        .is_err());
        assert!(rules(vec![rule(vec![ref_do(vec![hash(), pkg(b"a"), pkg(b"b")])])]).is_err());
        assert!(rules(vec![rule(vec![ref_do(vec![hash(), pkg(b""), pkg(b"")])])]).is_err());
        // A bad rule after good ones still fails the whole metadata.
        assert!(rules(vec![rule(vec![ref_do(vec![hash()])]), rule(vec![])]).is_err());
        // Accepted shapes.
        assert_eq!(rules(vec![]).unwrap(), []);
        assert_eq!(
            rules(vec![rule(vec![ref_do(vec![Node::Prim(0xC1, vec![])])])]).unwrap(),
            [ar("", None)]
        );
        assert_eq!(
            rules(vec![rule(vec![ref_do(vec![
                pkg(b"p\0q"),
                Node::Prim(0xDD, vec![1]),
                hash()
            ])])])
            .unwrap(),
            [ar("5aa5", Some("p"))]
        );
        assert_eq!(
            rules(vec![
                Node::Prim(0x30, vec![]),
                rule(vec![
                    Node::Prim(0xD0, vec![]),
                    ref_do(vec![hash(), pkg(b"")])
                ]),
                Node::Cons(0xE1, vec![]),
            ])
            .unwrap(),
            [ar("5aa5", Some(""))]
        );
        let many: Vec<Node> = (0..60u8)
            .map(|i| rule(vec![ref_do(vec![Node::Prim(0xC1, vec![i])])]))
            .collect();
        let got = rules(many).unwrap();
        assert_eq!(got.len(), 60);
        assert_eq!(got[59], ar("3b", None));
    }

    #[test]
    fn access_rule_iteration_stops_at_malformed_elements() {
        // Truncated element after the rule inside BF76: the rule is kept.
        let mut bf76 = rule(vec![ref_do(vec![Node::Prim(0xC1, vec![1])])]).encode();
        bf76.extend([0xE2, 0x09, 0xE1]);
        let mut body = vec![0xBF, 0x76, bf76.len() as u8];
        body.extend(&bf76);
        let mut buf = vec![0xBF, 0x25, body.len() as u8];
        buf.extend(&body);
        assert_eq!(
            parse_metadata(&b64(&buf)).unwrap().access_rules,
            [ar("01", None)]
        );
        // Truncated element inside REF-AR-DO after the REF-DO, and inside REF-DO after C1.
        let e1 = [0xE1, 0x03, 0xC1, 0x01, 0x07];
        let mut e2 = vec![0xE2, (e1.len() + 2) as u8];
        e2.extend(e1);
        e2.extend([0xE1, 0x05]);
        let bf76 = [vec![0xBF, 0x76, e2.len() as u8], e2].concat();
        let buf = [vec![0xBF, 0x25, bf76.len() as u8], bf76].concat();
        assert_eq!(
            parse_metadata(&b64(&buf)).unwrap().access_rules,
            [ar("07", None)]
        );
        let e1 = [0xE1, 0x05, 0xC1, 0x01, 0x07, 0xC1, 0x04];
        let e2 = [vec![0xE2, e1.len() as u8], e1.to_vec()].concat();
        let bf76 = [vec![0xBF, 0x76, e2.len() as u8], e2].concat();
        let buf = [vec![0xBF, 0x25, bf76.len() as u8], bf76].concat();
        assert_eq!(
            parse_metadata(&b64(&buf)).unwrap().access_rules,
            [ar("07", None)],
            "second C1 is malformed, not a duplicate"
        );
    }

    #[test]
    fn a_later_access_rule_list_replaces_the_earlier_one() {
        let list = |b: u8| {
            Node::Cons(
                0xBF76,
                vec![rule(vec![ref_do(vec![Node::Prim(0xC1, vec![b])])])],
            )
        };
        let m = parse(vec![list(1), list(2)]).unwrap();
        assert_eq!(m.access_rules, [ar("02", None)]);
        let m = parse(vec![list(1), Node::Cons(0xBF76, vec![])]).unwrap();
        assert!(m.access_rules.is_empty());
        assert!(parse(vec![list(1), Node::Cons(0xBF76, vec![rule(vec![])])]).is_err());
    }

    #[test]
    fn hostile_input_never_panics() {
        let mut rng = Rng(0xD1B5_4A32_D192_ED03);
        let seed = metadata(vec![
            Node::Prim(0x5A, vec![0x98; 10]),
            Node::Prim(0x91, b"spn".to_vec()),
            Node::Prim(0x93, vec![1]),
            Node::Prim(0x94, vec![1, 2, 3]),
            Node::Cons(
                0xBF76,
                vec![rule(vec![ref_do(vec![
                    Node::Prim(0xC1, vec![1]),
                    Node::Prim(0xCA, b"p".to_vec()),
                ])])],
            ),
        ]);
        for round in 0..20_000 {
            let mut buf = seed.clone();
            match round % 4 {
                0 => {
                    let i = rng.below(buf.len());
                    buf[i] = rng.next() as u8;
                }
                1 => buf.truncate(rng.below(buf.len() + 1)),
                2 => {
                    let i = rng.below(buf.len() + 1);
                    buf.insert(i, rng.next() as u8);
                }
                _ => buf = (0..rng.below(64)).map(|_| rng.next() as u8).collect(),
            }
            let _ = parse_metadata(&b64(&buf));
            let _ = parse_store_metadata(&buf);
        }
        // Lengths that claim more than is there, up to the 4-octet maximum and beyond.
        for raw in [
            &[0xBF, 0x25, 0x84, 0xFF, 0xFF, 0xFF, 0xFF][..],
            &[0xBF, 0x25, 0x88, 0, 0, 0, 0, 0, 0, 0, 1, 0],
            &[0xBF, 0x25, 0x04, 0xBF, 0x76, 0x84, 0xFF],
            &[0xBF, 0x25, 0x06, 0xBF, 0x76, 0x03, 0xE2, 0x01, 0xE1],
        ] {
            let _ = parse_metadata(&b64(raw));
        }
    }
}
