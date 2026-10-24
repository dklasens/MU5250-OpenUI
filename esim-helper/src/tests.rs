use super::*;
use euicc::der::Node;
use std::sync::mpsc;

const EID: &str = "89049032000000000000000000000000";
const AMAYSIM: &str = "8961000000000000001";
const FIJI: &str = "89679000000000000002";
const TEST: &str = "8999000000000000003";

#[derive(Clone)]
struct Profile {
    iccid: &'static str,
    enabled: bool,
    class: u8,
    nickname: Option<String>,
}

/// A scripted eUICC answering the ES10 requests the helper may send.
#[derive(Clone)]
struct FakeCard {
    eid: &'static str,
    profiles: Vec<Profile>,
    notifications: Vec<(u64, u8, &'static str)>,
    switch_code: u64,
    lose_write_answer: bool,
    open_error: Option<&'static str>,
    sent: std::rc::Rc<std::cell::RefCell<Vec<u16>>>,
}

impl FakeCard {
    fn new() -> Self {
        FakeCard {
            eid: EID,
            profiles: vec![
                Profile {
                    iccid: AMAYSIM,
                    enabled: true,
                    class: 2,
                    nickname: None,
                },
                Profile {
                    iccid: FIJI,
                    enabled: false,
                    class: 2,
                    nickname: Some("Vodafone Fiji".into()),
                },
                Profile {
                    iccid: TEST,
                    enabled: false,
                    class: 0,
                    nickname: None,
                },
            ],
            notifications: vec![(30, 0x40, "rsp-3110.idemia.io")],
            switch_code: 0,
            lose_write_answer: false,
            open_error: None,
            sent: Default::default(),
        }
    }

    fn tags(&self) -> Vec<u16> {
        self.sent.borrow().clone()
    }

    fn answer(&mut self, req: &[u8]) -> euicc::Result<Vec<u8>> {
        let t = euicc::der::parse(req).unwrap();
        self.sent.borrow_mut().push(t.tag);
        let body = match t.tag {
            0xBF3E => Node::Cons(
                0xBF3E,
                vec![Node::Prim(0x5A, euicc::hex::decode(self.eid).unwrap())],
            ),
            0xBF22 => Node::Cons(
                0xBF22,
                vec![
                    Node::Prim(0x82, vec![2, 3, 0]),
                    Node::Prim(0x83, vec![37, 4, 3]),
                    Node::Cons(
                        0xA9,
                        vec![Node::Prim(
                            0x04,
                            euicc::hex::decode(https::GSMA_CI1_KEY_ID).unwrap(),
                        )],
                    ),
                ],
            ),
            0xBF2D => Node::Cons(
                0xBF2D,
                vec![Node::Cons(
                    0xA0,
                    self.profiles
                        .iter()
                        .map(|p| {
                            let mut f = vec![
                                Node::Prim(0x5A, euicc::hex::iccid_to_bcd(p.iccid, 10).unwrap()),
                                Node::Prim(0x9F70, vec![p.enabled as u8]),
                                Node::Prim(0x95, vec![p.class]),
                            ];
                            if let Some(n) = &p.nickname {
                                f.push(Node::Prim(0x90, n.as_bytes().to_vec()));
                            }
                            Node::Cons(0xE3, f)
                        })
                        .collect(),
                )],
            ),
            0xBF28 => Node::Cons(
                0xBF28,
                vec![Node::Cons(
                    0xA0,
                    self.notifications
                        .iter()
                        .map(|(seq, op, addr)| {
                            Node::Cons(
                                0xBF2F,
                                vec![
                                    Node::Prim(0x80, euicc::der::from_u64(*seq)),
                                    Node::Prim(0x81, vec![0x07, *op]),
                                    Node::Prim(0x0C, addr.as_bytes().to_vec()),
                                ],
                            )
                        })
                        .collect(),
                )],
            ),
            0xBF2B => {
                let (seq, op, addr) = self.notifications[0];
                Node::Cons(
                    0xBF2B,
                    vec![Node::Cons(
                        0xA0,
                        vec![Node::Cons(
                            0x30,
                            vec![Node::Cons(
                                0xBF2F,
                                vec![
                                    Node::Prim(0x80, euicc::der::from_u64(seq)),
                                    Node::Prim(0x81, vec![0x07, op]),
                                    Node::Prim(0x0C, addr.as_bytes().to_vec()),
                                ],
                            )],
                        )],
                    )],
                )
            }
            0xBF31 | 0xBF32 => {
                if self.lose_write_answer {
                    return Err(euicc::Error::Transport(
                        "modem did not answer in time".into(),
                    ));
                }
                Node::Cons(t.tag, vec![Node::Prim(0x80, vec![self.switch_code as u8])])
            }
            0xBF29 => Node::Cons(0xBF29, vec![Node::Prim(0x80, vec![0])]),
            0xBF33 => Node::Cons(0xBF33, vec![Node::Prim(0x80, vec![self.switch_code as u8])]),
            0xBF30 => {
                self.notifications.clear();
                Node::Cons(0xBF30, vec![Node::Prim(0x80, vec![0])])
            }
            other => panic!("unexpected request {other:04X}"),
        };
        let mut out = body.encode();
        out.extend_from_slice(&[0x90, 0x00]);
        Ok(out)
    }
}

impl Apdu for FakeCard {
    fn open_channel(&mut self, _aid: &[u8]) -> euicc::Result<u8> {
        match self.open_error {
            Some(e) => Err(euicc::Error::Transport(e.into())),
            None => Ok(1),
        }
    }
    fn close_channel(&mut self, _channel: u8) {}
    fn transmit(&mut self, apdu: &[u8]) -> euicc::Result<Vec<u8>> {
        self.answer(&apdu[5..])
    }
}

fn card(fake: &FakeCard) -> Card<impl FnMut() -> Result<FakeCard, String> + '_> {
    Card {
        connect: move || Ok(fake.clone()),
    }
}

