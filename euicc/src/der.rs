// SPDX-License-Identifier: LGPL-2.1-only
// SPDX-FileCopyrightText: 2023-2025 ESTKME TECHNOLOGY LIMITED, Hong Kong
// SPDX-FileCopyrightText: 2026 Jesther Silvestre
//
// Rust translation of lpac euicc/derutil.c (lpac commit c7c8d4f).
// Changed 2026-10-19: rewritten in Rust. Parsing returns borrowed slices instead of filling
// node structs; encoding builds a `Node` tree into a Vec instead of packing in place; long-form
// lengths over 4 bytes are rejected instead of overflowing.

//! Minimal BER-TLV / DER as used by SGP.22 ES10: one- or two-byte tags, definite lengths.

/// One parsed element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tlv<'a> {
    pub tag: u16,
    /// Contents (without tag and length).
    pub value: &'a [u8],
    /// The whole element: tag, length and contents.
    pub raw: &'a [u8],
}

/// Parses the element at the start of `buf`.
pub fn parse(buf: &[u8]) -> Option<Tlv<'_>> {
    let mut i = 0;
    let mut tag = u16::from(*buf.get(i)?);
    i += 1;
    if tag & 0x1F == 0x1F {
        tag = (tag << 8) | u16::from(*buf.get(i)?);
        i += 1;
    }
    let first = *buf.get(i)?;
    i += 1;
    let length = if first & 0x80 != 0 {
        let count = usize::from(first & 0x7F);
        if count > 4 {
            return None;
        }
        let mut length = 0usize;
        for _ in 0..count {
            length = (length << 8) | usize::from(*buf.get(i)?);
            i += 1;
        }
        length
    } else {
        usize::from(first)
    };
    let end = i.checked_add(length)?;
    let value = buf.get(i..end)?;
    Some(Tlv {
        tag,
        value,
        raw: &buf[..end],
    })
}

/// The elements in `buf`, one after another. Stops at the first malformed one, like
/// `euicc_derutil_unpack_next` loops.
pub fn iter(buf: &[u8]) -> Iter<'_> {
    Iter { rest: buf }
}

pub struct Iter<'a> {
    rest: &'a [u8],
}

impl<'a> Iterator for Iter<'a> {
    type Item = Tlv<'a>;

    fn next(&mut self) -> Option<Tlv<'a>> {
        let tlv = parse(self.rest)?;
        self.rest = &self.rest[tlv.raw.len()..];
        Some(tlv)
    }
}

/// First element in `buf` with `tag`.
pub fn find(buf: &[u8], tag: u16) -> Option<Tlv<'_>> {
    iter(buf).find(|t| t.tag == tag)
}

/// First element in `buf` whose tag is one of `tags`.
pub fn find_any<'a>(buf: &'a [u8], tags: &[u16]) -> Option<Tlv<'a>> {
    iter(buf).find(|t| tags.contains(&t.tag))
}

/// An element to encode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// Tag with literal contents.
    Prim(u16, Vec<u8>),
    /// Tag whose contents are the encoded children.
    Cons(u16, Vec<Node>),
}

impl Node {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(&mut out);
        out
    }

    fn encode_into(&self, out: &mut Vec<u8>) {
        let (tag, value) = match self {
            Node::Prim(tag, value) => (*tag, value.clone()),
            Node::Cons(tag, children) => {
                let mut value = Vec::new();
                for child in children {
                    child.encode_into(&mut value);
                }
                (*tag, value)
            }
        };
        if tag >> 8 != 0 {
            out.push((tag >> 8) as u8);
        }
        out.push(tag as u8);
        let len = value.len();
        if len < 0x80 {
            out.push(len as u8);
        } else {
            let bytes = (len as u32).to_be_bytes();
            let skip = bytes.iter().take_while(|b| **b == 0).count();
            out.push(0x80 | (4 - skip) as u8);
            out.extend_from_slice(&bytes[skip..]);
        }
        out.extend_from_slice(&value);
    }
}

/// Big-endian unsigned integer contents.
pub fn to_u64(value: &[u8]) -> u64 {
    value.iter().fold(0u64, |acc, b| (acc << 8) | u64::from(*b))
}

