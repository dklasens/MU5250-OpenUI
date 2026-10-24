// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2023-2025 ESTKME TECHNOLOGY LIMITED, Hong Kong
// SPDX-FileCopyrightText: 2026 Jesther Silvestre
//
// Rust translation of the notification parts of lpac euicc/es10b.c and euicc/es10b.h, and of
// euicc_profilemanagementoperation2str from euicc/tostr.c (lpac commit c7c8d4f).
// Changed 2026-10-19: rewritten in Rust returning typed values; a pending notification is
// returned as its DER bytes (the caller base64-encodes it for ES9+).

//! ES10b notifications: list, retrieve one for sending, remove after delivery.

use crate::der::{self, Node};
use crate::{hex, Apdu, Error, Euicc, Result};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotificationMetadata {
    pub seq_number: u64,
    /// `install`, `enable`, `disable`, `delete` or `unknown`.
    pub profile_management_operation: Option<&'static str>,
    pub notification_address: Option<String>,
    pub iccid: Option<String>,
}

/// A notification ready for ES9+ HandleNotification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingNotification {
    pub notification_address: String,
    /// The whole PendingNotification element (ProfileInstallationResult or
    /// OtherSignedNotification).
    pub der: Vec<u8>,
}

fn operation(value: &[u8]) -> Option<&'static str> {
    // NotificationEvent is a BIT STRING: [unused bits][flags]
    let flags = *value.get(1)?;
    Some(match flags {
        0x80 => "install",
        0x40 => "enable",
        0x20 => "disable",
        0x10 => "delete",
        _ => "unknown",
    })
}

/// Parses a ListNotificationResponse body (inside BF28).
pub fn parse_notifications(body: &[u8]) -> Result<Vec<NotificationMetadata>> {
    let list = der::find(body, 0xA0).ok_or(Error::Malformed("notification list error"))?;
    Ok(der::iter(list.value)
        .filter(|t| t.tag == 0xBF2F)
        .map(|meta| {
            let mut n = NotificationMetadata::default();
            for t in der::iter(meta.value) {
                match t.tag {
                    0x80 => n.seq_number = der::to_u64(t.value),
                    0x81 => n.profile_management_operation = operation(t.value),
                    0x0C => {
                        n.notification_address = Some(String::from_utf8_lossy(t.value).into_owned())
                    }
                    0x5A => n.iccid = Some(hex::bcd_to_iccid(t.value)),
                    _ => {}
                }
            }
            n
        })
        .collect())
}

/// Parses a RetrieveNotificationsListResponse body (inside BF2B).
pub fn parse_pending(body: &[u8]) -> Result<PendingNotification> {
    let list = der::find(body, 0xA0).ok_or(Error::Malformed("no pending notification"))?;
    let pending = der::find_any(list.value, &[0xBF37, 0x30])
        .ok_or(Error::Malformed("no pending notification"))?;
    let meta = match pending.tag {
        // ProfileInstallationResult → ProfileInstallationResultData → NotificationMetadata
        0xBF37 => der::find(pending.value, 0xBF27).and_then(|d| der::find(d.value, 0xBF2F)),
        // OtherSignedNotification → NotificationMetadata
        _ => der::find(pending.value, 0xBF2F),
    }
    .ok_or(Error::Malformed("no notification metadata"))?;
    let address = der::find(meta.value, 0x0C).ok_or(Error::Malformed("no notification address"))?;
    Ok(PendingNotification {
        notification_address: String::from_utf8_lossy(address.value).into_owned(),
        der: pending.raw.to_vec(),
    })
}

impl<T: Apdu> Euicc<T> {
    /// ListNotification (BF28).
    pub fn list_notifications(&mut self) -> Result<Vec<NotificationMetadata>> {
        let body = self.request(Node::Prim(0xBF28, Vec::new()))?;
        parse_notifications(&body)
    }

