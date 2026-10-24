// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2023-2025 ESTKME TECHNOLOGY LIMITED, Hong Kong
// SPDX-FileCopyrightText: 2026 Jesther Silvestre
//
// Rust translation of lpac euicc/euicc.c, euicc/interface.c and euicc/interface.h
// (lpac commit c7c8d4f).
// Changed 2026-10-19: rewritten in Rust. The APDU interface is a trait; the session owns the
// transport and closes its logical channel on drop; logical channels 4-19 get the ISO 7816-4
// further-interindustry class byte instead of OR-ing the channel into the low nibble.

//! SGP.22 eUICC card commands (ES10a/b/c): a Rust translation of
//! [lpac](https://github.com/estkme-group/lpac)'s libeuicc.
//!
//! This crate is licensed LGPL-2.1-only, like the library it was translated from. The
//! transport (how APDUs reach the card) is supplied by the caller through [`Apdu`].

pub mod b64;
pub mod der;
pub mod download;
pub mod es10b;
pub mod es10c;
pub mod es8p;
pub mod es9p;
pub mod hex;
pub mod info2;
pub mod rsp;

use std::fmt;

/// AID of the eUICC's ISD-R (the profile management application).
pub const ISD_R_AID: [u8; 16] = [
    0xA0, 0x00, 0x00, 0x05, 0x59, 0x10, 0x10, 0xFF, 0xFF, 0xFF, 0xFF, 0x89, 0x00, 0x00, 0x01, 0x00,
];

