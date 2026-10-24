//! ES9+ HTTPS for operator receipts: one POST per connection over rustls, trusting only the GSMA
//! eSIM root CI. No redirects, no proxy, no connection reuse, and only public addresses.
//!
//! A web client library would load the root as an ordinary certificate, which rustls refuses (its
//! certificatePolicies extension is critical), and would not let the resolved addresses be vetted
//! before connecting; hence this small client.

use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustls::pki_types::{ServerName, TrustAnchor};

/// GSMA RSP2 Root CI1, the CI every SM-DP+ TLS certificate on the tested card chains to.
const GSMA_CI1_PEM: &str = include_str!("../certs/gsma-rsp2-root-ci1.pem");
/// Its key ID (SHA-1 of the public key bits), as cards list it in `euiccCiPKIdListForVerification`.
pub const GSMA_CI1_KEY_ID: &str = "81370f5125d0b1d408d4c3b232e6d25e795bebfb";
/// Largest answer read for receipts and the authentication steps; HandleNotification answers 204
/// with no body.
pub const SMALL_RESPONSE: usize = 64 * 1024;
/// A bound profile package is tens of kilobytes; this leaves room for large profiles.
pub const PACKAGE_RESPONSE: usize = 8 << 20;

/// The root as a trust anchor, built from its subject and key.
fn gsma_anchor() -> Result<TrustAnchor<'static>, String> {
    let b64: String = GSMA_CI1_PEM
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect();
    let der = euicc::b64::decode(b64.trim());
    let cert = euicc::der::parse(&der)
        .filter(|t| t.tag == 0x30)
        .ok_or("GSMA root: not a certificate")?;
    let tbs = euicc::der::parse(cert.value)
        .filter(|t| t.tag == 0x30)
        .ok_or("GSMA root: no tbsCertificate")?;
    let mut fields = euicc::der::iter(tbs.value).skip_while(|t| t.tag == 0xA0);
    // serialNumber, signature, issuer, validity, subject, subjectPublicKeyInfo
    let subject = fields
        .nth(4)
        .filter(|t| t.tag == 0x30)
        .ok_or("GSMA root: no subject")?;
    let spki = fields
        .next()
        .filter(|t| t.tag == 0x30)
        .ok_or("GSMA root: no public key")?;
    Ok(TrustAnchor {
        subject: subject.value.to_vec().into(),
        subject_public_key_info: spki.value.to_vec().into(),
        name_constraints: None,
    })
}

fn tls_config() -> Result<Arc<rustls::ClientConfig>, String> {
    let mut roots = rustls::RootCertStore::empty();
    roots.roots.push(gsma_anchor()?);
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| e.to_string())?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(Arc::new(config))
}

/// A server address from the card: a DNS host name, optionally with a port. IP literals are
/// refused; the card has no reason to name a server by address.
pub fn valid_address(address: &str) -> bool {
    let (host, port) = match address.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (address, None),
    };
    if port.is_some_and(|p| p.parse::<u16>().map_or(true, |p| p == 0)) {
        return false;
    }
    let labels: Vec<&str> = host.split('.').collect();
    !host.is_empty()
        && host.len() <= 253
        && labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        // A numeric last label means an IPv4 literal (or nonsense).
        && !labels.last().is_some_and(|l| l.bytes().all(|b| b.is_ascii_digit()))
}

/// Addresses an operator server may legitimately have: no loopback, private, link-local, shared,
/// multicast or reserved ranges, so a card cannot point the router at its own LAN.
pub fn public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_unspecified()
                || v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_broadcast()
                || v4.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1]))
                || (o[0] == 198 && (18..20).contains(&o[1]))
                || o[0] >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public_address(IpAddr::V4(v4));
            }
            let s = v6.segments();
            !(v6.is_unspecified()
                || v6.is_loopback()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] == 0x2001 && s[1] == 0x0db8))
        }
    }
}

/// `euicc::es9p::Http` over [`post`]; each request gets `timeout`, capped by `deadline`, and may
/// answer with at most `max_response` bytes.
pub struct Es9pHttp {
    pub timeout: Duration,
    pub deadline: Instant,
    pub max_response: usize,
}

impl euicc::es9p::Http for Es9pHttp {
    fn post(&mut self, url: &str, headers: &[&str], body: &[u8]) -> Result<(u32, Vec<u8>), String> {
        let until = (Instant::now() + self.timeout).min(self.deadline);
        post(url, headers, body, until, self.max_response)
    }
}