    /// RetrieveNotificationsList (BF2B) for one sequence number.
    pub fn retrieve_notification(&mut self, seq_number: u64) -> Result<PendingNotification> {
        let body = self.request(Node::Cons(
            0xBF2B,
            vec![Node::Cons(
                0xA0,
                vec![Node::Prim(0x80, der::from_u64(seq_number))],
            )],
        ))?;
        parse_pending(&body)
    }

    /// NotificationSent (BF30): removes a delivered notification. Returns the result code
    /// (0 = ok, 1 = nothing to delete, 127 = undefined error).
    pub fn remove_notification(&mut self, seq_number: u64) -> Result<u64> {
        self.result_code(Node::Cons(
            0xBF30,
            vec![Node::Prim(0x80, der::from_u64(seq_number))],
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Script;

    fn with_sw(node: Node) -> Vec<u8> {
        let mut v = node.encode();
        v.extend([0x90, 0x00]);
        v
    }

    fn metadata(seq: u64, op: u8, addr: &str) -> Node {
        Node::Cons(
            0xBF2F,
            vec![
                Node::Prim(0x80, der::from_u64(seq)),
                Node::Prim(0x81, vec![0x07, op]),
                Node::Prim(0x0C, addr.as_bytes().to_vec()),
                Node::Prim(0x5A, hex::iccid_to_bcd("8963000000000000001", 10).unwrap()),
            ],
        )
    }

    #[test]
    fn lists_notifications() {
        let resp = with_sw(Node::Cons(
            0xBF28,
            vec![Node::Cons(
                0xA0,
                vec![
                    metadata(128, 0x40, "smdp.example.com"),
                    metadata(129, 0x20, "rsp.example.net"),
                ],
            )],
        ));
        let mut card = Script::new(1, &[&resp]);
        let list = Euicc::open(&mut card)
            .unwrap()
            .list_notifications()
            .unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].seq_number, 128);
        assert_eq!(list[0].profile_management_operation, Some("enable"));
        assert_eq!(
            list[0].notification_address.as_deref(),
            Some("smdp.example.com")
        );
        assert_eq!(list[0].iccid.as_deref(), Some("8963000000000000001"));
        assert_eq!(list[1].profile_management_operation, Some("disable"));
    }

    #[test]
    fn retrieves_a_signed_notification_whole() {
        let other = Node::Cons(
            0x30,
            vec![
                metadata(7, 0x10, "smdp.example.com"),
                Node::Prim(0x5F37, vec![0xAA; 64]),
            ],
        );
        let raw = other.encode();
        let resp = with_sw(Node::Cons(0xBF2B, vec![Node::Cons(0xA0, vec![other])]));
        let mut card = Script::new(
            1,
            &[&resp, &[0xBF, 0x30, 0x03, 0x80, 0x01, 0x00, 0x90, 0x00]],
        );
        let mut e = Euicc::open(&mut card).unwrap();
        let p = e.retrieve_notification(7).unwrap();
        assert_eq!(p.notification_address, "smdp.example.com");
        assert_eq!(
            p.der, raw,
            "sent to the server exactly as the card produced it"
        );
        assert_eq!(e.remove_notification(7).unwrap(), 0);
        drop(e);
        assert_eq!(
            &card.sent[0][5..],
            &[0xBF, 0x2B, 0x05, 0xA0, 0x03, 0x80, 0x01, 0x07]
        );
        assert_eq!(&card.sent[1][5..], &[0xBF, 0x30, 0x03, 0x80, 0x01, 0x07]);
    }

    #[test]
    fn installation_results_carry_their_metadata_deeper() {
        let result = Node::Cons(
            0xBF37,
            vec![Node::Cons(
                0xBF27,
                vec![
                    Node::Prim(0x80, vec![1]),
                    metadata(9, 0x80, "dp.example.org"),
                ],
            )],
        );
        let body = Node::Cons(0xA0, vec![result.clone()]).encode();
        let p = parse_pending(&body).unwrap();
        assert_eq!(p.notification_address, "dp.example.org");
        assert_eq!(p.der, result.encode());
    }
}
