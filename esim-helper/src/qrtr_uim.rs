// SPDX-License-Identifier: MIT
// Copyright (c) 2024, Luca Weiss <luca.weiss@fairphone.com>
// Copyright (c) 2026, Jesther Silvestre
//
// Based on lpac's QMI-over-QRTR APDU driver (driver/apdu/qmi_qrtr.c, qmi_common.c, MIT), which
// uses libqmi-glib. This Rust version speaks QMI directly on an AF_QIPCRTR socket: the same
// explicit node and slot, one owned logical channel, no retry of an uncertain open or close.
// Changed 2026-10-24 for zte-esim: waits stop early once the helper is cancelled (SIGTERM), except
// for opening a channel (its answer names the channel to close) and closing an owned channel.

//! APDUs to the SIM through the modem's QMI UIM service over QRTR (the U60's modem is node 3).

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

const AF_QIPCRTR: libc::c_int = 42;
#[cfg(target_os = "linux")]
const SOCK_FLAGS: libc::c_int = libc::SOCK_DGRAM | libc::SOCK_CLOEXEC;
#[cfg(not(target_os = "linux"))]
const SOCK_FLAGS: libc::c_int = libc::SOCK_DGRAM;
const QRTR_PORT_CTRL: u32 = 0xFFFF_FFFE;
const QRTR_TYPE_NEW_SERVER: u32 = 4;
const QRTR_TYPE_DEL_SERVER: u32 = 5;
const QRTR_TYPE_NEW_LOOKUP: u32 = 10;

const QMI_SERVICE_UIM: u32 = 0x0B;
const QMI_REQUEST: u8 = 0x00;
const QMI_RESPONSE: u8 = 0x02;
const UIM_SEND_APDU: u16 = 0x003B;
const UIM_LOGICAL_CHANNEL: u16 = 0x003F;
const UIM_OPEN_LOGICAL_CHANNEL: u16 = 0x0042;
const CLOSE_ALLOWANCE: Duration = Duration::from_secs(3);
const POLL_SLICE: Duration = Duration::from_millis(200);

#[repr(C)]
#[derive(Clone, Copy)]
struct SockaddrQrtr {
    sq_family: libc::sa_family_t,
    sq_node: u32,
    sq_port: u32,
}

/// A QMI UIM client on one QRTR node and SIM slot.
pub struct UimClient {
    fd: OwnedFd,
    node: u32,
    port: u32,
    slot: u8,
    txn: u16,
    timeout: Duration,
    /// The whole command gives up on the modem by this time.
    deadline: Instant,
    channel: Option<u8>,
    /// An open or close was sent but its answer never came: don't guess what the card holds.
    channel_unknown: bool,
}

/// QMI protocol error names for the codes seen around SIM access.
fn qmi_error_name(code: u16) -> &'static str {
    match code {
        0x01 => "MalformedMessage",
        0x02 => "NoMemory",
        0x03 => "Internal",
        0x10 => "NotProvisioned",
        0x11 => "MissingArgument",
        0x13 => "ArgumentTooLong",
        0x25 => "UimUninitialized",
        0x2E => "GeneralError",
        0x30 => "InvalidArgument",
        0x32 => "NoEntry",
        0x34 => "DeviceNotReady",
        0x44 => "InsufficientResources",
        0x46 => "InvalidOperation",
        0x47 => "InvalidQmiCommand",
        0x50 => "SimFileNotFound",
        0x52 => "AccessDenied",
        _ => "",
    }
}

pub(crate) fn qmi_error(code: u16) -> String {
    match qmi_error_name(code) {
        "" => format!("QMI error {code}"),
        name => format!("QMI error {code} {name}"),
    }
}

/// One QMI message as sent over QRTR: type, transaction, message id, TLVs.
pub(crate) fn encode(kind: u8, txn: u16, msg_id: u16, tlvs: &[(u8, &[u8])]) -> Vec<u8> {
    let mut body = Vec::new();
    for (t, v) in tlvs {
        body.push(*t);
        body.extend((v.len() as u16).to_le_bytes());
        body.extend_from_slice(v);
    }
    let mut out = vec![kind];
    out.extend(txn.to_le_bytes());
    out.extend(msg_id.to_le_bytes());
    out.extend((body.len() as u16).to_le_bytes());
    out.extend(body);
    out
}

pub(crate) struct Message<'a> {
    pub kind: u8,
    pub txn: u16,
    pub msg_id: u16,
    pub tlvs: Vec<(u8, &'a [u8])>,
}