struct NoHttp;
impl euicc::es9p::Http for NoHttp {
    fn post(&mut self, _: &str, _: &[&str], _: &[u8]) -> Result<(u32, Vec<u8>), String> {
        panic!("no HTTP expected")
    }
}

struct Server(u32, Vec<String>);
impl euicc::es9p::Http for Server {
    fn post(&mut self, url: &str, _: &[&str], _: &[u8]) -> Result<(u32, Vec<u8>), String> {
        self.1.push(url.to_string());
        Ok((self.0, vec![]))
    }
}

impl download::Es9pTransport for NoHttp {
    fn set_timeout(&mut self, _: Duration) {}
}
impl download::Es9pTransport for Server {
    fn set_timeout(&mut self, _: Duration) {}
}

/// Runs a request with no control lines and no progress output.
fn go<T: Apdu, H: download::Es9pTransport>(
    req: &Request,
    card: &mut Card<impl FnMut() -> Result<T, String>>,
    http: &mut H,
) -> Result<Value, Failure> {
    let (_tx, rx) = mpsc::channel();
    dispatch(req, card, http, &rx, &mut |_, _| {})
}

fn request(body: Value) -> Request {
    parse_request(body.to_string().as_bytes()).unwrap()
}

#[test]
fn inventory_reads_and_never_writes() {
    let _s = crate::test_serial();
    let fake = FakeCard::new();
    let data = go(
        &request(json!({"protocol": 2, "op": "inventory"})),
        &mut card(&fake),
        &mut NoHttp,
    )
    .unwrap();
    assert_eq!(data["eid"], EID);
    assert_eq!(data["profiles"].as_array().unwrap().len(), 3);
    assert_eq!(data["profiles"][0]["state"], "enabled");
    assert_eq!(data["profiles"][2]["class"], "test");
    assert_eq!(data["info"]["firmware"], "37.4.3");
    assert_eq!(data["notifications"][0]["seq"], 30);
    assert!(fake.tags().iter().all(|t| !tag::WRITES.contains(t)));
}

#[test]
fn the_guard_refuses_commands_outside_the_operation() {
    let mut g = Guard {
        inner: FakeCard::new(),
        allowed: &[tag::GET_EID],
        write_sent: Rc::new(Cell::new(false)),
        continuing: None,
    };
    let set_nickname = [0x81, 0xE2, 0x91, 0x00, 0x03, 0xBF, 0x29, 0x00];
    assert!(g.transmit(&set_nickname).is_err());
    let orphan_continuation = [0x81, 0xE2, 0x91, 0x01, 0x01, 0x00];
    assert!(g.transmit(&orphan_continuation).is_err());
    let select = [0x00, 0xA4, 0x04, 0x00, 0x02, 0x3F, 0x00];
    assert!(g.transmit(&select).is_err());
    assert!(!g.write_sent.get());
    assert!(g.allows(&[0x81, 0xC0, 0x00, 0x00, 0x10]).is_some());
}

