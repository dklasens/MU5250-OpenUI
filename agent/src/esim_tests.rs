use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

const EID: &str = "89049032000000000000000000000000";
const AMAYSIM: &str = "8961000000000000001";
const FIJI: &str = "89679000000000000002";
const TEST: &str = "8999000000000000003";

#[derive(Clone)]
struct P {
    iccid: &'static str,
    state: &'static str,
    class: &'static str,
    nickname: Option<&'static str>,
    name: Option<&'static str>,
}

/// What the scripted card does with the next write.
#[derive(Clone, Copy, PartialEq)]
enum Write {
    Apply,
    Busy,
    LoseAnswerApplied,
    LoseAnswerNotApplied,
}

struct Card {
    eid: &'static str,
    profiles: Vec<P>,
    notifications: Vec<u64>,
    write: Write,
    /// Inventory reads that fail as if the SIM were reloading.
    unreadable: u32,
    isdr: bool,
    next_seq: u64,
}

const NEW: &str = "8999000000000000004";
const CODE: &str = "LPA:1$smdp.example.com$TEST-MATCHING-ID";

/// How the scripted download goes.
#[derive(Clone, Copy, PartialEq)]
enum Dl {
    Normal,
    ServerRefuses,
    /// The card's answer to the install is lost; `true` if the profile was installed anyway.
    LoseInstallAnswer(bool),
}

struct FakeEnv {
    card: Arc<Mutex<Card>>,
    calls: Mutex<Vec<Value>>,
    wan: AtomicBool,
    deliver: AtomicBool,
    inputs: WarningInputs,
    dl: Mutex<Dl>,
    /// While set, the scripted install waits (to probe the installing state).
    hold_install: Arc<AtomicBool>,
    controls: Arc<Mutex<Vec<String>>>,
}

/// A scripted helper download over the shared fake card.
struct FakeRun {
    card: Arc<Mutex<Card>>,
    dl: Dl,
    hold: Arc<AtomicBool>,
    controls: Arc<Mutex<Vec<String>>>,
    queue: std::collections::VecDeque<RunEvent>,
    stage: u8,
}

impl FakeRun {
    fn progress(step: &str) -> RunEvent {
        RunEvent::Progress {
            step: step.into(),
            data: Value::Null,
        }
    }
    fn failed(kind: &str, message: &str, before: &[u64]) -> RunEvent {
        RunEvent::Done(HelperOutcome::Failed {
            kind: kind.into(),
            message: message.into(),
            detail: json!({"notifications_before": before}),
        })
    }
    fn take_control(&self) -> Option<String> {
        let mut c = self.controls.safe_lock();
        (!c.is_empty()).then(|| c.remove(0))
    }
}

impl DownloadRun for FakeRun {
    fn next(&mut self, until: Instant) -> RunEvent {
        if let Some(e) = self.queue.pop_front() {
            return e;
        }
        let before = self.card.safe_lock().notifications.clone();
        match self.stage {
            0 => {
                self.stage = 1;
                for step in [
                    "es10b_get_euicc_challenge_and_info",
                    "es9p_initiate_authentication",
                    "es10b_authenticate_server",
                    "es9p_authenticate_client",
                ] {
                    self.queue.push_back(Self::progress(step));
                }
                if self.dl == Dl::ServerRefuses {
                    self.queue.push_back(Self::failed(
                        "download_failed",
                        "es9p_authenticate_client: The matchingID is not found",
                        &before,
                    ));
                    return self.queue.pop_front().unwrap();
                }
                self.queue.push_back(RunEvent::Progress {
                    step: "es8p_metadata_parse".into(),
                    // The name is the ICCID, as some operators do: it must not reach a client.
                    data: json!({"iccid": NEW, "provider": "Test Mobile", "name": NEW, "class": "operational"}),
                });
                self.queue.push_back(Self::progress("preview"));
                self.queue.pop_front().unwrap()
            }
            1 => match self.take_control().as_deref() {
                Some("install") => {
                    self.stage = 2;
                    for step in [
                        "es10b_prepare_download",
                        "es9p_get_bound_profile_package",
                        "es10b_load_bound_profile_package",
                    ] {
                        self.queue.push_back(Self::progress(step));
                    }
                    self.queue.pop_front().unwrap()
                }
                Some("decline") => {
                    Self::failed("download_cancelled", "declined at the preview", &before)
                }
                Some("timeout") => {
                    Self::failed("download_cancelled", "not confirmed in time", &before)
                }
                Some(_) => Self::failed("download_cancelled", "cancelled", &before),
                None => {
                    std::thread::sleep(
                        until
                            .saturating_duration_since(Instant::now())
                            .min(Duration::from_millis(5)),
                    );
                    RunEvent::Wait
                }
            },
            _ => {
                if self.hold.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(5));
                    return RunEvent::Wait;
                }
                self.stage = 3;
                let installed = !matches!(self.dl, Dl::LoseInstallAnswer(false));
                if installed {
                    let mut card = self.card.safe_lock();
                    card.profiles.push(P {
                        iccid: NEW,
                        state: "disabled",
                        class: "operational",
                        nickname: None,
                        name: Some(NEW),
                    });
                    card.notifications.push(41);
                }
                match self.dl {
                    Dl::LoseInstallAnswer(_) => Self::failed(
                        "outcome_unknown",
                        "es10b_load_bound_profile_package: modem did not answer in time",
                        &before,
                    ),
                    _ => RunEvent::Done(HelperOutcome::Ok(
                        json!({"iccid": NEW, "seq": 41, "notifications_before": before}),
                    )),
                }
            }
        }
    }
    fn send(&mut self, control: &str) -> bool {
        self.controls.safe_lock().push(control.to_string());
        true
    }
}