/// Largest ES10 STORE DATA block, as in libeuicc.
const MSS: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The transport failed (no card, modem error, timeout).
    Transport(String),
    /// The card answered with an error status word.
    Status(u8, u8),
    /// The card's answer was not the expected structure.
    Malformed(&'static str),
    /// A caller argument could not be encoded (e.g. not an ICCID).
    Argument(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Transport(m) => f.write_str(m),
            Error::Status(sw1, sw2) => write!(f, "card status {sw1:02X}{sw2:02X}"),
            Error::Malformed(what) => write!(f, "unexpected card response ({what})"),
            Error::Argument(what) => write!(f, "invalid argument ({what})"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// How APDUs reach the card (QMI, PC/SC, AT...).
pub trait Apdu {
    /// Opens a logical channel and selects `aid` on it; returns the channel number.
    fn open_channel(&mut self, aid: &[u8]) -> Result<u8>;
    /// Closes a channel opened by `open_channel`.
    fn close_channel(&mut self, channel: u8);
    /// Sends one command APDU; returns the response data followed by SW1 SW2.
    fn transmit(&mut self, apdu: &[u8]) -> Result<Vec<u8>>;
}

/// An open session with the ISD-R. Dropping it closes the logical channel.
pub struct Euicc<T: Apdu> {
    io: T,
    channel: u8,
}

impl<T: Apdu> Euicc<T> {
    /// Opens a logical channel to the ISD-R (`euicc_init`).
    pub fn open(mut io: T) -> Result<Self> {
        let channel = io.open_channel(&ISD_R_AID)?;
        Ok(Euicc { io, channel })
    }

    fn class(&self, base: u8) -> u8 {
        if self.channel <= 3 {
            (base & 0xFC) | self.channel
        } else {
            (base & 0xB0) | 0x40 | ((self.channel - 4) & 0x0F)
        }
    }

    /// Sends one APDU and collects the response data, following `61xx` with GET RESPONSE.
    fn transmit_collect(&mut self, apdu: Vec<u8>, out: &mut Vec<u8>) -> Result<()> {
        let mut apdu = apdu;
        loop {
            let mut rx = self.io.transmit(&apdu)?;
            if rx.len() < 2 {
                return Err(Error::Malformed("response shorter than a status word"));
            }
            let sw2 = rx.pop().unwrap_or(0);
            let sw1 = rx.pop().unwrap_or(0);
            out.extend_from_slice(&rx);
            match sw1 {
                0x61 => apdu = vec![self.class(0x80), 0xC0, 0x00, 0x00, sw2],
                _ if sw1 & 0xF0 == 0x90 => return Ok(()),
                _ => return Err(Error::Status(sw1, sw2)),
            }
        }
    }

    /// Sends an ES10 request (DER) as STORE DATA blocks and returns the whole response
    /// (`es10x_command`).
    pub fn command(&mut self, der: &[u8]) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let blocks: Vec<&[u8]> = der.chunks(MSS).collect();
        let last = blocks.len().saturating_sub(1);
        for (seq, block) in blocks.iter().enumerate() {
            let p1 = if seq == last { 0x91 } else { 0x11 };
            let mut apdu = vec![
                self.class(0x80),
                0xE2,
                p1,
                (seq & 0xFF) as u8,
                block.len() as u8,
            ];
            apdu.extend_from_slice(block);
            self.transmit_collect(apdu, &mut out)?;
        }
        Ok(out)
    }

    /// The contents of the response element with the request's tag.
    pub(crate) fn request(&mut self, req: der::Node) -> Result<Vec<u8>> {
        let tag = match &req {
            der::Node::Prim(tag, _) | der::Node::Cons(tag, _) => *tag,
        };
        let resp = self.command(&req.encode())?;
        der::find(&resp, tag)
            .map(|t| t.value.to_vec())
            .ok_or(Error::Malformed("response tag differs from the request"))
    }

    /// The `[0]` result code inside a response, as for Enable/Disable/Delete/SetNickname.
    pub(crate) fn result_code(&mut self, req: der::Node) -> Result<u64> {
        let body = self.request(req)?;
        der::find(&body, 0x80)
            .map(|t| der::to_u64(t.value))
            .ok_or(Error::Malformed("no result code"))
    }
}

impl<T: Apdu> Drop for Euicc<T> {
    fn drop(&mut self) {
        self.io.close_channel(self.channel);
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::collections::VecDeque;

    /// Replays canned card responses and records what was sent.
    #[derive(Default)]
    pub struct Script {
        pub sent: Vec<Vec<u8>>,
        pub responses: VecDeque<Vec<u8>>,
        pub closed: Vec<u8>,
        pub channel: u8,
    }

    impl Script {
        pub fn new(channel: u8, responses: &[&[u8]]) -> Self {
            Script {
                channel,
                responses: responses.iter().map(|r| r.to_vec()).collect(),
                ..Default::default()
            }
        }
    }

    impl Apdu for &mut Script {
        fn open_channel(&mut self, aid: &[u8]) -> Result<u8> {
            assert_eq!(aid, ISD_R_AID);
            Ok(self.channel)
        }
        fn close_channel(&mut self, channel: u8) {
            self.closed.push(channel);
        }
        fn transmit(&mut self, apdu: &[u8]) -> Result<Vec<u8>> {
            self.sent.push(apdu.to_vec());
            self.responses
                .pop_front()
                .ok_or(Error::Transport("no scripted response".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::Script;
    use super::*;

    #[test]
    fn long_requests_are_split_and_61xx_is_followed() {
        let mut card = Script::new(
            1,
            &[
                &[0x90, 0x00],
                &[0xBF, 0x2D, 0x61, 0x03],
                &[0x02, 0x80, 0x00, 0x90, 0x00],
            ],
        );
        let req = vec![0xAB; 130];
        {
            let mut e = Euicc::open(&mut card).unwrap();
            assert_eq!(e.command(&req).unwrap(), [0xBF, 0x2D, 0x02, 0x80, 0x00]);
        }
        assert_eq!(card.sent.len(), 3);
        assert_eq!(
            &card.sent[0][..5],
            &[0x81, 0xE2, 0x11, 0x00, 120],
            "first block, more to come"
        );
        assert_eq!(
            &card.sent[1][..5],
            &[0x81, 0xE2, 0x91, 0x01, 10],
            "last block"
        );
        assert_eq!(
            card.sent[2],
            [0x81, 0xC0, 0x00, 0x00, 0x03],
            "GET RESPONSE for 61 03"
        );
        assert_eq!(card.closed, [1], "channel closed on drop");
    }

    #[test]
    fn error_status_words_fail() {
        let mut card = Script::new(2, &[&[0x6A, 0x88]]);
        let mut e = Euicc::open(&mut card).unwrap();
        assert_eq!(
            e.command(&[0xBF, 0x2D, 0x00]),
            Err(Error::Status(0x6A, 0x88))
        );
    }

    #[test]
    fn extended_channels_use_the_further_class_byte() {
        let mut card = Script::new(5, &[&[0x90, 0x00]]);
        let mut e = Euicc::open(&mut card).unwrap();
        e.command(&[0xBF, 0x2D, 0x00]).unwrap();
        drop(e);
        assert_eq!(card.sent[0][0], 0xC1);
    }
}