#[test]
fn enable_checks_state_in_the_same_session_and_reports_receipts_before() {
    let _s = crate::test_serial();
    let fake = FakeCard::new();
    let data = go(
        &request(json!({"protocol": 2, "op": "enable", "iccid": FIJI,
            "expect_eid": EID, "expect_enabled": AMAYSIM})),
        &mut card(&fake),
        &mut NoHttp,
    )
    .unwrap();
    assert_eq!(data["result"], "applied");
    assert_eq!(data["notifications_before"], json!([30]));
    assert_eq!(
        fake.tags(),
        [0xBF3E, 0xBF2D, 0xBF28, 0xBF31],
        "identity and state are read right before the one write"
    );
    CARD_RESET.store(false, Ordering::Relaxed);
}

#[test]
fn stale_or_unsafe_selections_send_no_write() {
    let _s = crate::test_serial();
    let cases = [
        // The card's enabled profile differs from what the client saw.
        (
            json!({"op": "enable", "iccid": FIJI, "expect_eid": EID, "expect_enabled": null}),
            "state_changed",
        ),
        // Test-class profiles are never activated.
        (
            json!({"op": "enable", "iccid": TEST, "expect_eid": EID, "expect_enabled": AMAYSIM}),
            "state_changed",
        ),
        // Already enabled.
        (
            json!({"op": "enable", "iccid": AMAYSIM, "expect_eid": EID, "expect_enabled": AMAYSIM}),
            "state_changed",
        ),
        // Disable of an inactive profile.
        (
            json!({"op": "disable", "iccid": FIJI, "expect_eid": EID, "expect_enabled": AMAYSIM}),
            "state_changed",
        ),
        // Another card.
        (
            json!({"op": "disable", "iccid": AMAYSIM, "expect_eid": "89049032000000000000000000000009", "expect_enabled": AMAYSIM}),
            "card_changed",
        ),
    ];
    for (mut body, kind) in cases {
        body["protocol"] = json!(2);
        let fake = FakeCard::new();
        let err = go(&request(body.clone()), &mut card(&fake), &mut NoHttp).unwrap_err();
        assert_eq!(err.kind, kind, "{body}");
        assert!(
            fake.tags().iter().all(|t| !tag::WRITES.contains(t)),
            "{body}"
        );
    }
}

#[test]
fn refusals_are_named_and_lost_answers_are_unknown() {
    let _s = crate::test_serial();
    let body = json!({"protocol": 2, "op": "disable", "iccid": AMAYSIM,
        "expect_eid": EID, "expect_enabled": AMAYSIM});
    let mut busy = FakeCard::new();
    busy.switch_code = 5;
    let err = go(&request(body.clone()), &mut card(&busy), &mut NoHttp).unwrap_err();
    assert_eq!((err.kind, err.message.as_str()), ("refused", "catBusy"));
    assert_eq!(err.extra["code"], 5);

    let mut policy = FakeCard::new();
    policy.switch_code = 3;
    let err = go(&request(body.clone()), &mut card(&policy), &mut NoHttp).unwrap_err();
    assert_eq!(err.message, "disallowedByPolicy");

    let mut lost = FakeCard::new();
    lost.lose_write_answer = true;
    let err = go(&request(body), &mut card(&lost), &mut NoHttp).unwrap_err();
    assert_eq!(err.kind, "outcome_unknown");
}

#[test]
fn a_cancelled_run_sends_nothing_further() {
    let _s = crate::test_serial();
    CANCELLED.store(true, Ordering::Relaxed);
    let fake = FakeCard::new();
    let err = go(
        &request(json!({"protocol": 2, "op": "enable", "iccid": FIJI,
            "expect_eid": EID, "expect_enabled": AMAYSIM})),
        &mut card(&fake),
        &mut NoHttp,
    )
    .unwrap_err();
    CANCELLED.store(false, Ordering::Relaxed);
    assert_eq!(err.kind, "cancelled");
    assert!(fake.tags().is_empty());
}

#[test]
fn reload_and_missing_isdr_are_told_apart_from_transport_failures() {
    assert_eq!(
        classify_open("open logical channel failed: QMI error 80 SimFileNotFound").kind,
        "isdr_unavailable"
    );
    assert_eq!(
        classify_open("QRTR node 3 has no UIM service").kind,
        "management_unavailable"
    );
    assert_eq!(
        classify_open("modem did not answer in time").kind,
        "transport"
    );
    let _s = crate::test_serial();
    let mut fake = FakeCard::new();
    fake.open_error = Some("open logical channel failed: QMI error 80 SimFileNotFound");
    let err = go(
        &request(json!({"protocol": 2, "op": "inventory"})),
        &mut card(&fake),
        &mut NoHttp,
    )
    .unwrap_err();
    assert_eq!(err.kind, "isdr_unavailable");
}