impl FakeEnv {
    fn new() -> Arc<Self> {
        Arc::new(FakeEnv {
            card: Arc::new(Mutex::new(Card {
                eid: EID,
                profiles: vec![
                    P {
                        iccid: AMAYSIM,
                        state: "enabled",
                        class: "operational",
                        nickname: None,
                        name: Some("Amaysim"),
                    },
                    P {
                        iccid: FIJI,
                        state: "disabled",
                        class: "operational",
                        nickname: Some("Vodafone Fiji"),
                        name: Some(FIJI),
                    },
                    P {
                        iccid: TEST,
                        state: "disabled",
                        class: "test",
                        nickname: None,
                        name: None,
                    },
                ],
                // A backlog left by other LPAs: never sent by the agent.
                notifications: vec![15, 16, 30],
                write: Write::Apply,
                unreadable: 0,
                isdr: true,
                next_seq: 40,
            })),
            calls: Mutex::new(Vec::new()),
            dl: Mutex::new(Dl::Normal),
            hold_install: Arc::new(AtomicBool::new(false)),
            controls: Arc::new(Mutex::new(Vec::new())),
            wan: AtomicBool::new(true),
            deliver: AtomicBool::new(true),
            inputs: WarningInputs {
                manual_apn: true,
                roaming_enabled: Some(false),
                sim_mcc: Some("505".into()),
                network_mcc: Some("505".into()),
            },
        })
    }

