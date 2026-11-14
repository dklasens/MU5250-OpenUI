use std::fs;
use std::path::Path;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::util::MutexExt;
use crate::{storage, ubus};

const STORAGE_PATH: &str = "/data/local/tmp/charge_limit.json";
const POLL_ACTIVE_SECS: u64 = 60;
const POLL_IDLE_SECS: u64 = 300;
fn default_hysteresis() -> u8 {
    5
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Persisted {
    enabled: bool,
    limit: u8,
    #[serde(default = "default_hysteresis")]
    hysteresis: u8,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargeUpdate {
    pub charging_stopped: Option<bool>,
    pub charge_limit_enabled: Option<bool>,
    pub charge_limit: Option<u8>,
    pub hysteresis: Option<u8>,
}

struct LimitState {
    policy: Persisted,
    manual_override: bool,
    last_error: Option<String>,
}

#[derive(Clone, Copy)]
struct Snapshot {
    stopped: bool,
    capacity: u8,
    discharging: bool,
}

trait Charger: Send + Sync {
    fn snapshot(&self) -> Result<Snapshot, String>;
    fn set_stopped(&self, stopped: bool) -> Result<(), String>;
    fn persist(&self, policy: &Persisted) -> Result<(), String>;
}
struct Hardware;

fn stopped_state() -> Result<bool, String> {
    let value = ubus::call("zwrt_bsp.charger", "list", Some("{}"))?;
    match value["direct_power_supply_mode"].as_str() {
        Some("enable") => Ok(true), // Firmware semantics are inverted.
        Some("disable") => Ok(false),
        _ => Err("charger control state is unavailable".into()),
    }
}
impl Charger for Hardware {
    fn snapshot(&self) -> Result<Snapshot, String> {
        let capacity = fs::read_to_string("/sys/class/power_supply/battery/capacity")
            .ok()
            .and_then(|s| s.trim().parse::<u8>().ok())
            .filter(|v| *v <= 100)
            .ok_or("battery capacity sensor is unavailable")?;
        Ok(Snapshot {
            stopped: stopped_state()?,
            capacity,
            discharging: fs::read_to_string("/sys/class/power_supply/battery/status")
                .map(|s| s.trim() == "Discharging")
                .unwrap_or(false),
        })
    }
    fn set_stopped(&self, stopped: bool) -> Result<(), String> {
        let mode = if stopped { "enable" } else { "disable" };
        ubus::call(
            "zwrt_bsp.charger",
            "set",
            Some(&format!(r#"{{"direct_power_supply_mode":"{mode}"}}"#)),
        )?;
        for _ in 0..3 {
            if stopped_state()? == stopped {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err("charger did not confirm the requested state".into())
    }
    fn persist(&self, policy: &Persisted) -> Result<(), String> {
        let bytes = serde_json::to_vec(policy).map_err(|e| e.to_string())?;
        storage::atomic_write(Path::new(STORAGE_PATH), &bytes)
            .map_err(|e| format!("saving charge policy: {e}"))
    }
}

pub struct ChargeLimitEnforcer {
    // All manual/configuration/enforcement changes use this one lock, including
    // hardware reads, writes and persistence. A tick cannot apply an old policy
    // after the user has disabled it.
    inner: Mutex<LimitState>,
    hardware: Box<dyn Charger>,
}

fn desired(policy: &Persisted, snapshot: Snapshot) -> bool {
    if !policy.enabled {
        return false;
    }
    if snapshot.discharging && !snapshot.stopped {
        return false;
    }
    if snapshot.capacity >= policy.limit {
        true
    } else if snapshot.capacity <= policy.limit.saturating_sub(policy.hysteresis) {
        false
    } else {
        snapshot.stopped
    }
}

impl ChargeLimitEnforcer {
    pub fn new() -> Self {
        let loaded = fs::read_to_string(STORAGE_PATH);
        let (policy, last_error) = match loaded {
            Ok(text) => match serde_json::from_str::<Persisted>(&text) {
                Ok(p) if (50..=100).contains(&p.limit) && (1..=20).contains(&p.hysteresis) => {
                    (p, None)
                }
                _ => (
                    Persisted {
                        enabled: false,
                        limit: 100,
                        hysteresis: 5,
                    },
                    Some("invalid saved charge policy; enforcement disabled".into()),
                ),
            },
            Err(e) => (
                Persisted {
                    enabled: false,
                    limit: 100,
                    hysteresis: 5,
                },
                (e.kind() != std::io::ErrorKind::NotFound)
                    .then(|| format!("reading charge policy: {e}")),
            ),
        };
        Self {
            inner: Mutex::new(LimitState {
                policy,
                manual_override: false,
                last_error,
            }),
            hardware: Box::new(Hardware),
        }
    }

    pub fn start(self: &Arc<Self>, events: mpsc::Receiver<Value>) {
        let enforcer = Arc::clone(self);
        std::thread::spawn(move || {
            let mut unplugged = false;
            loop {
                // A panic in one pass must not end enforcement: the next timed
                // pass runs as usual.
                crate::diag_log::contain("charge_policy", || enforcer.reconcile(unplugged));
                let interval = {
                    let state = enforcer.inner.safe_lock();
                    if state.policy.enabled && !state.manual_override {
                        POLL_ACTIVE_SECS
                    } else {
                        POLL_IDLE_SECS
                    }
                };
                // Always timed, including when unplugged or the event listener is
                // restarting. A missed reconnect cannot suspend enforcement.
                unplugged = match events.recv_timeout(Duration::from_secs(interval)) {
                    Ok(event) => {
                        matches!(&event["charger_connect"], Value::Number(n) if n.as_u64() == Some(0))
                            || event["charger_connect"].as_str() == Some("0")
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => false,
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        std::thread::sleep(Duration::from_secs(interval));
                        false
                    }
                };
            }
        });
    }

    fn reconcile(&self, unplugged: bool) {
        let mut state = self.inner.safe_lock();
        if !unplugged && (!state.policy.enabled || state.manual_override) {
            return;
        }
        let result = (|| {
            let snapshot = self.hardware.snapshot()?;
            let stopped = !unplugged && desired(&state.policy, snapshot);
            if stopped != snapshot.stopped {
                self.hardware.set_stopped(stopped)?;
            }
            if unplugged {
                state.manual_override = false;
            }
            Ok::<_, String>(())
        })();
        state.last_error = result.err();
        if let Some(error) = &state.last_error {
            eprintln!("[charge_policy] {error}");
        }
    }

    pub fn get(&self) -> (bool, u8, u8, bool) {
        let state = self.inner.safe_lock();
        (
            state.policy.enabled,
            state.policy.limit,
            state.policy.hysteresis,
            state.manual_override,
        )
    }
    pub fn last_error(&self) -> Option<String> {
        self.inner.safe_lock().last_error.clone()
    }

    pub fn update(&self, update: ChargeUpdate) -> Result<(), String> {
        let mut state = self.inner.safe_lock();
        let policy_changed = update.charge_limit_enabled.is_some()
            || update.charge_limit.is_some()
            || update.hysteresis.is_some();
        if !policy_changed && update.charging_stopped.is_none() {
            return Err("at least one charge control field is required".into());
        }
        let policy = Persisted {
            enabled: update.charge_limit_enabled.unwrap_or(state.policy.enabled),
            limit: update.charge_limit.unwrap_or(state.policy.limit),
            hysteresis: update.hysteresis.unwrap_or(state.policy.hysteresis),
        };
        if !(50..=100).contains(&policy.limit) || !(1..=20).contains(&policy.hysteresis) {
            return Err("limit must be 50-100 and hysteresis must be 1-20".into());
        }
        let result = (|| {
            let before = self.hardware.snapshot()?;
            let stopped = update
                .charging_stopped
                .unwrap_or_else(|| desired(&policy, before));
            // Always verify an explicit manual command, even if the read already
            // matches. Failed mutations never report a successful configuration.
            if let Err(error) = self.hardware.set_stopped(stopped) {
                let rollback = self.hardware.set_stopped(before.stopped);
                return Err(format!(
                    "{error}; restore previous charging state: {rollback:?}"
                ));
            }
            if policy_changed {
                if let Err(error) = self.hardware.persist(&policy) {
                    let hardware_restore = self.hardware.set_stopped(before.stopped);
                    let storage_restore = self.hardware.persist(&state.policy);
                    return Err(format!("{error}; restore charger: {hardware_restore:?}; restore policy: {storage_restore:?}"));
                }
            }
            state.policy = policy;
            state.manual_override = update.charging_stopped.unwrap_or(if policy_changed {
                false
            } else {
                state.manual_override
            });
            Ok(())
        })();
        state.last_error = result.as_ref().err().cloned();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fake {
        snapshot: Arc<Mutex<Snapshot>>,
        writes: Arc<Mutex<Vec<bool>>>,
        fail_write: bool,
        fail_save: bool,
    }
    impl Charger for Fake {
        fn snapshot(&self) -> Result<Snapshot, String> {
            Ok(*self.snapshot.lock().unwrap())
        }
        fn set_stopped(&self, value: bool) -> Result<(), String> {
            if self.fail_write {
                return Err("hardware failure".into());
            }
            self.writes.lock().unwrap().push(value);
            self.snapshot.lock().unwrap().stopped = value;
            Ok(())
        }
        fn persist(&self, _: &Persisted) -> Result<(), String> {
            if self.fail_save {
                Err("storage full".into())
            } else {
                Ok(())
            }
        }
    }
    fn fixture(fail_write: bool, fail_save: bool) -> (ChargeLimitEnforcer, Arc<Mutex<Snapshot>>) {
        let snapshot = Arc::new(Mutex::new(Snapshot {
            capacity: 90,
            stopped: false,
            discharging: false,
        }));
        let hardware = Fake {
            snapshot: snapshot.clone(),
            writes: Arc::default(),
            fail_write,
            fail_save,
        };
        (
            ChargeLimitEnforcer {
                inner: Mutex::new(LimitState {
                    policy: Persisted {
                        enabled: true,
                        limit: 80,
                        hysteresis: 5,
                    },
                    manual_override: false,
                    last_error: None,
                }),
                hardware: Box::new(hardware),
            },
            snapshot,
        )
    }
    #[test]
    fn dropped_reconnect_is_repaired_by_next_periodic_tick() {
        let (enforcer, snapshot) = fixture(false, false);
        enforcer.reconcile(false);
        assert!(snapshot.lock().unwrap().stopped);
        enforcer.reconcile(true);
        assert!(!snapshot.lock().unwrap().stopped);
        enforcer.reconcile(false); // No reconnect event was received.
        assert!(snapshot.lock().unwrap().stopped);
    }
    #[test]
    fn persistence_failure_restores_hardware_and_keeps_previous_policy() {
        let (enforcer, snapshot) = fixture(false, true);
        assert!(enforcer
            .update(ChargeUpdate {
                charge_limit: Some(70),
                ..Default::default()
            })
            .is_err());
        assert!(!snapshot.lock().unwrap().stopped);
        assert_eq!(enforcer.get().1, 80);
        assert!(enforcer.last_error().unwrap().contains("storage full"));
    }
    #[test]
    fn hardware_failure_does_not_claim_policy_was_disabled() {
        let (enforcer, _) = fixture(true, false);
        assert!(enforcer
            .update(ChargeUpdate {
                charge_limit_enabled: Some(false),
                ..Default::default()
            })
            .is_err());
        assert!(enforcer.get().0);
    }
    #[test]
    fn hysteresis_and_manual_override_are_respected() {
        let (enforcer, snapshot) = fixture(false, false);
        enforcer
            .update(ChargeUpdate {
                charging_stopped: Some(true),
                ..Default::default()
            })
            .unwrap();
        snapshot.lock().unwrap().capacity = 60;
        enforcer.reconcile(false);
        assert!(snapshot.lock().unwrap().stopped);
        enforcer
            .update(ChargeUpdate {
                charge_limit: Some(80),
                ..Default::default()
            })
            .unwrap();
        assert!(!snapshot.lock().unwrap().stopped);
        snapshot.lock().unwrap().capacity = 78;
        enforcer.reconcile(false);
        assert!(!snapshot.lock().unwrap().stopped);
    }
    #[test]
    fn invalid_or_overflowing_updates_are_rejected() {
        assert!(serde_json::from_str::<ChargeUpdate>(r#"{"charge_limit":336}"#).is_err());
        assert!(serde_json::from_str::<ChargeUpdate>(r#"{"charging_stopped":"true"}"#).is_err());
    }
}
