// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2023-2025 ESTKME TECHNOLOGY LIMITED, Hong Kong
// SPDX-FileCopyrightText: 2026 Jesther Silvestre
//
// Rust translation of euicc_base64_decode and euicc_base64_encode from lpac euicc/base64.c
// (lpac commit c7c8d4f).
// Changed 2026-10-19: rewritten in Rust; encoding uses the base64 crate's standard engine, which
// produces the same text.

//! Base64 as libeuicc reads and writes it.

use base64::Engine;

fn sextet(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// `euicc_base64_decode`: only the leading run of standard-alphabet characters counts, and the
/// first other character ends the input (`=` padding, whitespace, the URL-safe `-`/`_`, NUL...).
/// A final group of one character yields nothing and unused low bits are not checked. This
/// never fails.
pub fn decode(text: &str) -> Vec<u8> {
    let sextets: Vec<u8> = text.bytes().map_while(sextet).collect();
    let mut out = Vec::with_capacity(sextets.len() / 4 * 3 + 2);
    for group in sextets.chunks(4) {
        if let [a, b, ..] = group {
            out.push(a << 2 | b >> 4);
        }
        if let [_, b, c, ..] = group {
            out.push(b << 4 | c >> 2);
        }
        if let [_, _, c, d] = group {
            out.push(c << 6 | d);
        }
    }
    out
}

/// `euicc_base64_encode`: standard alphabet with padding.
pub fn encode(bin: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bin)
}
