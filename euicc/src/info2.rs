// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2023-2025 ESTKME TECHNOLOGY LIMITED, Hong Kong
// SPDX-FileCopyrightText: 2026 Jesther Silvestre
//
// Rust translation of lpac euicc/es10c_ex.c and euicc/es10c_ex.h (lpac commit c7c8d4f).
// Changed 2026-10-19: rewritten in Rust returning a typed struct.

//! GetEUICCInfo2 (BF22): versions, free memory, capabilities, trusted CI keys.

use crate::der::{self, Node};
use crate::{hex, Apdu, Euicc, Result};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtCardResource {
    pub installed_application: u64,
    pub free_non_volatile_memory: u64,
    pub free_volatile_memory: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CertificationDataObject {
    pub platform_label: Option<String>,
    pub discovery_base_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EuiccInfo2 {
    pub profile_version: Option<String>,
    pub svn: Option<String>,
    pub euicc_firmware_ver: Option<String>,
    pub ext_card_resource: Option<ExtCardResource>,
    pub uicc_capability: Vec<&'static str>,
    pub ts102241_version: Option<String>,
    pub globalplatform_version: Option<String>,
    pub rsp_capability: Vec<&'static str>,
    pub euicc_ci_pkid_list_for_verification: Vec<String>,
    pub euicc_ci_pkid_list_for_signing: Vec<String>,
    pub euicc_category: Option<&'static str>,
    pub forbidden_profile_policy_rules: Vec<&'static str>,
    pub pp_version: Option<String>,
    pub sas_acreditation_number: Option<String>,
    pub certification_data_object: Option<CertificationDataObject>,
}

const UICC_CAPABILITY: &[&str] = &[
    "contactlessSupport",
    "usimSupport",
    "isimSupport",
    "csimSupport",
    "akaMilenage",
    "akaCave",
    "akaTuak128",
    "akaTuak256",
    "rfu1",
    "rfu2",
    "gbaAuthenUsim",
    "gbaAuthenISim",
    "mbmsAuthenUsim",
    "eapClient",
    "javacard",
    "multos",
    "multipleUsimSupport",
    "multipleIsimSupport",
    "multipleCsimSupport",
    "berTlvFileSupport",
    "dfLinkSupport",
    "catTp",
    "getIdentity",
    "profile-a-x25519",
    "profile-b-p256",
    "suciCalculatorApi",
];

const RSP_CAPABILITY: &[&str] = &[
    "additionalProfile",
    "crlSupport",
    "rpmSupport",
    "testProfileSupport",
    "deviceInfoExtensibilitySupport",
];

const FORBIDDEN_PPR: &[&str] = &["pprUpdateControl", "ppr1", "ppr2", "ppr3"];

/// VersionType (3 octets) as "a.b.c".
fn version(value: &[u8]) -> Option<String> {
    match value {
        [a, b, c] => Some(format!("{a}.{b}.{c}")),
        _ => None,
    }
}

fn key_ids(value: &[u8]) -> Vec<String> {
    der::iter(value).map(|t| hex::encode(t.value)).collect()
}

fn text(value: &[u8]) -> String {
    String::from_utf8_lossy(value).into_owned()
}

/// Parses the EUICCInfo2 contents (inside BF22).
pub fn parse_info2(body: &[u8]) -> EuiccInfo2 {
    let mut info = EuiccInfo2::default();
    for t in der::iter(body) {
        match t.tag {
            0x81 => info.profile_version = version(t.value).or(info.profile_version.take()),
            0x82 => info.svn = version(t.value).or(info.svn.take()),
            0x83 => info.euicc_firmware_ver = version(t.value).or(info.euicc_firmware_ver.take()),
            0x84 => {
                let mut r = ExtCardResource::default();
                for c in der::iter(t.value) {
                    match c.tag {
                        0x81 => r.installed_application = der::to_u64(c.value),
                        0x82 => r.free_non_volatile_memory = der::to_u64(c.value),
                        0x83 => r.free_volatile_memory = der::to_u64(c.value),
                        _ => {}
                    }
                }
                info.ext_card_resource = Some(r);
            }
            0x85 => info.uicc_capability = der::bit_names(t.value, UICC_CAPABILITY),
            0x86 => info.ts102241_version = version(t.value).or(info.ts102241_version.take()),
            0x87 => {
                info.globalplatform_version =
                    version(t.value).or(info.globalplatform_version.take())
            }
            0x88 => info.rsp_capability = der::bit_names(t.value, RSP_CAPABILITY),
            0xA9 => info.euicc_ci_pkid_list_for_verification = key_ids(t.value),
            0xAA => info.euicc_ci_pkid_list_for_signing = key_ids(t.value),
            0xAB => {
                info.euicc_category = Some(match der::to_u64(t.value) {
                    1 => "basicEuicc",
                    2 => "mediumEuicc",
                    3 => "contactlessEuicc",
                    _ => "other",
                })
            }
            0x99 => info.forbidden_profile_policy_rules = der::bit_names(t.value, FORBIDDEN_PPR),
            0x04 => info.pp_version = version(t.value).or(info.pp_version.take()),
            0x0C => info.sas_acreditation_number = Some(text(t.value)),
            0xAC => {
                let mut c = CertificationDataObject::default();
                for f in der::iter(t.value) {
                    match f.tag {
                        0x80 => c.platform_label = Some(text(f.value)),
                        0x81 => c.discovery_base_url = Some(text(f.value)),
                        _ => {}
                    }
                }
                info.certification_data_object = Some(c);
            }
            _ => {}
        }
    }
    info
}

impl<T: Apdu> Euicc<T> {
    /// GetEUICCInfo2 (BF22).
    pub fn euicc_info2(&mut self) -> Result<EuiccInfo2> {
        let body = self.request(Node::Prim(0xBF22, Vec::new()))?;
        Ok(parse_info2(&body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_versions_memory_and_ci_keys() {
        let body = [
            Node::Prim(0x81, vec![2, 3, 1]),
            Node::Prim(0x82, vec![2, 2, 2]),
            Node::Prim(0x83, vec![4, 2, 0]),
            Node::Cons(
                0x84,
                vec![
                    Node::Prim(0x81, vec![3]),
                    Node::Prim(0x82, vec![0x0E, 0xD3, 0xE8]),
                    Node::Prim(0x83, vec![0x40, 0xBE]),
                ],
            ),
            Node::Prim(0x85, vec![0x02, 0x7E, 0x00, 0x00]),
            Node::Prim(0x88, vec![0x06, 0x80]),
            Node::Cons(0xA9, vec![Node::Prim(0x04, vec![0x81, 0x37, 0x0F, 0x51])]),
            Node::Cons(0xAA, vec![Node::Prim(0x04, vec![0x81, 0x37, 0x0F, 0x51])]),
            Node::Prim(0xAB, vec![2]),
            Node::Prim(0x0C, b"SAS-123".to_vec()),
        ]
        .iter()
        .flat_map(Node::encode)
        .collect::<Vec<u8>>();
        let info = parse_info2(&body);
        assert_eq!(info.profile_version.as_deref(), Some("2.3.1"));
        assert_eq!(info.svn.as_deref(), Some("2.2.2"));
        assert_eq!(info.euicc_firmware_ver.as_deref(), Some("4.2.0"));
        let r = info.ext_card_resource.unwrap();
        assert_eq!(
            (
                r.installed_application,
                r.free_non_volatile_memory,
                r.free_volatile_memory
            ),
            (3, 971752, 16574)
        );
        assert_eq!(
            info.uicc_capability,
            [
                "usimSupport",
                "isimSupport",
                "csimSupport",
                "akaMilenage",
                "akaCave",
                "akaTuak128"
            ]
        );
        assert_eq!(info.rsp_capability, ["additionalProfile"]);
        assert_eq!(info.euicc_ci_pkid_list_for_verification, ["81370f51"]);
        assert_eq!(info.euicc_category, Some("mediumEuicc"));
        assert_eq!(info.sas_acreditation_number.as_deref(), Some("SAS-123"));
        // A later malformed duplicate does not wipe a good value (libeuicc keeps it).
        let dup = [Node::Prim(0x83, vec![4, 2, 0]), Node::Prim(0x83, vec![9])]
            .iter()
            .flat_map(Node::encode)
            .collect::<Vec<u8>>();
        assert_eq!(
            parse_info2(&dup).euicc_firmware_ver.as_deref(),
            Some("4.2.0")
        );
        assert_eq!(parse_info2(&[]), EuiccInfo2::default());
    }
}