pub(crate) fn decode(buf: &[u8]) -> Option<Message<'_>> {
    let le16 = |b: &[u8], i: usize| Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]));
    let kind = *buf.first()?;
    let txn = le16(buf, 1)?;
    let msg_id = le16(buf, 3)?;
    let len = usize::from(le16(buf, 5)?);
    let mut rest = buf.get(7..7 + len)?;
    let mut tlvs = Vec::new();
    while !rest.is_empty() {
        let t = *rest.first()?;
        let l = usize::from(le16(rest, 1)?);
        let v = rest.get(3..3 + l)?;
        tlvs.push((t, v));
        rest = &rest[3 + l..];
    }
    Some(Message {
        kind,
        txn,
        msg_id,
        tlvs,
    })
}

fn tlv<'a>(tlvs: &[(u8, &'a [u8])], t: u8) -> Option<&'a [u8]> {
    tlvs.iter().find(|(id, _)| *id == t).map(|(_, v)| *v)
}

/// The Result TLV (0x02): Ok, or the QMI error.
pub(crate) fn check_result(tlvs: &[(u8, &[u8])]) -> Result<(), String> {
    let r = tlv(tlvs, 0x02).ok_or("QMI response without a result")?;
    if r.len() < 4 {
        return Err("short QMI result".into());
    }
    let status = u16::from_le_bytes([r[0], r[1]]);
    let error = u16::from_le_bytes([r[2], r[3]]);
    if status == 0 {
        Ok(())
    } else {
        Err(qmi_error(error))
    }
}

fn os_err(what: &str) -> String {
    format!("{what}: {}", io::Error::last_os_error())
}

fn send_to(fd: &OwnedFd, node: u32, port: u32, buf: &[u8]) -> Result<(), String> {
    let addr = SockaddrQrtr {
        sq_family: AF_QIPCRTR as libc::sa_family_t,
        sq_node: node,
        sq_port: port,
    };
    // SAFETY: valid buffer and a sockaddr_qrtr of the declared size.
    let n = unsafe {
        libc::sendto(
            fd.as_raw_fd(),
            buf.as_ptr().cast(),
            buf.len(),
            0,
            (&addr as *const SockaddrQrtr).cast(),
            std::mem::size_of::<SockaddrQrtr>() as libc::socklen_t,
        )
    };
    if n < 0 {
        return Err(os_err("QRTR send"));
    }
    Ok(())
}

/// Waits until `deadline` for one datagram; returns its length and sender (node, port). With
/// `cancellable`, gives up once the helper has been asked to stop.
fn recv_from(
    fd: &OwnedFd,
    buf: &mut [u8],
    deadline: Instant,
    cancellable: bool,
) -> Result<(usize, u32, u32), String> {
    loop {
        if cancellable && crate::interruptible() {
            return Err("cancelled".into());
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("modem did not answer in time".into());
        }
        let mut pfd = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // Short slices so a cancellation is noticed even when the signal hit another thread.
        let ms = left.min(POLL_SLICE).as_millis().max(1) as libc::c_int;
        // SAFETY: one valid pollfd.
        let ready = unsafe { libc::poll(&mut pfd, 1, ms) };
        if ready < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(os_err("QRTR poll"));
        }
        if ready > 0 {
            break;
        }
    }
    // SAFETY: zeroed sockaddr_qrtr is a valid value for the kernel to fill.
    let mut addr: SockaddrQrtr = unsafe { std::mem::zeroed() };
    let mut alen = std::mem::size_of::<SockaddrQrtr>() as libc::socklen_t;
    // SAFETY: valid buffer, address storage and length.
    let n = unsafe {
        libc::recvfrom(
            fd.as_raw_fd(),
            buf.as_mut_ptr().cast(),
            buf.len(),
            0,
            (&mut addr as *mut SockaddrQrtr).cast(),
            &mut alen,
        )
    };
    if n < 0 {
        return Err(os_err("QRTR receive"));
    }
    Ok((n as usize, addr.sq_node, addr.sq_port))
}

