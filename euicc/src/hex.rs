// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2023-2025 ESTKME TECHNOLOGY LIMITED, Hong Kong
// SPDX-FileCopyrightText: 2026 Jesther Silvestre
//
// Rust translation of lpac euicc/hexutil.c (lpac commit c7c8d4f).
// Changed 2026-10-19: rewritten in Rust with owned Strings/Vecs instead of caller buffers.

//! Hex and the swapped-nibble BCD that SIM cards use for ICCIDs.

pub fn encode(bin: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bin.len() * 2);
    for b in bin {
        out.push(DIGITS[usize::from(b >> 4)] as char);
        out.push(DIGITS[usize::from(b & 0xF)] as char);
    }
    out
}

pub fn decode(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    hex.as_bytes()
        .chunks(2)
        .map(|pair| {
            let digit = |c: u8| (c as char).to_digit(16);
            Some((digit(pair[0])? << 4 | digit(pair[1])?) as u8)
        })
        .collect()
}

/// ICCID digits to card bytes: pad an odd count with `f`, swap each digit pair, then pad with
/// 0xFF to `padding_to` bytes. More than `padding_to` bytes is refused, as libeuicc's fixed
/// output buffer does.
pub fn iccid_to_bcd(digits: &str, padding_to: usize) -> Option<Vec<u8>> {
    let mut chars: Vec<u8> = digits.bytes().collect();
    if !chars.len().is_multiple_of(2) {
        chars.push(b'f');
    }
    for pair in chars.chunks_mut(2) {
        pair.swap(0, 1);
    }
    let mut out = decode(std::str::from_utf8(&chars).ok()?)?;
    if out.len() > padding_to {
        return None;
    }
    if out.len() < padding_to {
        out.resize(padding_to, 0xFF);
    }
    Some(out)
}

/// Card bytes to ICCID digits: swap each digit pair and drop the trailing `f` padding (the first
/// character always stays, as in libeuicc).
pub fn bcd_to_iccid(bin: &[u8]) -> String {
    let mut chars: Vec<u8> = encode(bin).into_bytes();
    for pair in chars.chunks_mut(2) {
        pair.swap(0, 1);
    }
    while chars.len() > 1 && chars.last() == Some(&b'f') {
        chars.pop();
    }
    String::from_utf8(chars).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        assert_eq!(encode(&[0x00, 0xAB, 0x5A]), "00ab5a");
        assert_eq!(decode("00Ab5a").unwrap(), [0x00, 0xAB, 0x5A]);
        assert!(decode("abc").is_none());
        assert!(decode("zz").is_none());
    }

    #[test]
    fn iccids_swap_nibbles_and_pad() {
        // 19 digits: padded with f, then each digit pair swapped ("1f" -> "f1")
        let bcd = iccid_to_bcd("8963000000000000001", 10).unwrap();
        assert_eq!(encode(&bcd), "983600000000000000f1");
        assert_eq!(bcd_to_iccid(&bcd), "8963000000000000001");
        let twenty = iccid_to_bcd("89630000000000000027", 10).unwrap();
        assert_eq!(bcd_to_iccid(&twenty), "89630000000000000027");
        assert_eq!(iccid_to_bcd("12", 4).unwrap(), [0x21, 0xFF, 0xFF, 0xFF]);
        assert!(
            iccid_to_bcd("896300000000000000271", 10).is_none(),
            "21 digits do not fit 10 bytes"
        );
        assert_eq!(
            bcd_to_iccid(&[0xFF; 10]),
            "f",
            "libeuicc keeps the first character"
        );
        assert!(iccid_to_bcd("12x4", 2).is_none());
    }
}