fn post(
    url: &str,
    headers: &[&str],
    body: &[u8],
    until: Instant,
    max_response: usize,
) -> Result<(u32, Vec<u8>), String> {
    let rest = url.strip_prefix("https://").ok_or("not an HTTPS URL")?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    if !valid_address(authority) {
        return Err("invalid server address".into());
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().map_err(|_| "invalid port")?),
        None => (authority, 443),
    };
    let addrs: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("resolving {host}: {e}"))?
        .collect();
    let addr = addrs
        .iter()
        .copied()
        .find(|a| public_address(a.ip()))
        .ok_or_else(|| format!("{host} has no public address"))?;
    if crate::cancelled() {
        return Err("cancelled".into());
    }
    let left = |until: Instant| {
        let d = until.saturating_duration_since(Instant::now());
        if d.is_zero() {
            Err("timed out".to_string())
        } else {
            Ok(d)
        }
    };
    let tcp = TcpStream::connect_timeout(&addr, left(until)?).map_err(|e| e.to_string())?;
    let name = ServerName::try_from(host.to_string()).map_err(|e| e.to_string())?;
    let conn = rustls::ClientConnection::new(tls_config()?, name).map_err(|e| e.to_string())?;
    let mut tls = rustls::StreamOwned::new(conn, tcp);

    let mut request = format!("POST {path} HTTP/1.1\r\nHost: {authority}\r\n");
    for h in headers {
        request.push_str(h);
        request.push_str("\r\n");
    }
    request.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    let mut wire = request.into_bytes();
    wire.extend_from_slice(body);
    tls.sock
        .set_write_timeout(Some(left(until)?))
        .map_err(|e| e.to_string())?;
    tls.write_all(&wire).map_err(|e| e.to_string())?;
    tls.flush().map_err(|e| e.to_string())?;

    let mut answer = Vec::new();
    let mut buf = [0u8; 16384];
    loop {
        if crate::cancelled() {
            return Err("cancelled".into());
        }
        // Short read slices keep the deadline and a cancellation responsive.
        let slice = left(until)?.min(Duration::from_millis(500));
        tls.sock
            .set_read_timeout(Some(slice))
            .map_err(|e| e.to_string())?;
        match tls.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if answer.len() + n > max_response {
                    return Err("answer too large".into());
                }
                answer.extend_from_slice(&buf[..n]);
                if response_complete(&answer) {
                    break;
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            // Servers that close without close_notify: the status line is what matters.
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof && head_end(&answer).is_some() => {
                break
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    parse_response(&answer)
}

fn head_end(answer: &[u8]) -> Option<usize> {
    answer
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| i + 4)
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().skip(1).find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then_some(v.trim())
    })
}

fn is_chunked(head: &str) -> bool {
    header(head, "transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"))
}

/// The body of a chunked answer, once its last chunk has arrived (`None` until then).
/// Trailers are skipped.
pub fn decode_chunked(mut body: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let line_end = body.windows(2).position(|w| w == b"\r\n")?;
        let size_text = std::str::from_utf8(&body[..line_end]).ok()?;
        let size = usize::from_str_radix(size_text.split(';').next()?.trim(), 16).ok()?;
        body = &body[line_end + 2..];
        if size == 0 {
            // Optional trailers, then the blank line that ends the message.
            return body.windows(2).position(|w| w == b"\r\n").map(|_| out);
        }
        if body.len() < size + 2 {
            return None;
        }
        out.extend_from_slice(&body[..size]);
        body = &body[size + 2..];
    }
}