fn le32(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

impl UimClient {
    /// Finds the UIM service on `node` and prepares a client for SIM `slot` (1-based). Each request
    /// waits at most `timeout`, and nothing waits past `deadline`.
    pub fn connect(
        node: u32,
        slot: u8,
        timeout: Duration,
        deadline: Instant,
    ) -> Result<Self, String> {
        // SAFETY: plain socket call; the fd is owned immediately.
        let raw = unsafe { libc::socket(AF_QIPCRTR, SOCK_FLAGS, 0) };
        if raw < 0 {
            return Err(os_err("QRTR socket"));
        }
        // SAFETY: `raw` is a fresh, valid descriptor.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: zeroed sockaddr_qrtr for getsockname to fill.
        let mut local: SockaddrQrtr = unsafe { std::mem::zeroed() };
        let mut alen = std::mem::size_of::<SockaddrQrtr>() as libc::socklen_t;
        // SAFETY: valid address storage and length.
        if unsafe {
            libc::getsockname(
                fd.as_raw_fd(),
                (&mut local as *mut SockaddrQrtr).cast(),
                &mut alen,
            )
        } < 0
        {
            return Err(os_err("QRTR getsockname"));
        }
        let mut lookup = Vec::with_capacity(20);
        for v in [QRTR_TYPE_NEW_LOOKUP, QMI_SERVICE_UIM, 0, 0, 0] {
            lookup.extend(v.to_le_bytes());
        }
        send_to(&fd, local.sq_node, QRTR_PORT_CTRL, &lookup)?;
        let lookup_deadline = (Instant::now() + timeout).min(deadline);
        let mut buf = [0u8; 64];
        let mut port = None;
        loop {
            let (n, _, from_port) = recv_from(&fd, &mut buf, lookup_deadline, true)?;
            if from_port != QRTR_PORT_CTRL || n < 20 || le32(&buf, 0) != QRTR_TYPE_NEW_SERVER {
                continue;
            }
            let (service, s_node, s_port) = (le32(&buf, 4), le32(&buf, 12), le32(&buf, 16));
            if service == 0 && s_node == 0 && s_port == 0 {
                break; // end of the current list
            }
            if service == QMI_SERVICE_UIM && s_node == node && port.is_none() {
                port = Some(s_port);
            }
        }
        let port = port.ok_or_else(|| format!("QRTR node {node} has no UIM service"))?;
        Ok(UimClient {
            fd,
            node,
            port,
            slot,
            txn: 0,
            timeout,
            deadline,
            channel: None,
            channel_unknown: false,
        })
    }

    /// Sends one request and returns the response TLVs (Result already checked).
    fn request(&mut self, msg_id: u16, tlvs: &[(u8, &[u8])]) -> Result<Vec<(u8, Vec<u8>)>, String> {
        let deadline = (Instant::now() + self.timeout).min(self.deadline);
        self.request_until(msg_id, tlvs, deadline, true)
    }

    fn request_until(
        &mut self,
        msg_id: u16,
        tlvs: &[(u8, &[u8])],
        deadline: Instant,
        cancellable: bool,
    ) -> Result<Vec<(u8, Vec<u8>)>, String> {
        self.txn = self.txn.wrapping_add(1).max(1);
        let txn = self.txn;
        send_to(
            &self.fd,
            self.node,
            self.port,
            &encode(QMI_REQUEST, txn, msg_id, tlvs),
        )?;
        let mut buf = vec![0u8; 65536];
        loop {
            let (n, node, port) = recv_from(&self.fd, &mut buf, deadline, cancellable)?;
            let data = &buf[..n];
            if port == QRTR_PORT_CTRL {
                if n >= 20
                    && le32(data, 0) == QRTR_TYPE_DEL_SERVER
                    && le32(data, 12) == self.node
                    && le32(data, 16) == self.port
                {
                    return Err("UIM service went away".into());
                }
                continue;
            }
            if node != self.node || port != self.port {
                continue;
            }
            let Some(m) = decode(data) else { continue };
            if m.kind != QMI_RESPONSE || m.txn != txn || m.msg_id != msg_id {
                continue; // indications and stale answers
            }
            check_result(&m.tlvs)?;
            return Ok(m.tlvs.into_iter().map(|(t, v)| (t, v.to_vec())).collect());
        }
    }
}

fn field(tlvs: &[(u8, Vec<u8>)], t: u8) -> Option<&[u8]> {
    tlvs.iter()
        .find(|(id, _)| *id == t)
        .map(|(_, v)| v.as_slice())
}

impl euicc::Apdu for UimClient {
    fn open_channel(&mut self, aid: &[u8]) -> euicc::Result<u8> {
        if self.channel.is_some() || self.channel_unknown || aid.is_empty() || aid.len() > 255 {
            return Err(euicc::Error::Transport(
                "open logical channel refused: channel state".into(),
            ));
        }
        let mut aid_tlv = vec![aid.len() as u8];
        aid_tlv.extend_from_slice(aid);
        let slot = [self.slot];
        // Not cancellable: once the open has been sent, its answer names the channel the modem
        // allocated, and abandoning it would leave that channel open until the SIM resets (seen
        // on the device). A stop takes effect right after, and the channel is then closed.
        let deadline = (Instant::now() + self.timeout).min(self.deadline);
        let out = self
            .request_until(
                UIM_OPEN_LOGICAL_CHANNEL,
                &[(0x01, &slot), (0x10, &aid_tlv)],
                deadline,
                false,
            )
            .map_err(|e| {
                self.channel_unknown = !e.starts_with("QMI error");
                if self.channel_unknown {
                    // The modem may have opened a channel whose number never reached us.
                    crate::note_channel_leak(0, &e);
                }
                euicc::Error::Transport(format!("open logical channel failed: {e}"))
            })?;
        match field(&out, 0x10).and_then(|v| v.first().copied()) {
            Some(ch) if ch != 0 => {
                self.channel = Some(ch);
                crate::note_channel(ch);
                Ok(ch)
            }
            _ => Err(euicc::Error::Transport(
                "open logical channel failed: no channel id".into(),
            )),
        }
    }

    fn close_channel(&mut self, channel: u8) {
        if self.channel != Some(channel) {
            return;
        }
        // Never retry a close after an uncertain answer; ownership is unknown then.
        self.channel = None;
        let slot = [self.slot];
        // The close gets its own short allowance, even after the command's deadline: an open
        // channel left on the card is worse than a few seconds' delay.
        let until = Instant::now() + CLOSE_ALLOWANCE.min(self.timeout);
        if let Err(e) = self.request_until(
            UIM_LOGICAL_CHANNEL,
            &[(0x01, &slot), (0x11, &[channel])],
            until,
            false,
        ) {
            self.channel_unknown = true;
            crate::note_channel_leak(channel, &e);
        }
    }

    fn transmit(&mut self, apdu: &[u8]) -> euicc::Result<Vec<u8>> {
        let channel = self.channel.ok_or(euicc::Error::Transport(
            "APDU without an open channel".into(),
        ))?;
        if apdu.len() > u16::MAX as usize {
            return Err(euicc::Error::Transport("APDU too long".into()));
        }
        let mut apdu_tlv = (apdu.len() as u16).to_le_bytes().to_vec();
        apdu_tlv.extend_from_slice(apdu);
        let slot = [self.slot];
        let out = self
            .request(
                UIM_SEND_APDU,
                &[(0x01, &slot), (0x02, &apdu_tlv), (0x10, &[channel])],
            )
            .map_err(|e| euicc::Error::Transport(format!("APDU failed: {e}")))?;
        let resp =
            field(&out, 0x10).ok_or(euicc::Error::Transport("APDU failed: no response".into()))?;
        if resp.len() < 2 {
            return Err(euicc::Error::Transport(
                "APDU failed: short response".into(),
            ));
        }
        let len = usize::from(u16::from_le_bytes([resp[0], resp[1]]));
        resp.get(2..2 + len)
            .map(<[u8]>::to_vec)
            .ok_or(euicc::Error::Transport(
                "APDU failed: truncated response".into(),
            ))
    }
}

impl Drop for UimClient {
    fn drop(&mut self) {
        if let Some(ch) = self.channel {
            euicc::Apdu::close_channel(self, ch);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qmi_messages_round_trip() {
        let m = encode(
            QMI_REQUEST,
            7,
            UIM_SEND_APDU,
            &[(0x01, &[1]), (0x02, &[2, 0, 0x80, 0xCA]), (0x10, &[3])],
        );
        assert_eq!(&m[..7], &[0x00, 7, 0, 0x3B, 0x00, 15, 0]);
        let d = decode(&m).unwrap();
        assert_eq!((d.kind, d.txn, d.msg_id), (QMI_REQUEST, 7, UIM_SEND_APDU));
        assert_eq!(
            d.tlvs,
            [
                (0x01, &[1][..]),
                (0x02, &[2, 0, 0x80, 0xCA][..]),
                (0x10, &[3][..])
            ]
        );
        assert!(decode(&m[..m.len() - 1]).is_none(), "truncated");
    }

    #[test]
    fn results_and_error_names() {
        let ok = encode(
            QMI_RESPONSE,
            1,
            UIM_OPEN_LOGICAL_CHANNEL,
            &[(0x02, &[0, 0, 0, 0]), (0x10, &[2])],
        );
        assert!(check_result(&decode(&ok).unwrap().tlvs).is_ok());
        let fail = encode(
            QMI_RESPONSE,
            1,
            UIM_OPEN_LOGICAL_CHANNEL,
            &[(0x02, &[1, 0, 0x50, 0])],
        );
        let e = check_result(&decode(&fail).unwrap().tlvs).unwrap_err();
        assert_eq!(
            e, "QMI error 80 SimFileNotFound",
            "matches what the agent classifies as a SIM reload"
        );
        assert_eq!(qmi_error(999), "QMI error 999");
    }
}