    fn ops(&self) -> Vec<String> {
        self.calls
            .safe_lock()
            .iter()
            .map(|c| c["op"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    fn inventory(card: &Card) -> HelperOutcome {
        HelperOutcome::Ok(json!({
            "eid": card.eid,
            "info": {"firmware": "37.4.3", "sgp22_version": "2.3.0", "free_nvm_bytes": 1428096},
            "profiles": card.profiles.iter().map(|p| json!({
                "iccid": p.iccid, "state": p.state, "class": p.class,
                "nickname": p.nickname, "name": p.name, "provider": p.name,
            })).collect::<Vec<_>>(),
            "notifications": card.notifications.iter().map(|s| json!({"seq": s})).collect::<Vec<_>>(),
        }))
    }
}

impl Env for FakeEnv {
    fn start_download(&self, request: &Value, _deadline: Duration) -> Box<dyn DownloadRun> {
        self.calls.safe_lock().push(request.clone());
        Box::new(FakeRun {
            card: self.card.clone(),
            dl: *self.dl.safe_lock(),
            hold: self.hold_install.clone(),
            controls: self.controls.clone(),
            queue: Default::default(),
            stage: 0,
        })
    }
    fn helper(&self, request: &Value, _deadline: Duration) -> HelperOutcome {
        self.calls.safe_lock().push(request.clone());
        let mut card = self.card.safe_lock();
        let failed = |kind: &str, detail: Value| HelperOutcome::Failed {
            kind: kind.into(),
            message: kind.into(),
            detail,
        };
        match request["op"].as_str().unwrap() {
            "inventory" => {
                if !card.isdr {
                    return failed("isdr_unavailable", Value::Null);
                }
                if card.unreadable > 0 {
                    card.unreadable -= 1;
                    return failed("isdr_unavailable", Value::Null);
                }
                Self::inventory(&card)
            }
            op @ ("enable" | "disable") => {
                let iccid = request["iccid"].as_str().unwrap().to_string();
                let apply = |card: &mut Card| {
                    for p in card.profiles.iter_mut() {
                        if op == "enable" {
                            p.state = if p.iccid == iccid {
                                "enabled"
                            } else {
                                "disabled"
                            };
                        } else if p.iccid == iccid {
                            p.state = "disabled";
                        }
                    }
                    let seq = card.next_seq;
                    card.next_seq += 1;
                    card.notifications.push(seq);
                };
                match card.write {
                    Write::Apply => {
                        apply(&mut card);
                        HelperOutcome::Ok(json!({"result": "applied"}))
                    }
                    Write::Busy => failed("refused", json!({"code": 5})),
                    Write::LoseAnswerApplied => {
                        apply(&mut card);
                        failed("outcome_unknown", Value::Null)
                    }
                    Write::LoseAnswerNotApplied => failed("outcome_unknown", Value::Null),
                }
            }
            "delete" => {
                let iccid = request["iccid"].as_str().unwrap();
                card.profiles.retain(|p| p.iccid != iccid);
                let seq = card.next_seq;
                card.next_seq += 1;
                card.notifications.push(seq);
                HelperOutcome::Ok(json!({"result": "applied"}))
            }
            "nickname" => {
                let iccid = request["iccid"].as_str().unwrap();
                let name: &'static str = Box::leak(
                    request["nickname"]
                        .as_str()
                        .unwrap()
                        .to_string()
                        .into_boxed_str(),
                );
                for p in card.profiles.iter_mut().filter(|p| p.iccid == iccid) {
                    p.nickname = (!name.is_empty()).then_some(name);
                }
                HelperOutcome::Ok(json!({"result": "applied"}))
            }
            "notify" => {
                let seqs: Vec<u64> = request["seqs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_u64)
                    .collect();
                if !self.deliver.load(Ordering::Relaxed) {
                    return HelperOutcome::Ok(json!({"results": seqs.iter().map(|s| json!({
                        "seq": s, "delivered": false, "host": "rsp.example", "error": "HTTP 500",
                    })).collect::<Vec<_>>()}));
                }
                card.notifications.retain(|s| !seqs.contains(s));
                HelperOutcome::Ok(json!({"results": seqs.iter().map(|s| json!({
                    "seq": s, "delivered": true, "removed": true,
                })).collect::<Vec<_>>()}))
            }
            other => panic!("unexpected helper op {other}"),
        }
    }
    fn sim_key(&self) -> Option<String> {
        Some("sim ready|505|02".into())
    }
    fn wan_up(&self) -> bool {
        self.wan.load(Ordering::Relaxed)
    }
    fn warning_inputs(&self) -> WarningInputs {
        self.inputs.clone()
    }
    fn invalidate_network(&self) {}
}

fn fast() -> Timing {
    Timing {
        verify_delay: Duration::from_millis(5),
        verify_interval: Duration::from_millis(5),
        verify_window: Duration::from_millis(300),
        receipt_settle: Duration::from_millis(1),
        wan_poll: Duration::from_millis(5),
        preview_limit: Duration::from_millis(300),
    }
}

fn temp_paths() -> (PathBuf, PathBuf) {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "esim-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    (dir.join("state.json"), dir.join("journal.json"))
}

fn manager(env: &Arc<FakeEnv>) -> (Arc<EsimManager>, PathBuf, PathBuf) {
    let (state, journal) = temp_paths();
    let m = EsimManager::with_paths(env.clone(), state.clone(), journal.clone(), fast());
    (m, state, journal)
}

fn view(m: &EsimManager) -> Value {
    m.profiles().1["data"].clone()
}

fn handle_of(v: &Value, masked: &str) -> String {
    v["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["iccid_masked"] == masked)
        .unwrap()["handle"]
        .as_str()
        .unwrap()
        .to_string()
}

fn request(v: &Value, masked: &str, id: &str) -> Vec<u8> {
    json!({"handle": handle_of(v, masked), "revision": v["revision"], "operation_id": id})
        .to_string()
        .into_bytes()
}

fn settle(m: &EsimManager) -> Value {
    for _ in 0..400 {
        let v = m.view();
        if v["operation"]["resolved"] == true {
            return v;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("operation never resolved: {}", m.view());
}

#[test]
fn identifiers_never_reach_the_client() {
    let env = FakeEnv::new();
    let (m, ..) = manager(&env);
    let v = view(&m);
    let text = v.to_string();
    for id in [EID, AMAYSIM, FIJI, TEST] {
        assert!(!text.contains(id), "{id} leaked: {text}");
    }
    let fiji = v["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["iccid_masked"] == "…0002")
        .unwrap();
    assert_eq!(fiji["display_name"], "Vodafone Fiji");
    assert!(
        fiji["name"].is_null(),
        "an ICCID used as the name is dropped"
    );
    assert_eq!(v["card"]["id"], "…0000");
}

#[test]
fn the_list_puts_the_active_profile_first_and_warns_before_switching() {
    let env = FakeEnv::new();
    let (m, ..) = manager(&env);
    let v = view(&m);
    let p = v["profiles"].as_array().unwrap();
    assert_eq!(p[0]["state"], "enabled");
    assert_eq!(p[1]["display_name"], "Vodafone Fiji");
    assert_eq!(p[1]["warnings"], json!(["manual_apn", "roaming_off"]));
    assert!(p[2]["display_name"].is_null(), "unnamed last");
    assert_eq!(p[0]["warnings"], json!([]));
}

#[test]
fn reads_and_detection_never_write() {
    let env = FakeEnv::new();
    let (m, ..) = manager(&env);
    m.summary();
    view(&m);
    m.refresh();
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        env.ops().iter().all(|op| op == "inventory"),
        "{:?}",
        env.ops()
    );
}

#[test]
fn an_ordinary_sim_is_unsupported_and_not_probed_again() {
    let env = FakeEnv::new();
    env.card.safe_lock().isdr = false;
    let (m, ..) = manager(&env);
    m.detect();
    assert_eq!(m.view()["status"], "unsupported");
    m.inner.safe_lock().detected_at = None;
    m.detect();
    assert_eq!(env.ops().len(), 1, "same SIM: no second probe");
}

#[test]
fn activation_is_verified_and_sends_only_its_own_receipts() {
    let env = FakeEnv::new();
    let (m, _, journal) = manager(&env);
    let v = view(&m);
    let (code, body) = m.switch(Kind::Enable, &request(&v, "…0002", "op-enable-1"));
    assert_eq!(code, 202, "{body}");
    let done = settle(&m);
    assert_eq!(done["operation"]["state"], "succeeded");
    assert_eq!(done["operation"]["target_name"], "Vodafone Fiji");
    assert!(!journal.exists(), "the journal is cleared once settled");
    for _ in 0..200 {
        if env.ops().contains(&"notify".to_string()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let notify = env
        .calls
        .safe_lock()
        .iter()
        .find(|c| c["op"] == "notify")
        .cloned()
        .unwrap();
    assert_eq!(
        notify["seqs"],
        json!([40]),
        "the backlog (15, 16, 30) is left alone"
    );
    assert_eq!(env.ops().iter().filter(|o| *o == "enable").count(), 1);
}

#[test]
fn a_busy_card_is_reported_and_not_retried() {
    let env = FakeEnv::new();
    env.card.safe_lock().write = Write::Busy;
    let (m, ..) = manager(&env);
    let v = view(&m);
    m.switch(Kind::Enable, &request(&v, "…0002", "op-busy-0001"));
    let done = settle(&m);
    assert_eq!(done["operation"]["state"], "busy");
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(env.ops().iter().filter(|o| *o == "enable").count(), 1);
}

#[test]
fn stale_locked_test_and_repeated_requests_are_refused_without_a_write() {
    let env = FakeEnv::new();
    let (m, ..) = manager(&env);
    let v = view(&m);

    let mut stale: Value = serde_json::from_slice(&request(&v, "…0002", "op-stale-01")).unwrap();
    stale["revision"] = json!(v["revision"].as_u64().unwrap() + 1);
    assert_eq!(m.switch(Kind::Enable, stale.to_string().as_bytes()).0, 409);

    assert_eq!(
        m.switch(Kind::Enable, &request(&v, "…0003", "op-test-001"))
            .0,
        409
    );
    assert_eq!(
        m.switch(Kind::Disable, &request(&v, "…0002", "op-disable-1"))
            .0,
        409
    );

    let lock = json!({"handle": handle_of(&v, "…0002"), "locked": true});
    assert_eq!(m.lock(lock.to_string().as_bytes()).0, 200);
    assert_eq!(m.view()["profiles"][1]["locked"], true);
    assert_eq!(
        m.switch(Kind::Enable, &request(&v, "…0002", "op-locked-1"))
            .0,
        423
    );

    assert_eq!(m.switch(Kind::Enable, b"{}").0, 400);
    assert!(env.ops().iter().all(|op| op == "inventory"));

    let unlock = json!({"handle": handle_of(&v, "…0002"), "locked": false});
    m.lock(unlock.to_string().as_bytes());
    assert_eq!(
        m.switch(Kind::Enable, &request(&v, "…0002", "op-repeat-1"))
            .0,
        202
    );
    let again = m.switch(Kind::Enable, &request(&v, "…0002", "op-repeat-1"));
    assert_eq!(
        again.0, 200,
        "the same operation id is answered, not repeated"
    );
    settle(&m);
    assert_eq!(env.ops().iter().filter(|o| *o == "enable").count(), 1);
}

#[test]
fn concurrent_requests_admit_exactly_one_change() {
    let env = FakeEnv::new();
    let (m, ..) = manager(&env);
    let v = view(&m);
    let codes: Vec<u16> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let (m, v) = (&m, &v);
                scope.spawn(move || {
                    m.switch(
                        Kind::Enable,
                        &request(v, "…0002", &format!("op-race-{i:04}")),
                    )
                    .0
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(codes.iter().filter(|c| **c == 202).count(), 1, "{codes:?}");
    assert!(codes.iter().all(|c| *c == 202 || *c == 409), "{codes:?}");
    settle(&m);
    assert_eq!(env.ops().iter().filter(|o| *o == "enable").count(), 1);
}

#[test]
fn locks_survive_a_restart() {
    let env = FakeEnv::new();
    let (state, journal) = temp_paths();
    let m = EsimManager::with_paths(env.clone(), state.clone(), journal.clone(), fast());
    let v = view(&m);
    let h = handle_of(&v, "…0002");
    m.lock(json!({"handle": h, "locked": true}).to_string().as_bytes());
    let again = EsimManager::with_paths(env.clone(), state, journal, fast());
    let v = view(&again);
    assert_eq!(
        handle_of(&v, "…0002"),
        h,
        "handles are stable across restarts"
    );
    assert_eq!(v["profiles"][1]["locked"], true);
}

#[test]
fn a_lost_answer_is_settled_by_reading_back() {
    for (write, expect) in [
        (Write::LoseAnswerApplied, "succeeded"),
        (Write::LoseAnswerNotApplied, "failed"),
    ] {
        let env = FakeEnv::new();
        env.card.safe_lock().write = write;
        let (m, ..) = manager(&env);
        let v = view(&m);
        m.switch(Kind::Disable, &request(&v, "…0001", "op-lost-0001"));
        assert_eq!(settle(&m)["operation"]["state"], expect);
        assert_eq!(env.ops().iter().filter(|o| *o == "disable").count(), 1);
    }
}

#[test]
fn reloading_is_waited_out_and_an_unreadable_card_ends_unknown() {
    let env = FakeEnv::new();
    let (m, ..) = manager(&env);
    let v = view(&m);
    env.card.safe_lock().unreadable = 3;
    m.switch(Kind::Enable, &request(&v, "…0002", "op-reload-01"));
    assert_eq!(settle(&m)["operation"]["state"], "succeeded");

    let env = FakeEnv::new();
    let (m, ..) = manager(&env);
    let v = view(&m);
    env.card.safe_lock().unreadable = 10_000;
    m.switch(Kind::Enable, &request(&v, "…0002", "op-reload-02"));
    let done = settle(&m);
    assert_eq!(done["operation"]["state"], "unknown");
    assert_eq!(
        done["freshness"]["stale"], true,
        "the last good list stays, marked stale"
    );
    assert_eq!(done["profiles"].as_array().unwrap().len(), 3);
}

#[test]
fn a_restart_reads_back_and_never_replays() {
    let env = FakeEnv::new();
    let (state, journal) = temp_paths();
    let m = EsimManager::with_paths(env.clone(), state.clone(), journal.clone(), fast());
    let v = view(&m);
    let target = handle_of(&v, "…0001");
    std::fs::write(
        &journal,
        json!({"version": 1, "id": "op-before-restart", "kind": "disable",
               "target": target, "previous": target, "receipt_before": [15, 16, 30]})
        .to_string(),
    )
    .unwrap();
    let after = EsimManager::with_paths(env.clone(), state, journal.clone(), fast());
    after.recover();
    let done = settle(&after);
    // The card still shows the profile enabled. The helper holds the card lock while it runs,
    // so this read cannot race a late write: the change did not happen, and it is not sent.
    assert_eq!(done["operation"]["state"], "failed");
    assert!(!env.ops().contains(&"disable".to_string()));
    assert!(!journal.exists());
}

#[test]
fn undelivered_receipts_stay_pending() {
    let env = FakeEnv::new();
    env.deliver.store(false, Ordering::Relaxed);
    let (m, ..) = manager(&env);
    let v = view(&m);
    m.switch(Kind::Disable, &request(&v, "…0001", "op-receipt-1"));
    settle(&m);
    for _ in 0..200 {
        if m.view()["receipts"]["last_error"].is_string() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let v = m.view();
    assert_eq!(v["receipts"]["pending"], 1);
    assert!(env.card.safe_lock().notifications.contains(&40));
}

#[test]
fn receipts_wait_for_data() {
    let env = FakeEnv::new();
    env.wan.store(false, Ordering::Relaxed);
    let (m, ..) = manager(&env);
    let v = view(&m);
    m.switch(Kind::Enable, &request(&v, "…0002", "op-nodata-01"));
    settle(&m);
    std::thread::sleep(Duration::from_millis(60));
    assert!(!env.ops().contains(&"notify".to_string()));
    env.wan.store(true, Ordering::Relaxed);
    for _ in 0..200 {
        if env.ops().contains(&"notify".to_string()) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("receipts were not sent once data returned");
}

#[test]
fn renames_are_validated_and_read_back() {
    let env = FakeEnv::new();
    let (m, ..) = manager(&env);
    let v = view(&m);
    let body = |name: &str, id: &str| {
        let mut b: Value = serde_json::from_slice(&request(&v, "…0001", id)).unwrap();
        b["nickname"] = json!(name);
        b.to_string().into_bytes()
    };
    assert_eq!(m.nickname(&body(&"x".repeat(65), "op-name-0001")).0, 400);
    assert_eq!(m.nickname(&body("a\u{202E}b", "op-name-0002")).0, 400);
    let (code, answer) = m.nickname(&body("Home", "op-name-0003"));
    assert_eq!(code, 200, "{answer}");
    assert_eq!(answer["data"]["profiles"][0]["display_name"], "Home");
    assert!(env
        .ops()
        .iter()
        .all(|op| op == "inventory" || op == "nickname"));
}

#[test]
fn helper_answers_are_parsed_strictly() {
    let ok =
        br#"{"ok":true,"protocol":2,"data":{"x":1},"channel":{"last":3,"leak_suspected":false}}"#;
    assert!(matches!(parse_helper_output(ok), HelperOutcome::Ok(_)));
    let progress_then_answer = b"{\"progress\":\"x\"}\n{\"ok\":false,\"protocol\":2,\"error\":{\"kind\":\"refused\",\"message\":\"catBusy\",\"detail\":{\"code\":5}}}\n";
    assert!(matches!(
        parse_helper_output(progress_then_answer),
        HelperOutcome::Failed { ref kind, .. } if kind == "refused"
    ));
    assert!(matches!(
        parse_helper_output(br#"{"ok":true,"protocol":1}"#),
        HelperOutcome::Missing(_)
    ));
    assert!(matches!(
        parse_helper_output(b""),
        HelperOutcome::Garbled(_)
    ));
}

#[test]
fn names_identifiers_and_countries() {
    assert_eq!(mask(FIJI), "…0002");
    assert!(identifier_like(FIJI, FIJI, EID));
    assert!(identifier_like("8967 9000 0000 0000 0002", FIJI, EID));
    assert!(!identifier_like("Vodafone Fiji", FIJI, EID));
    assert!(!identifier_like("Travel 2026", FIJI, EID));
    assert_eq!(iccid_country(AMAYSIM), Some("61"));
    assert_eq!(iccid_country(FIJI), Some("679"));
    assert_eq!(iccid_country("8901260000000000000"), Some("1"));
    assert_eq!(iccid_country("123"), None);
    assert!(valid_nickname("").is_ok());
    assert!(valid_nickname(&"é".repeat(33)).is_err());
}

// -- phase 2: download, delete, receipts -------------------------------------------------------

fn wait_for(m: &EsimManager, what: &str, f: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..600 {
        let v = m.view();
        if f(&v) {
            return v;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("{what} never happened: {}", m.view());
}

fn start_download(m: &Arc<EsimManager>, id: &str) -> (u16, Value) {
    m.download(
        json!({"activation_code": CODE, "operation_id": id})
            .to_string()
            .as_bytes(),
    )
}

fn op_body(id: &str, extra: Value) -> Vec<u8> {
    let mut b = json!({"operation_id": id});
    if let (Some(o), Some(e)) = (b.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    b.to_string().into_bytes()
}

#[test]
fn a_download_previews_installs_names_and_sends_only_its_receipt() {
    let env = FakeEnv::new();
    let (m, _, journal) = manager(&env);
    view(&m);
    let (code, _) = start_download(&m, "op-download-01");
    assert_eq!(code, 202);
    let v = wait_for(&m, "the preview", |v| {
        v["operation"]["state"] == "awaiting_confirmation"
    });
    assert_eq!(v["operation"]["preview"]["provider"], "Test Mobile");
    assert!(
        v["operation"]["preview"]["name"].is_null(),
        "an ICCID used as a name is hidden"
    );
    assert_eq!(v["operation"]["can_cancel"], true);
    assert!(v["operation"]["preview_expires_in_ms"].as_u64().unwrap() > 0);
    // The code went to the helper and nowhere else.
    let journal_text = std::fs::read_to_string(&journal).unwrap();
    for text in [v.to_string(), journal_text] {
        assert!(
            !text.contains("TEST-MATCHING-ID") && !text.contains(NEW),
            "{text}"
        );
    }
    assert_eq!(
        env.calls
            .safe_lock()
            .iter()
            .filter(|c| c["op"] == "download")
            .count(),
        1
    );

    let (code, _) = m.download_confirm(&op_body(
        "op-download-01",
        json!({"install": true, "nickname": "Trip"}),
    ));
    assert_eq!(code, 200);
    let done = settle(&m);
    assert_eq!(done["operation"]["state"], "succeeded", "{done}");
    assert_eq!(done["operation"]["target_name"], "Trip");
    let new = done["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["display_name"] == "Trip")
        .expect("the new profile, named");
    assert_eq!(new["state"], "disabled");
    assert!(!journal.exists());
    for _ in 0..200 {
        if env.ops().contains(&"notify".to_string()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let notify = env
        .calls
        .safe_lock()
        .iter()
        .find(|c| c["op"] == "notify")
        .cloned()
        .unwrap();
    assert_eq!(notify["seqs"], json!([41]), "only the install receipt");
}

#[test]
fn declining_cancelling_and_not_answering_install_nothing() {
    for (answer, expect) in [
        (Some(json!({"install": false})), "Declined"),
        (None, "Cancelled"),
        (Some(Value::Null), "not confirmed"),
    ] {
        let env = FakeEnv::new();
        let (m, ..) = manager(&env);
        view(&m);
        start_download(&m, "op-download-02");
        wait_for(&m, "the preview", |v| {
            v["operation"]["state"] == "awaiting_confirmation"
        });
        match &answer {
            Some(Value::Null) => {} // let the preview expire
            Some(body) => {
                assert_eq!(
                    m.download_confirm(&op_body("op-download-02", body.clone()))
                        .0,
                    200
                )
            }
            None => assert_eq!(
                m.download_cancel(&op_body("op-download-02", json!({}))).0,
                200
            ),
        }
        let done = settle(&m);
        assert_eq!(done["operation"]["state"], "cancelled", "{done}");
        assert!(done["operation"]["message"]
            .as_str()
            .unwrap()
            .contains(expect));
        assert_eq!(done["profiles"].as_array().unwrap().len(), 3);
        if answer == Some(Value::Null) {
            assert_eq!(
                env.controls.safe_lock().len(),
                0,
                "the timeout was consumed"
            );
        }
    }
}

#[test]
fn an_install_in_progress_cannot_be_cancelled() {
    let env = FakeEnv::new();
    env.hold_install.store(true, Ordering::Relaxed);
    let (m, ..) = manager(&env);
    view(&m);
    start_download(&m, "op-download-03");
    wait_for(&m, "the preview", |v| {
        v["operation"]["state"] == "awaiting_confirmation"
    });
    m.download_confirm(&op_body("op-download-03", json!({"install": true})));
    let v = wait_for(&m, "installing", |v| {
        v["operation"]["state"] == "installing"
    });
    assert_eq!(v["operation"]["can_cancel"], false);
    assert_eq!(
        m.download_cancel(&op_body("op-download-03", json!({}))).0,
        409
    );
    assert_eq!(
        m.receipts_send().0,
        409,
        "nothing else runs while a download is in flight"
    );
    env.hold_install.store(false, Ordering::Relaxed);
    assert_eq!(settle(&m)["operation"]["state"], "succeeded");
}

#[test]
fn downloads_are_refused_up_front_without_echoing_the_code() {
    let env = FakeEnv::new();
    let (m, ..) = manager(&env);
    view(&m);
    let secret = "LPA:1$192.168.0.1$SECRET-MATCHING-ID";
    for (body, code) in [
        (
            json!({"activation_code": secret, "operation_id": "op-refuse-01"}),
            400,
        ),
        (
            json!({"activation_code": "LPA:2$a.example.com$SECRET", "operation_id": "op-refuse-02"}),
            400,
        ),
        (
            json!({"activation_code": "LPA:1$a.example.com$SECRET$$1", "operation_id": "op-refuse-03"}),
            400,
        ),
        (
            json!({"activation_code": CODE, "operation_id": "short"}),
            400,
        ),
    ] {
        let (status, answer) = m.download(body.to_string().as_bytes());
        assert_eq!(status, code, "{answer}");
        assert!(!answer.to_string().contains("SECRET"), "{answer}");
    }
    env.wan.store(false, Ordering::Relaxed);
    let (status, answer) = start_download(&m, "op-nodata-01");
    assert_eq!(status, 409);
    assert!(answer["error"].as_str().unwrap().contains("mobile data"));
    assert!(
        m.view()["operation"].is_null(),
        "a refused start leaves no operation"
    );
    assert!(!env.ops().contains(&"download".to_string()));
}

#[test]
fn a_server_refusal_installs_nothing() {
    let env = FakeEnv::new();
    *env.dl.safe_lock() = Dl::ServerRefuses;
    let (m, ..) = manager(&env);
    view(&m);
    start_download(&m, "op-refused-01");
    let done = settle(&m);
    assert_eq!(done["operation"]["state"], "failed");
    let message = done["operation"]["message"].as_str().unwrap();
    assert!(
        message.contains("refused this activation code"),
        "{message}"
    );
    assert!(
        message.contains("matchingID is not found"),
        "the server's reason is kept"
    );
    assert_eq!(done["profiles"].as_array().unwrap().len(), 3);
}

#[test]
fn a_lost_install_answer_is_settled_by_the_card() {
    for (installed, expect) in [(true, "succeeded"), (false, "failed")] {
        let env = FakeEnv::new();
        *env.dl.safe_lock() = Dl::LoseInstallAnswer(installed);
        let (m, ..) = manager(&env);
        view(&m);
        start_download(&m, "op-lost-dl-01");
        wait_for(&m, "the preview", |v| {
            v["operation"]["state"] == "awaiting_confirmation"
        });
        m.download_confirm(&op_body("op-lost-dl-01", json!({"install": true})));
        assert_eq!(settle(&m)["operation"]["state"], expect);
    }
}

#[test]
fn a_restart_during_a_download_reads_back_and_never_resumes() {
    for (installed, expect) in [(true, "succeeded"), (false, "failed")] {
        let env = FakeEnv::new();
        let (state, journal) = temp_paths();
        let m = EsimManager::with_paths(env.clone(), state.clone(), journal.clone(), fast());
        let v = view(&m);
        let before: Vec<Value> = v["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["handle"].clone())
            .collect();
        if installed {
            env.card.safe_lock().profiles.push(P {
                iccid: NEW,
                state: "disabled",
                class: "operational",
                nickname: None,
                name: None,
            });
        }
        std::fs::write(
            &journal,
            json!({"version": 1, "id": "op-before-restart", "kind": "download", "target": "",
                   "receipt_before": [15, 16, 30], "profiles_before": before})
            .to_string(),
        )
        .unwrap();
        let after = EsimManager::with_paths(env.clone(), state, journal.clone(), fast());
        after.recover();
        assert_eq!(settle(&after)["operation"]["state"], expect);
        assert!(!env.ops().contains(&"download".to_string()));
        assert!(!journal.exists());
    }
}

#[test]
fn deletion_needs_an_inactive_unlocked_profile() {
    let env = FakeEnv::new();
    let (m, ..) = manager(&env);
    let v = view(&m);
    assert_eq!(
        m.switch(Kind::Delete, &request(&v, "…0001", "op-del-active"))
            .0,
        409
    );
    let lock = json!({"handle": handle_of(&v, "…0002"), "locked": true});
    m.lock(lock.to_string().as_bytes());
    assert_eq!(
        m.switch(Kind::Delete, &request(&v, "…0002", "op-del-locked"))
            .0,
        423
    );
    assert!(!env.ops().contains(&"delete".to_string()));

    let (code, _) = m.switch(Kind::Delete, &request(&v, "…0003", "op-del-test-1"));
    assert_eq!(code, 202);
    let done = settle(&m);
    assert_eq!(done["operation"]["state"], "succeeded", "{done}");
    assert_eq!(done["profiles"].as_array().unwrap().len(), 2);
    for _ in 0..200 {
        if env.ops().contains(&"notify".to_string()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let notify = env
        .calls
        .safe_lock()
        .iter()
        .find(|c| c["op"] == "notify")
        .cloned()
        .unwrap();
    assert_eq!(notify["seqs"], json!([40]), "only the delete receipt");
}

#[test]
fn send_receipts_delivers_the_backlog_only_on_request() {
    let env = FakeEnv::new();
    let (m, ..) = manager(&env);
    view(&m);
    m.summary();
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        !env.ops().contains(&"notify".to_string()),
        "never on its own"
    );
    assert_eq!(m.receipts_send().0, 202);
    for _ in 0..200 {
        if env.ops().contains(&"notify".to_string()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let notify = env
        .calls
        .safe_lock()
        .iter()
        .find(|c| c["op"] == "notify")
        .cloned()
        .unwrap();
    assert_eq!(notify["seqs"], json!([15, 16, 30]));
}

#[test]
fn activation_codes_are_checked() {
    assert_eq!(check_activation_code(CODE), Ok(false));
    assert_eq!(
        check_activation_code("LPA:1$a.example.com$M$1.2.3$1"),
        Ok(true)
    );
    assert_eq!(
        check_activation_code(" lpa:1$a.example.com:8443$M "),
        Ok(false)
    );
    for bad in [
        "",
        "LPA:1$a.example.com",
        "LPA:1$10.0.0.1$M",
        "LPA:1$localhost$M",
        "LPA:1$a.example.com/x$M",
        "http://a.example.com",
        "LPA:1$a.example.com$M\u{0}",
    ] {
        assert!(check_activation_code(bad).is_err(), "{bad:?}");
    }
    assert!(check_activation_code(&format!("LPA:1$a.example.com${}", "x".repeat(600))).is_err());
}

#[test]
fn download_refusals_are_explained_in_plain_words() {
    let eid = explain_download_failure("es9p_authenticate_client: EID-Blacklist");
    assert!(
        eid.contains("refuses this eSIM card (EID-Blacklist)"),
        "{eid}"
    );
    assert!(eid.contains("removable eSIM cards"));
    let code = explain_download_failure("es9p_authenticate_client: MatchingID is refused");
    assert!(
        code.contains("refused this activation code (MatchingID is refused)"),
        "{code}"
    );
    let net =
        explain_download_failure("es9p_initiate_authentication: HTTP transport failed (dns error)");
    assert!(net.contains("could not be reached"), "{net}");
    let card =
        explain_download_failure("the card refused the profile: storeMetadata, iccidAlreadyExists");
    assert_eq!(
        card,
        "Nothing was installed. The card refused the profile: storeMetadata, iccidAlreadyExists."
    );
    assert_eq!(
        explain_download_failure("something else"),
        "Nothing was installed: something else."
    );
}