/// The whole answer is in when its head says how long the body is and that much has arrived, or
/// a chunked body's last chunk has.
fn response_complete(answer: &[u8]) -> bool {
    let Some(end) = head_end(answer) else {
        return false;
    };
    let head = String::from_utf8_lossy(&answer[..end]);
    let status = head.to_ascii_lowercase();
    if status.starts_with("http/1.1 204") || status.starts_with("http/1.0 204") {
        return true;
    }
    if is_chunked(&head) {
        return decode_chunked(&answer[end..]).is_some();
    }
    header(&head, "content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .is_some_and(|len| answer.len() >= end + len)
}

/// Status and body of an HTTP/1.x answer. Redirects are not followed: any 3xx is returned as is
/// and counts as a failure.
pub fn parse_response(answer: &[u8]) -> Result<(u32, Vec<u8>), String> {
    let end = head_end(answer).ok_or("incomplete HTTP answer")?;
    let head = std::str::from_utf8(&answer[..end]).map_err(|_| "HTTP head is not text")?;
    let status_line = head.lines().next().unwrap_or_default();
    let mut parts = status_line.split_whitespace();
    if !parts.next().is_some_and(|v| v.starts_with("HTTP/1.")) {
        return Err("not an HTTP/1.x answer".into());
    }
    let code = parts
        .next()
        .and_then(|c| c.parse::<u32>().ok())
        .filter(|c| (100..600).contains(c))
        .ok_or("no HTTP status")?;
    let body = &answer[end..];
    if is_chunked(head) {
        return decode_chunked(body)
            .map(|b| (code, b))
            .ok_or_else(|| "incomplete chunked answer".to_string());
    }
    let len = header(head, "content-length").and_then(|v| v.parse::<usize>().ok());
    Ok((
        code,
        body[..len.unwrap_or(body.len()).min(body.len())].to_vec(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_addresses_are_dns_names_only() {
        assert!(valid_address("rsp-3110.idemia.io"));
        assert!(valid_address("SM-V4-012-A-GTM.PR.GO-ESIM.COM"));
        assert!(valid_address("smdp.example.com:8443"));
        for bad in [
            "",
            "localhost",
            "192.168.0.1",
            "10.0.0.1:443",
            "[::1]",
            "::1",
            "evil.example/path",
            "evil.example?x",
            "-bad.example.com",
            "smdp.example.com:0",
            "smdp.example.com:99999",
            "a..example.com",
        ] {
            assert!(!valid_address(bad), "{bad:?}");
        }
    }

    #[test]
    fn only_public_addresses_are_dialled() {
        for ip in ["8.8.8.8", "203.0.114.5", "2606:4700::1111"] {
            assert!(public_address(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "127.0.0.1",
            "192.168.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "::1",
            "fe80::1",
            "fd00::1",
            "::ffff:192.168.0.1",
            "2001:db8::1",
        ] {
            assert!(!public_address(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn responses_parse_and_redirects_are_not_successes() {
        assert_eq!(
            parse_response(b"HTTP/1.1 204 No Content\r\nX: y\r\n\r\n").unwrap(),
            (204, vec![])
        );
        let (code, body) =
            parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}").unwrap();
        assert_eq!((code, body.as_slice()), (200, &b"{}"[..]));
        assert_eq!(
            parse_response(b"HTTP/1.1 302 Found\r\nLocation: x\r\n\r\n")
                .unwrap()
                .0,
            302
        );
        assert!(parse_response(b"HTTP/1.1 204").is_err());
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"a\"\r\n3;ext=1\r\n:1}\r\n0\r\n\r\n";
        assert!(response_complete(chunked));
        assert_eq!(parse_response(chunked).unwrap().1, br#"{"a":1}"#);
        assert!(!response_complete(&chunked[..chunked.len() - 2]));
        assert!(parse_response(&chunked[..chunked.len() - 7]).is_err());
        assert_eq!(decode_chunked(b"0\r\nX-Trailer: y\r\n\r\n"), Some(vec![]));
        assert!(parse_response(b"SSH-2.0\r\n\r\n").is_err());
        assert!(response_complete(b"HTTP/1.1 204 No Content\r\n\r\n"));
        assert!(!response_complete(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nab"
        ));
    }

    #[test]
    fn the_gsma_root_becomes_a_trust_anchor_with_the_cards_key_id() {
        let anchor = gsma_anchor().unwrap();
        assert!(anchor
            .subject
            .as_ref()
            .windows(30)
            .any(|w| w == b"GSM Association - RSP2 Root CI"));
        let bits = euicc::der::iter(anchor.subject_public_key_info.as_ref())
            .find(|t| t.tag == 0x03)
            .unwrap()
            .value;
        assert_eq!(euicc::hex::encode(&sha1(&bits[1..])), GSMA_CI1_KEY_ID);
        assert!(tls_config().is_ok());
    }

    /// Minimal SHA-1 for the key-ID check (the helper has no SHA-1 dependency).
    fn sha1(data: &[u8]) -> [u8; 20] {
        let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
        let mut msg = data.to_vec();
        let bit_len = (data.len() as u64) * 8;
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        msg.extend(bit_len.to_be_bytes());
        for chunk in msg.chunks(64) {
            let mut w = [0u32; 80];
            for i in 0..16 {
                w[i] = u32::from_be_bytes([
                    chunk[4 * i],
                    chunk[4 * i + 1],
                    chunk[4 * i + 2],
                    chunk[4 * i + 3],
                ]);
            }
            for i in 16..80 {
                w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
            }
            let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
            for (i, wi) in w.iter().enumerate() {
                let (f, k) = match i {
                    0..=19 => ((b & c) | (!b & d), 0x5A827999),
                    20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                    40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                    _ => (b ^ c ^ d, 0xCA62C1D6),
                };
                let t = a
                    .rotate_left(5)
                    .wrapping_add(f)
                    .wrapping_add(e)
                    .wrapping_add(k)
                    .wrapping_add(*wi);
                e = d;
                d = c;
                c = b.rotate_left(30);
                b = a;
                a = t;
            }
            for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
                *x = x.wrapping_add(y);
            }
        }
        let mut out = [0u8; 20];
        for (i, v) in h.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&v.to_be_bytes());
        }
        out
    }
}
