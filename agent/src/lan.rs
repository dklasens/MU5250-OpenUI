//! Confirmed LAN changes with a persistent recovery record and listener rebinding.
use crate::util::MutexExt;
use crate::{storage, ubus};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const JOURNAL: &str = "/data/local/tmp/lan_transition.json";
const CONFIRM_SECONDS: u64 = 120;

pub struct Binding {
    address: Mutex<String>,
    pub generation: AtomicU64,
    fixed: bool,
}
impl Binding {
    pub fn new() -> Self {
        let explicit = std::env::var("ZTE_AGENT_BIND").ok();
        let fixed = explicit.is_some();
        let address = explicit.unwrap_or_else(|| {
            format!(
                "{}:9090",
                configured_ip().unwrap_or_else(|| "192.168.0.1".into())
            )
        });
        Self {
            address: Mutex::new(address),
            generation: AtomicU64::new(0),
            fixed,
        }
    }
    pub fn address(&self) -> String {
        self.address.safe_lock().clone()
    }
    fn set(&self, ip: &str) {
        if self.fixed {
            return;
        }
        let mut address = self.address.safe_lock();
        let desired = format!("{ip}:9090");
        if *address != desired {
            *address = desired;
            self.generation.fetch_add(1, Ordering::Release);
        }
    }
}
fn configured_ip() -> Option<String> {
    ubus::uci_get("zwrt_router.network.lan_ipaddr")
        .ok()
        .filter(|ip| ip.parse::<Ipv4Addr>().is_ok_and(|ip| ip.is_private()))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanSettings {
    pub ipaddr: String,
    pub netmask: String,
    pub dhcp_enabled: bool,
    pub dhcp_start: String,
    pub dhcp_end: String,
    pub lease_seconds: u32,
}
impl LanSettings {
    pub fn read() -> Result<Self, String> {
        let read = |key| ubus::uci_get(key);
        Ok(Self {
            ipaddr: read("zwrt_router.network.lan_ipaddr")?,
            netmask: read("zwrt_router.dhcp.lan_netmask")?,
            dhcp_enabled: read("zwrt_router.dhcp.ignore")? != "1",
            dhcp_start: read("zwrt_router.dhcp.zte_start")?,
            dhcp_end: read("zwrt_router.dhcp.zte_end")?,
            lease_seconds: read("zwrt_router.dhcp.leasetime")?
                .parse()
                .map_err(|_| "invalid DHCP lease time")?,
        })
    }
    fn apply(&self) -> Result<(), String> {
        ubus::call(
            "zwrt_router.api",
            "router_set_lan_para",
            Some(
                &json!({
                    "ipaddr": self.ipaddr, "netmask": self.netmask,
                    "ignore": if self.dhcp_enabled {0} else {1}, "zte_start": self.dhcp_start,
                    "zte_end": self.dhcp_end, "leasetime": self.lease_seconds.to_string(),
                })
                .to_string(),
            ),
        )?;
        if Self::read()? != *self {
            return Err("firmware did not confirm the requested LAN settings".into());
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Journal {
    previous: LanSettings,
    proposed: LanSettings,
    token: String,
}
struct Pending {
    journal: Journal,
    deadline: Instant,
    applied: bool,
}
#[derive(Default)]
struct State {
    pending: Option<Pending>,
    confirmed: Option<Journal>,
    error: Option<String>,
}

trait Backend: Send + Sync {
    fn read(&self) -> Result<LanSettings, String>;
    fn apply(&self, settings: &LanSettings) -> Result<(), String>;
    fn load(&self) -> Result<Option<Journal>, String>;
    fn save(&self, journal: &Journal) -> Result<(), String>;
    fn clear(&self) -> Result<(), String>;
}
struct Hardware;
impl Backend for Hardware {
    fn read(&self) -> Result<LanSettings, String> {
        LanSettings::read()
    }
    fn apply(&self, settings: &LanSettings) -> Result<(), String> {
        settings.apply()
    }
    fn load(&self) -> Result<Option<Journal>, String> {
        match fs::read(JOURNAL) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| format!("invalid LAN recovery record: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("cannot read LAN recovery record: {e}")),
        }
    }
    fn save(&self, journal: &Journal) -> Result<(), String> {
        let bytes = serde_json::to_vec(journal).map_err(|e| e.to_string())?;
        storage::atomic_write(Path::new(JOURNAL), &bytes)
            .map_err(|e| format!("saving LAN recovery record: {e}"))
    }
    fn clear(&self) -> Result<(), String> {
        storage::durable_remove(Path::new(JOURNAL))
            .map_err(|e| format!("clearing LAN recovery record: {e}"))
    }
}

/// Called after every attempt to write LAN settings, so caches of what they
/// affect (the client list) are re-read.
pub type OnChange = Box<dyn Fn() + Send + Sync>;

pub struct LanManager {
    state: Mutex<State>,
    binding: Arc<Binding>,
    backend: Arc<dyn Backend>,
    on_change: OnChange,
}
impl LanManager {
    pub fn new(binding: Arc<Binding>, on_change: OnChange) -> Self {
        Self {
            state: Mutex::default(),
            binding,
            backend: Arc::new(Hardware),
            on_change,
        }
    }

    /// `backend.apply`, then the invalidation hook, whatever the outcome: a
    /// failed apply may still have changed some settings.
    fn apply(&self, settings: &LanSettings) -> Result<(), String> {
        let result = self.backend.apply(settings);
        (self.on_change)();
        result
    }

    /// Before starting the listener, recover any change that was never confirmed.
    /// A failed recovery remains recorded and blocks new changes.
    pub fn recover(&self) {
        let result = (|| {
            let Some(journal) = self.backend.load()? else {
                return Ok(());
            };
            crate::router::validate_lan_settings(&journal.previous)?;
            self.apply(&journal.previous)?;
            self.binding.set(&journal.previous.ipaddr);
            self.backend.clear()
        })();
        let mut state = self.state.safe_lock();
        if result.is_ok() {
            state.pending = None;
        }
        state.error = result.err();
    }

    pub fn begin(self: &Arc<Self>, proposed: LanSettings) -> Result<Value, String> {
        let mut state = self.state.safe_lock();
        if state.pending.is_some() || self.backend.load()?.is_some() {
            return Err(
                "a LAN change or recovery is pending; confirm it or restore connectivity first"
                    .into(),
            );
        }
        crate::router::validate_lan_settings(&proposed)?;
        let previous = self.backend.read()?;
        crate::router::validate_lan_settings(&previous)?;
        if previous == proposed {
            return Ok(json!({"changed": false}));
        }
        if self.binding.fixed && previous.ipaddr != proposed.ipaddr {
            return Err("LAN address changes require automatic LAN binding; remove the ZTE_AGENT_BIND override first".into());
        }
        let mut random = [0; 16];
        use std::io::Read;
        fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut random))
            .map_err(|e| format!("creating LAN confirmation token: {e}"))?;
        let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let journal = Journal {
            previous,
            proposed,
            token: token.clone(),
        };
        self.backend.save(&journal)?;
        let response = json!({"changed": true, "reconnect_ip": journal.proposed.ipaddr, "confirmation_token": token, "confirm_within_secs": CONFIRM_SECONDS});
        state.confirmed = None;
        state.pending = Some(Pending {
            journal,
            deadline: Instant::now() + Duration::from_secs(CONFIRM_SECONDS),
            applied: false,
        });
        #[cfg(not(test))]
        {
            let manager = self.clone();
            std::thread::spawn(move || manager.transition());
        }
        Ok(response)
    }

    #[cfg(not(test))]
    fn transition(&self) {
        std::thread::sleep(Duration::from_secs(1)); // Respond before interrupting LAN.
        self.apply_pending();
        loop {
            std::thread::sleep(Duration::from_millis(250));
            if !self.expire(Instant::now()) {
                return;
            }
        }
    }
    fn apply_pending(&self) {
        let mut state = self.state.safe_lock();
        let Some(pending) = state.pending.as_mut() else {
            return;
        };
        match self.apply(&pending.journal.proposed) {
            Ok(()) => {
                self.binding.set(&pending.journal.proposed.ipaddr);
                pending.applied = true;
            }
            Err(error) => {
                state.error = Some(error);
                self.rollback_locked(&mut state);
            }
        }
    }
    fn expire(&self, now: Instant) -> bool {
        let mut state = self.state.safe_lock();
        let Some(pending) = &state.pending else {
            return false;
        };
        if now >= pending.deadline {
            state.error = Some("LAN change was not confirmed; restoring previous settings".into());
            self.rollback_locked(&mut state);
            return false;
        }
        true
    }
    fn rollback_locked(&self, state: &mut State) {
        let Some(pending) = &state.pending else {
            return;
        };
        match self.apply(&pending.journal.previous) {
            Ok(()) => {
                self.binding.set(&pending.journal.previous.ipaddr);
                match self.backend.clear() {
                    Ok(()) => {
                        state.pending = None;
                    }
                    Err(error) => {
                        state.error = Some(format!(
                            "LAN restored but recovery record could not be cleared: {error}"
                        ));
                    }
                }
            }
            Err(error) => {
                state.error = Some(format!(
                    "LAN restoration failed: {error}; recovery will be retried on agent restart"
                ));
            }
        }
    }
    pub fn confirm(&self, token: &str) -> Result<(), String> {
        let mut state = self.state.safe_lock();
        if state.pending.is_none() {
            if let Some(confirmed) = &state.confirmed {
                if token == confirmed.token && self.backend.read()? == confirmed.proposed {
                    return Ok(());
                }
            }
        }
        let pending = state.pending.as_ref().ok_or("no LAN change is pending")?;
        if !pending.applied || Instant::now() >= pending.deadline || token != pending.journal.token
        {
            return Err("LAN confirmation is invalid or expired".into());
        }
        if self.backend.read()? != pending.journal.proposed {
            return Err("LAN settings no longer match the proposed change".into());
        }
        self.backend.clear()?;
        state.confirmed = Some(pending.journal.clone());
        state.pending = None;
        state.error = None;
        Ok(())
    }
    pub fn status(&self) -> Value {
        let state = self.state.safe_lock();
        json!({"pending": state.pending.is_some(), "last_error": state.error,
            "remaining_secs": state.pending.as_ref().map(|p| p.deadline.saturating_duration_since(Instant::now()).as_secs())})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Faults {
        apply: usize,
        save: bool,
        clear: bool,
    }
    struct Fake {
        settings: Mutex<LanSettings>,
        journal: Mutex<Option<Journal>>,
        faults: Mutex<Faults>,
    }
    impl Backend for Fake {
        fn read(&self) -> Result<LanSettings, String> {
            Ok(self.settings.safe_lock().clone())
        }
        fn apply(&self, settings: &LanSettings) -> Result<(), String> {
            // Model firmware that partially mutates before reporting failure.
            *self.settings.safe_lock() = settings.clone();
            let mut faults = self.faults.safe_lock();
            if faults.apply > 0 {
                faults.apply -= 1;
                return Err("injected apply failure".into());
            }
            Ok(())
        }
        fn load(&self) -> Result<Option<Journal>, String> {
            Ok(self.journal.safe_lock().clone())
        }
        fn save(&self, journal: &Journal) -> Result<(), String> {
            if self.faults.safe_lock().save {
                return Err("injected save failure".into());
            }
            *self.journal.safe_lock() = Some(journal.clone());
            Ok(())
        }
        fn clear(&self) -> Result<(), String> {
            if self.faults.safe_lock().clear {
                return Err("injected clear failure".into());
            }
            *self.journal.safe_lock() = None;
            Ok(())
        }
    }
    fn settings(subnet: u8) -> LanSettings {
        LanSettings {
            ipaddr: format!("192.168.{subnet}.1"),
            netmask: "255.255.255.0".into(),
            dhcp_enabled: true,
            dhcp_start: format!("192.168.{subnet}.100"),
            dhcp_end: format!("192.168.{subnet}.200"),
            lease_seconds: 86400,
        }
    }
    fn fixture() -> (Arc<LanManager>, Arc<Fake>) {
        fixture_with(Box::new(|| {}))
    }
    fn fixture_with(on_change: OnChange) -> (Arc<LanManager>, Arc<Fake>) {
        let backend = Arc::new(Fake {
            settings: Mutex::new(settings(0)),
            journal: Mutex::new(None),
            faults: Mutex::default(),
        });
        let binding = Arc::new(Binding {
            address: Mutex::new("192.168.0.1:9090".into()),
            generation: AtomicU64::new(0),
            fixed: false,
        });
        (
            Arc::new(LanManager {
                state: Mutex::default(),
                binding,
                backend: backend.clone(),
                on_change,
            }),
            backend,
        )
    }
    #[test]
    fn confirmation_requires_applied_matching_settings_and_token() {
        let (manager, backend) = fixture();
        let response = manager.begin(settings(1)).unwrap();
        let token = response["confirmation_token"].as_str().unwrap();
        assert!(manager.confirm(token).is_err());
        assert!(manager.begin(settings(2)).is_err());
        manager.apply_pending();
        assert_eq!(manager.binding.address(), "192.168.1.1:9090");
        assert!(manager.confirm("wrong token").is_err());
        manager.confirm(token).unwrap();
        manager.confirm(token).unwrap(); // Retry after a lost acknowledgement.
        assert!(backend.load().unwrap().is_none());
        assert!(!manager.expire(Instant::now() + Duration::from_secs(121)));
        assert_eq!(backend.read().unwrap(), settings(1));
    }
    #[test]
    fn every_apply_and_rollback_invalidates_dependent_caches() {
        use std::sync::atomic::AtomicUsize;
        let changes = Arc::new(AtomicUsize::new(0));
        let counter = changes.clone();
        let (manager, _backend) = fixture_with(Box::new(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        }));
        manager.begin(settings(1)).unwrap();
        assert_eq!(changes.load(Ordering::Relaxed), 0, "nothing applied yet");
        manager.apply_pending();
        assert_eq!(changes.load(Ordering::Relaxed), 1);
        manager.expire(Instant::now() + Duration::from_secs(121));
        assert_eq!(changes.load(Ordering::Relaxed), 2, "rollback");
    }
    #[test]
    fn timeout_restores_configuration_and_listener() {
        let (manager, backend) = fixture();
        manager.begin(settings(1)).unwrap();
        manager.apply_pending();
        manager.expire(Instant::now() + Duration::from_secs(121));
        assert_eq!(backend.read().unwrap(), settings(0));
        assert_eq!(manager.binding.address(), "192.168.0.1:9090");
        assert!(backend.load().unwrap().is_none());
        assert!(manager.status()["last_error"].is_string());
        manager.begin(settings(2)).unwrap();
    }
    #[test]
    fn failed_save_never_changes_firmware_and_partial_apply_rolls_back() {
        let (manager, backend) = fixture();
        backend.faults.safe_lock().save = true;
        assert!(manager.begin(settings(1)).is_err());
        assert_eq!(backend.read().unwrap(), settings(0));
        backend.faults.safe_lock().save = false;
        backend.faults.safe_lock().apply = 1;
        manager.begin(settings(1)).unwrap();
        manager.apply_pending();
        assert_eq!(backend.read().unwrap(), settings(0));
        assert!(backend.load().unwrap().is_none());
    }
    #[test]
    fn failed_restore_keeps_journal_for_restart_recovery() {
        let (manager, backend) = fixture();
        backend.faults.safe_lock().apply = 2;
        manager.begin(settings(1)).unwrap();
        manager.apply_pending();
        assert!(backend.load().unwrap().is_some());
        assert!(manager.begin(settings(2)).is_err());
        manager.recover();
        assert!(backend.load().unwrap().is_none());
        assert_eq!(backend.read().unwrap(), settings(0));
    }
    #[test]
    fn failed_confirmation_storage_keeps_recovery_record() {
        let (manager, backend) = fixture();
        let response = manager.begin(settings(1)).unwrap();
        manager.apply_pending();
        backend.faults.safe_lock().clear = true;
        assert!(manager
            .confirm(response["confirmation_token"].as_str().unwrap())
            .is_err());
        assert!(backend.load().unwrap().is_some());
        backend.faults.safe_lock().clear = false;
        manager.recover();
        assert_eq!(backend.read().unwrap(), settings(0));
    }
}