#[test]
fn nicknames_are_bounded_and_visible() {
    assert!(valid_nickname("").is_ok());
    assert!(valid_nickname("Vodafone Fiji").is_ok());
    assert!(valid_nickname("Été 🌴 Travel").is_ok());
    assert!(valid_nickname(&"a".repeat(64)).is_ok());
    assert!(valid_nickname(&"a".repeat(65)).is_err());
    assert!(valid_nickname(&"é".repeat(33)).is_err(), "66 bytes");
    assert!(valid_nickname("tab\there").is_err());
    assert!(valid_nickname("evil\u{202E}txt").is_err());
    assert!(valid_nickname("zero\u{200B}width").is_err());
}

#[test]
fn nickname_writes_only_the_nickname() {
    let _s = crate::test_serial();
    let fake = FakeCard::new();
    let data = go(
        &request(json!({"protocol": 2, "op": "nickname", "iccid": FIJI,
            "expect_eid": EID, "nickname": "Holiday"})),
        &mut card(&fake),
        &mut NoHttp,
    )
    .unwrap();
    assert_eq!(data["result"], "applied");
    assert_eq!(fake.tags(), [0xBF3E, 0xBF2D, 0xBF29]);
}

#[test]
fn receipts_are_removed_only_after_delivery() {
    let _s = crate::test_serial();
    let body = json!({"protocol": 2, "op": "notify", "expect_eid": EID, "seqs": [30, 31]});

    let failing = FakeCard::new();
    let mut server = Server(500, vec![]);
    let data = go(&request(body.clone()), &mut card(&failing), &mut server).unwrap();
    assert_eq!(
        server.1,
        ["https://rsp-3110.idemia.io/gsma/rsp2/es9plus/handleNotification"]
    );
    assert!(
        !failing.tags().contains(&0xBF30),
        "an undelivered receipt stays on the card"
    );
    let results = data["results"].as_array().unwrap();
    assert!(results.iter().any(|r| r["seq"] == 31 && r["gone"] == true));
    assert!(results
        .iter()
        .any(|r| r["seq"] == 30 && r["delivered"] == false));

    let accepting = FakeCard::new();
    let data = go(
        &request(body),
        &mut card(&accepting),
        &mut Server(204, vec![]),
    )
    .unwrap();
    assert!(accepting.tags().ends_with(&[0xBF3E, 0xBF30]));
    assert!(data["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["seq"] == 30 && r["delivered"] == true && r["removed"] == true));
}

#[test]
fn requests_are_validated() {
    for (raw, why) in [
        (json!({"op": "inventory"}), "no protocol"),
        (json!({"protocol": 3, "op": "inventory"}), "future protocol"),
        (json!({"protocol": 2, "op": "inventory", "slot": 3}), "slot"),
        (
            json!({"protocol": 2, "op": "inventory", "lock_path": "/data/x"}),
            "lock path",
        ),
        (
            json!({"protocol": 2, "op": "inventory", "lock_path": "/tmp/../etc/x"}),
            "lock path",
        ),
    ] {
        assert!(parse_request(raw.to_string().as_bytes()).is_err(), "{why}");
    }
    let _s = crate::test_serial();
    for body in [
        json!({"protocol": 2, "op": "enable", "iccid": "8961; rm -rf /", "expect_eid": EID, "expect_enabled": null}),
        json!({"protocol": 2, "op": "enable", "iccid": FIJI, "expect_eid": "short", "expect_enabled": null}),
        json!({"protocol": 2, "op": "apdu", "data": "00A4"}),
        json!({"protocol": 2, "op": "notify", "expect_eid": EID, "seqs": []}),
    ] {
        let fake = FakeCard::new();
        let err = go(&request(body.clone()), &mut card(&fake), &mut NoHttp).unwrap_err();
        assert_eq!(err.kind, "invalid_request", "{body}");
        assert!(fake.tags().is_empty());
    }
}

#[test]
fn delete_needs_an_inactive_profile_and_reports_receipts_before() {
    let _s = crate::test_serial();
    let fake = FakeCard::new();
    let data = go(
        &request(json!({"protocol": 2, "op": "delete", "iccid": FIJI, "expect_eid": EID})),
        &mut card(&fake),
        &mut NoHttp,
    )
    .unwrap();
    assert_eq!(data["result"], "applied");
    assert_eq!(data["notifications_before"], json!([30]));
    assert_eq!(fake.tags(), [0xBF3E, 0xBF2D, 0xBF28, 0xBF33]);
    assert!(
        !CARD_RESET.load(Ordering::Relaxed),
        "a delete does not reset the card"
    );

    let active = FakeCard::new();
    let err = go(
        &request(json!({"protocol": 2, "op": "delete", "iccid": AMAYSIM, "expect_eid": EID})),
        &mut card(&active),
        &mut NoHttp,
    )
    .unwrap_err();
    assert_eq!(err.kind, "state_changed");
    assert!(!active.tags().contains(&0xBF33));

    let mut policy = FakeCard::new();
    policy.switch_code = 3;
    let err = go(
        &request(json!({"protocol": 2, "op": "delete", "iccid": FIJI, "expect_eid": EID})),
        &mut card(&policy),
        &mut NoHttp,
    )
    .unwrap_err();
    assert_eq!(
        (err.kind, err.message.as_str()),
        ("refused", "disallowedByPolicy")
    );
}

#[test]
fn the_guard_follows_multi_block_commands_and_nothing_else() {
    let mut g = Guard {
        inner: FakeCard::new(),
        allowed: &[0xBF36, 0x86],
        write_sent: Rc::new(Cell::new(false)),
        continuing: None,
    };
    // A continuation block with no first block before it.
    assert!(g.allows(&[0x81, 0xE2, 0x91, 0x01, 0x01, 0x00]).is_none());
    // First block of an allowed command, then its continuation and last block.
    assert_eq!(
        g.allows(&[0x81, 0xE2, 0x11, 0x00, 0x03, 0xBF, 0x36, 0x82]),
        Some(0xBF36)
    );
    assert_eq!(
        g.allows(&[0x81, 0xE2, 0x11, 0x01, 0x01, 0x00]),
        Some(0xBF36)
    );
    assert_eq!(
        g.allows(&[0x81, 0xE2, 0x91, 0x02, 0x01, 0x00]),
        Some(0xBF36)
    );
    // The command ended: another continuation is refused.
    assert!(g.allows(&[0x81, 0xE2, 0x91, 0x03, 0x01, 0x00]).is_none());
    // One-byte tags are read as one byte.
    assert_eq!(
        g.allows(&[0x81, 0xE2, 0x91, 0x00, 0x02, 0x86, 0x00]),
        Some(0x86)
    );
    // Not in this operation's list.
    assert!(g
        .allows(&[0x81, 0xE2, 0x91, 0x00, 0x03, 0xBF, 0x31, 0x00])
        .is_none());
    assert!(
        g.allows(&[0x81, 0xE2, 0x51, 0x00, 0x03, 0xBF, 0x36, 0x00])
            .is_none(),
        "odd P1"
    );
}

#[test]
fn a_download_passes_the_guard_end_to_end() {
    let _s = crate::test_serial();
    let mut scripted = download::tests::Card::default();
    let mut smdp = download::tests::Smdp::default();
    let guard = Guard {
        inner: &mut scripted,
        allowed: &[
            tag::CHALLENGE,
            tag::INFO1,
            tag::AUTHENTICATE_SERVER,
            tag::PREPARE_DOWNLOAD,
            tag::CANCEL_SESSION,
            0xBF36,
            0xA0,
            0xA1,
            0x88,
            0xA2,
            0xA3,
            0x86,
        ],
        write_sent: Rc::new(Cell::new(false)),
        continuing: None,
    };
    let (tx, rx) = mpsc::channel();
    let code = download::parse_activation_code("LPA:1$smdp.example.com$TEST-MATCHING-ID").unwrap();
    let result = {
        let mut e = Euicc::open(guard).unwrap();
        download::download(
            &mut e,
            &mut smdp,
            &code,
            None,
            &rx,
            // Answer at the preview, as the agent does; earlier decisions are discarded.
            &mut |step, _| {
                if step == "preview" {
                    tx.send(download::DownloadCtl::Accept).unwrap();
                }
            },
            Instant::now() + Duration::from_secs(300),
        )
    };
    INSTALLING.store(false, Ordering::Relaxed);
    assert_eq!(result.unwrap()["seq"], 7);
    assert!(scripted.log.contains(&"install".to_string()));
}