/// Minimal INTEGER contents for a non-negative value (a leading zero keeps the sign bit clear).
pub fn from_u64(value: u64) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    let skip = bytes.iter().take_while(|b| **b == 0).count().min(7);
    let mut out = bytes[skip..].to_vec();
    if out[0] & 0x80 != 0 {
        out.insert(0, 0);
    }
    out
}

/// Names of the bits set in a BIT STRING (first byte: unused bits), in `names` order.
pub fn bit_names(value: &[u8], names: &[&'static str]) -> Vec<&'static str> {
    let Some((&unused, bits)) = value.split_first() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (j, byte) in bits.iter().enumerate() {
        let mut byte = u32::from(*byte);
        if j == bits.len() - 1 {
            byte &= !(0xFFu32 >> (8 - u32::from(unused.min(8))));
        }
        for i in 0..8 {
            let index = j * 8 + i;
            if index >= names.len() {
                break;
            }
            if byte & 0x80 != 0 {
                out.push(names[index]);
            }
            byte <<= 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_short_long_and_two_byte_tags() {
        let t = parse(&[0x5A, 0x02, 0x89, 0x01, 0xFF]).unwrap();
        assert_eq!((t.tag, t.value, t.raw.len()), (0x5A, &[0x89, 0x01][..], 4));
        let t = parse(&[0xBF, 0x2D, 0x00]).unwrap();
        assert_eq!((t.tag, t.value.len()), (0xBF2D, 0));
        let mut long = vec![0x04, 0x81, 0x80];
        long.extend(vec![7u8; 0x80]);
        assert_eq!(parse(&long).unwrap().value.len(), 0x80);
        assert!(parse(&[0x04, 0x05, 1, 2]).is_none(), "truncated");
        assert!(
            parse(&[0x04, 0x85, 1, 1, 1, 1, 1]).is_none(),
            "length over 4 bytes"
        );
        assert!(parse(&[0xBF]).is_none());
    }

    #[test]
    fn iterates_siblings_and_finds_tags() {
        let buf = [0x80, 0x01, 0x05, 0x0C, 0x02, b'a', b'b', 0x5A, 0x00];
        let tags: Vec<u16> = iter(&buf).map(|t| t.tag).collect();
        assert_eq!(tags, [0x80, 0x0C, 0x5A]);
        assert_eq!(find(&buf, 0x0C).unwrap().value, b"ab");
        assert_eq!(find_any(&buf, &[0x99, 0x5A]).unwrap().tag, 0x5A);
        assert!(find(&buf, 0x99).is_none());
    }

    #[test]
    fn encodes_nested_requests() {
        // EnableProfileRequest with refresh: BF31 { A0 { 5A iccid } 81 FF }
        let req = Node::Cons(
            0xBF31,
            vec![
                Node::Cons(0xA0, vec![Node::Prim(0x5A, vec![0x98, 0x10])]),
                Node::Prim(0x81, vec![0xFF]),
            ],
        );
        assert_eq!(
            req.encode(),
            [0xBF, 0x31, 0x09, 0xA0, 0x04, 0x5A, 0x02, 0x98, 0x10, 0x81, 0x01, 0xFF]
        );
        assert_eq!(Node::Prim(0xBF2D, vec![]).encode(), [0xBF, 0x2D, 0x00]);
        let big = Node::Prim(0x04, vec![0; 300]).encode();
        assert_eq!(&big[..4], &[0x04, 0x82, 0x01, 0x2C]);
        assert_eq!(parse(&big).unwrap().value.len(), 300);
    }

    #[test]
    fn integers_round_trip() {
        assert_eq!(from_u64(0), [0]);
        assert_eq!(from_u64(5), [5]);
        assert_eq!(from_u64(0x80), [0, 0x80]);
        assert_eq!(from_u64(300), [0x01, 0x2C]);
        for v in [0u64, 1, 127, 128, 255, 256, 65535, 1 << 40] {
            assert_eq!(to_u64(&from_u64(v)), v);
        }
    }

    #[test]
    fn bit_strings_name_the_set_bits() {
        let names = ["a", "b", "c", "d", "e", "f", "g", "h", "i"];
        // 0b1010_0000, 0b1000_0000 with 7 unused bits in the last byte
        assert_eq!(bit_names(&[7, 0xA0, 0x80], &names), ["a", "c", "i"]);
        assert_eq!(bit_names(&[0, 0xFF], &names[..3]), ["a", "b", "c"]);
        assert!(bit_names(&[], &names).is_empty());
    }
}
