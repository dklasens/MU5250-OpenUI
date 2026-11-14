use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::cache::{Cached, Observed, Sample};
use crate::util::MutexExt;

/// Shortest window a CPU delta is computed over. Both `/api/cpu` and the
/// dashboard batch call `sample()`; without this each would reset the other's
/// baseline, so the percentages jittered according to who asked last.
const CPU_MIN_INTERVAL: Duration = Duration::from_secs(2);

/// Previous CPU sample for delta calculation.
pub struct CpuTracker {
    prev: Mutex<Vec<CpuSample>>,
    recent: Cached<CpuUsage>,
}

struct CpuSample {
    user: u64,
    nice: u64,
    system: u64,
    idle: u64,
    iowait: u64,
    irq: u64,
    softirq: u64,
}

impl CpuSample {
    fn total(&self) -> u64 {
        self.user + self.nice + self.system + self.idle + self.iowait + self.irq + self.softirq
    }

    fn busy(&self) -> u64 {
        self.total() - self.idle - self.iowait
    }
}

#[derive(Serialize, Clone)]
pub struct CpuUsage {
    pub cores: Vec<f64>,
    pub overall: f64,
}

impl CpuTracker {
    pub fn new() -> Self {
        Self {
            prev: Mutex::new(Vec::new()),
            recent: Cached::default(),
        }
    }

    pub fn sample(&self) -> CpuUsage {
        self.recent
            .get_or_refresh(CPU_MIN_INTERVAL, || self.sample_fresh())
    }

    /// Take a baseline reading without publishing it, so the first real
    /// `sample()` measures a meaningful window rather than a few milliseconds.
    pub fn seed(&self) {
        let _ = self.sample_fresh();
    }

    fn sample_fresh(&self) -> CpuUsage {
        let current = read_cpu_samples();
        let mut prev = self.prev.safe_lock();

        let mut cores = Vec::new();
        let mut total_busy: u64 = 0;
        let mut total_all: u64 = 0;

        for (i, cur) in current.iter().enumerate() {
            if let Some(old) = prev.get(i) {
                let dt = cur.total().saturating_sub(old.total());
                let db = cur.busy().saturating_sub(old.busy());
                if dt > 0 {
                    let pct = (db as f64 / dt as f64) * 100.0;
                    cores.push((pct * 10.0).round() / 10.0);
                    total_busy += db;
                    total_all += dt;
                } else {
                    cores.push(0.0);
                }
            } else {
                cores.push(0.0);
            }
        }

        let overall = if total_all > 0 {
            let pct = (total_busy as f64 / total_all as f64) * 100.0;
            (pct * 10.0).round() / 10.0
        } else {
            0.0
        };

        *prev = current;
        CpuUsage { cores, overall }
    }
}

fn read_cpu_samples() -> Vec<CpuSample> {
    let content = match fs::read_to_string("/proc/stat") {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut samples = Vec::new();
    for line in content.lines() {
        // Match "cpu0", "cpu1", etc. but not the aggregate "cpu " line
        if line.starts_with("cpu") && line.as_bytes().get(3).is_some_and(|b| b.is_ascii_digit()) {
            let parts: Vec<u64> = line
                .split_whitespace()
                .skip(1)
                .filter_map(|s| s.parse().ok())
                .collect();
            if parts.len() >= 7 {
                samples.push(CpuSample {
                    user: parts[0],
                    nice: parts[1],
                    system: parts[2],
                    idle: parts[3],
                    iowait: parts[4],
                    irq: parts[5],
                    softirq: parts[6],
                });
            }
        }
    }
    samples
}

#[derive(Serialize)]
pub struct MemInfo {
    pub total_kb: u64,
    pub free_kb: u64,
    pub available_kb: u64,
    pub buffers_kb: u64,
    pub cached_kb: u64,
    pub used_kb: u64,
    pub usage_pct: f64,
}

pub fn read_meminfo() -> Option<MemInfo> {
    let content = fs::read_to_string("/proc/meminfo").ok()?;
    let mut map: HashMap<String, u64> = HashMap::new();
    for line in content.lines() {
        let mut parts = line.split(':');
        let key = parts.next()?.trim().to_string();
        let val_str = parts.next()?.trim();
        let val: u64 = val_str.split_whitespace().next()?.parse().ok()?;
        map.insert(key, val);
    }
    let total = *map.get("MemTotal")?;
    let free = *map.get("MemFree").unwrap_or(&0);
    let available = *map.get("MemAvailable").unwrap_or(&free);
    let buffers = *map.get("Buffers").unwrap_or(&0);
    let cached = *map.get("Cached").unwrap_or(&0);
    let used = total.saturating_sub(available);
    let usage_pct = if total > 0 {
        ((used as f64 / total as f64) * 1000.0).round() / 10.0
    } else {
        0.0
    };
    Some(MemInfo {
        total_kb: total,
        free_kb: free,
        available_kb: available,
        buffers_kb: buffers,
        cached_kb: cached,
        used_kb: used,
        usage_pct,
    })
}

#[derive(Serialize)]
pub struct DeviceInfo {
    pub hostname: String,
    pub uptime_secs: u64,
    pub load_avg: [f64; 3],
    pub kernel: String,
}

pub fn read_device_info() -> DeviceInfo {
    let hostname = fs::read_to_string("/proc/sys/kernel/hostname")
        .unwrap_or_default()
        .trim()
        .to_string();

    let uptime_str = fs::read_to_string("/proc/uptime").unwrap_or_default();
    let uptime_secs = uptime_str
        .split_whitespace()
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0) as u64;

    let loadavg_str = fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let parts: Vec<f64> = loadavg_str
        .split_whitespace()
        .take(3)
        .filter_map(|s| s.parse().ok())
        .collect();
    let load_avg = [
        parts.first().copied().unwrap_or(0.0),
        parts.get(1).copied().unwrap_or(0.0),
        parts.get(2).copied().unwrap_or(0.0),
    ];

    let kernel = fs::read_to_string("/proc/version")
        .unwrap_or_default()
        .trim()
        .to_string();

    DeviceInfo {
        hostname,
        uptime_secs,
        load_avg,
        kernel,
    }
}

#[derive(Serialize)]
pub struct BatteryInfo {
    pub status: String,
    pub capacity: i64,
    pub voltage_uv: i64,
    pub current_ua: i64,
    pub temperature: i64,
    /// USB supply present (`power_supply/usb/online`). Status alone cannot
    /// tell "plugged in, charging paused" from "on battery".
    pub external_power: Option<bool>,
}

pub fn read_battery() -> Option<BatteryInfo> {
    let base = "/sys/class/power_supply/battery";
    let read_str = |name: &str| -> String {
        fs::read_to_string(format!("{base}/{name}"))
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let read_i64 = |name: &str| -> i64 { read_str(name).parse().unwrap_or(0) };

    let status = read_str("status");
    if status.is_empty() {
        return None;
    }
    Some(BatteryInfo {
        status,
        capacity: read_i64("capacity"),
        voltage_uv: read_i64("voltage_now"),
        current_ua: read_i64("current_now"),
        temperature: read_i64("temp"),
        external_power: fs::read_to_string("/sys/class/power_supply/usb/online")
            .ok()
            .and_then(|v| match v.trim() {
                "1" => Some(true),
                "0" => Some(false),
                _ => None,
            }),
    })
}

// -- WAN throughput (modem data counters) --

/// Minimum spacing between counter reads. Each read is one `ubus` call, and
/// concurrent dashboard clients share it.
const SPEED_TTL: Duration = Duration::from_secs(1);
/// A previous sample older than this no longer describes the current rate (the
/// dashboard was idle), so the firmware's own one-second rate is used instead.
const SPEED_MAX_WINDOW: Duration = Duration::from_secs(10);

/// Live WAN throughput. Field names are part of the dashboard contract —
/// `web-app/src/data/api.ts::mapSpeed` reads them verbatim.
#[derive(Serialize, Clone)]
pub struct SpeedSnapshot {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_speed: f64,
    pub tx_speed: f64,
    /// Highest one-second rate the modem has seen on this data connection.
    pub max_rx_speed: f64,
    pub max_tx_speed: f64,
    pub elapsed_ms: u64,
}

/// `zwrt_data get_wwandst` "real" (current connection) counters.
#[derive(Clone, Copy)]
struct WanCounters {
    rx_bytes: u64,
    tx_bytes: u64,
    /// Bytes transferred in the modem's last one-second window.
    rx_speed: u64,
    tx_speed: u64,
    max_rx_speed: u64,
    max_tx_speed: u64,
}

/// Throughput from the modem's own data counters.
///
/// The `rmnet_*` netdev counters this used to sample miss most tethered
/// traffic: IPA hardware offload forwards it without touching the host stack
/// (on-device they read ~17x less than the modem counter). The modem's
/// `real_*` counters include offloaded traffic.
pub struct SpeedTracker {
    source: Observed<SpeedSnapshot>,
    previous: Mutex<Option<(Instant, WanCounters)>>,
    /// The last raw `get_wwandst` reply, for data usage to reuse.
    last_reply: Mutex<Option<(Instant, Value)>>,
}

impl SpeedTracker {
    pub fn new() -> Self {
        Self {
            source: Observed::default(),
            previous: Mutex::new(None),
            last_reply: Mutex::new(None),
        }
    }

    pub fn sample(&self) -> Sample<SpeedSnapshot> {
        self.source.read(SPEED_TTL, || {
            let stats = read_wwandst()?;
            *self.last_reply.safe_lock() = Some((Instant::now(), stats.clone()));
            let current = wan_counters(&stats)?;
            let now = Instant::now();
            let mut previous = self.previous.safe_lock();
            let snapshot = speed_snapshot(previous.as_ref(), now, &current);
            *previous = Some((now, current));
            Ok(snapshot)
        })
    }

    /// The speed source's last `get_wwandst` reply, if it is at most `max_age`
    /// old. Data usage issues the identical call, so it reuses this one.
    pub fn recent_wwandst(&self, max_age: Duration) -> Option<Value> {
        self.last_reply
            .safe_lock()
            .as_ref()
            .filter(|(at, _)| at.elapsed() <= max_age)
            .map(|(_, stats)| stats.clone())
    }
}

/// `zwrt_data get_wwandst` for the web module's cid 1: the WAN counters behind
/// both the throughput and the data-usage readings.
pub fn read_wwandst() -> Result<Value, String> {
    crate::ubus::call_poll(
        "zwrt_data",
        "get_wwandst",
        Some(r#"{"source_module":"web","cid":1,"type":4}"#),
    )
}

fn wan_counters(stats: &Value) -> Result<WanCounters, String> {
    let counter = |key: &str| -> Result<u64, String> {
        match stats.get(key) {
            Some(Value::Number(n)) => n.as_u64(),
            Some(Value::String(s)) => s.trim().parse().ok(),
            _ => None,
        }
        .ok_or_else(|| format!("get_wwandst: {key} unavailable"))
    };
    Ok(WanCounters {
        rx_bytes: counter("real_rx_bytes")?,
        tx_bytes: counter("real_tx_bytes")?,
        rx_speed: counter("real_rx_speed")?,
        tx_speed: counter("real_tx_speed")?,
        max_rx_speed: counter("real_max_rx_speed").unwrap_or(0),
        max_tx_speed: counter("real_max_tx_speed").unwrap_or(0),
    })
}

/// Average rate since the previous sample, which smooths the modem's spiky
/// one-second figure across the dashboard's poll interval. Falls back to that
/// figure when there is no recent sample or the counters reset (reconnect).
fn speed_snapshot(
    previous: Option<&(Instant, WanCounters)>,
    now: Instant,
    current: &WanCounters,
) -> SpeedSnapshot {
    let window = previous.and_then(|(at, prev)| {
        let elapsed = now.saturating_duration_since(*at);
        (elapsed > Duration::ZERO
            && elapsed <= SPEED_MAX_WINDOW
            && current.rx_bytes >= prev.rx_bytes
            && current.tx_bytes >= prev.tx_bytes)
            .then_some((elapsed, prev))
    });
    let (rx_speed, tx_speed, elapsed_ms) = match window {
        Some((elapsed, prev)) => {
            let secs = elapsed.as_secs_f64();
            (
                (current.rx_bytes - prev.rx_bytes) as f64 / secs,
                (current.tx_bytes - prev.tx_bytes) as f64 / secs,
                elapsed.as_millis() as u64,
            )
        }
        None => (current.rx_speed as f64, current.tx_speed as f64, 1000),
    };
    SpeedSnapshot {
        rx_bytes: current.rx_bytes,
        tx_bytes: current.tx_bytes,
        rx_speed,
        tx_speed,
        max_rx_speed: (current.max_rx_speed as f64).max(rx_speed),
        max_tx_speed: (current.max_tx_speed as f64).max(tx_speed),
        elapsed_ms,
    }
}

// -- Firmware identity --

/// Retry spacing while the firmware identity cannot be read (early boot).
const FIRMWARE_RETRY: Duration = Duration::from_secs(60);

#[derive(Clone, Serialize)]
pub struct FirmwareIdentity {
    /// ZTE build, e.g. `XCBZ_HK_MU5250V1.0.0B04`.
    pub version: Option<String>,
    pub hardware: Option<String>,
}

/// Firmware build from `zwrt_zte_mdm.api get_zwrt_common_info`. It cannot
/// change without a reboot, so the first successful read is kept for the life
/// of the process.
pub struct FirmwareInfo {
    state: Mutex<(Option<FirmwareIdentity>, Option<Instant>)>,
}

impl FirmwareInfo {
    pub fn new() -> Self {
        Self {
            state: Mutex::new((None, None)),
        }
    }

    pub fn get(&self) -> Option<FirmwareIdentity> {
        let mut state = self.state.safe_lock();
        if state.0.is_some() {
            return state.0.clone();
        }
        if state.1.is_some_and(|at| at.elapsed() < FIRMWARE_RETRY) {
            return None;
        }
        // Claim the attempt, then release the lock for the subprocess: this
        // runs on the dashboard heartbeat, and a hung daemon must not queue
        // every worker behind it. Concurrent callers see the fresh attempt
        // and return None, as they would after a failed read.
        state.1 = Some(Instant::now());
        drop(state);
        let info =
            crate::ubus::call_poll("zwrt_zte_mdm.api", "get_zwrt_common_info", Some("{}")).ok()?;
        let text = |key: &str| {
            info.get(key)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        let identity = FirmwareIdentity {
            version: text("integrate_version"),
            hardware: text("hardware_version"),
        };
        if identity.version.is_some() {
            self.state.safe_lock().0 = Some(identity.clone());
        }
        Some(identity)
    }
}

// -- Process monitor --

/// Optional services documented as safe to stop in `docs/SAFETY.md`.
///
/// This allowlist is only the first safety gate.  At runtime we also parse the
/// firmware's daemon synchronization barrier and exclude every process it
/// names.  If that file cannot be read, stopping services fails closed.
const SAFE_OPTIONAL_DAEMONS: &[&str] = &[
    "zte_topsw_tr069_sub",
    "zte_mqtt_sdk_st",
    "zte_topsw_diag",
    "zte_topsw_samba",
    "zte_topsw_nfc",
    "zte_topsw_get_brand",
    "zte_topsw_jwxk_query",
    "zte-topsw-tunnel",
    "zte_topsw_dua",
];

const DAEMON_SYNC_CONFIG: &str = "/etc/config/zte_topsw_daemon.conf";

fn parse_daemon_sync_config(content: &str) -> HashSet<String> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let columns: Vec<&str> = line.split_whitespace().collect();
            if columns.first().copied() == Some("sync") && columns.len() >= 3 {
                columns.last().map(|name| (*name).to_string())
            } else {
                None
            }
        })
        .collect()
}

fn protected_daemons(path: &str) -> Result<HashSet<String>, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("cannot verify firmware daemon safety barrier: {e}"))?;
    let protected = parse_daemon_sync_config(&content);
    if protected.is_empty() {
        return Err("firmware daemon safety barrier contained no sync entries".to_string());
    }
    Ok(protected)
}

fn is_safe_optional_daemon(name: &str, protected: &HashSet<String>) -> bool {
    SAFE_OPTIONAL_DAEMONS.contains(&name) && !protected.contains(name)
}

#[derive(Serialize, Clone)]
pub struct ProcessEntry {
    pub pid: u32,
    pub name: String,
    pub cpu_pct: f64,
    pub rss_kb: u64,
    pub state: String,
    pub is_bloat: bool,
}

#[derive(Serialize, Clone)]
pub struct ProcessListResult {
    pub processes: Vec<ProcessEntry>,
    pub total_count: usize,
    pub bloat_count: usize,
    pub bloat_cpu_pct: f64,
    pub bloat_rss_kb: u64,
}

#[derive(Serialize)]
pub struct KilledProcess {
    pub pid: u32,
    pub name: String,
}

#[derive(Serialize)]
pub struct KillBloatResult {
    pub killed: Vec<KilledProcess>,
    pub skipped: Vec<KilledProcess>,
    pub freed_rss_kb: u64,
}

/// A /proc scan is reused for this long. CPU% is computed over the window
/// between scans, so it is never shorter than this either.
const PROCESS_TTL: Duration = Duration::from_secs(3);

pub struct ProcessTracker {
    prev: Mutex<(HashMap<u32, u64>, u64)>,
    /// Single-flight: concurrent callers share one scan instead of each
    /// scanning /proc, which also left the second computing CPU% over a
    /// near-zero window.
    cache: Cached<ProcessListResult>,
}

impl ProcessTracker {
    pub fn new() -> Self {
        Self {
            prev: Mutex::new((HashMap::new(), 0)),
            cache: Cached::default(),
        }
    }

    pub fn sample(&self) -> ProcessListResult {
        self.sample_with(|| self.sample_fresh())
    }

    fn sample_with(&self, scan: impl FnOnce() -> ProcessListResult) -> ProcessListResult {
        self.cache.get_or_refresh(PROCESS_TTL, scan)
    }

    fn sample_fresh(&self) -> ProcessListResult {
        // An unreadable or malformed barrier deliberately produces no
        // stop-eligible processes in the UI.
        let protected = protected_daemons(DAEMON_SYNC_CONFIG).ok();
        let total_ticks = read_total_cpu_ticks();
        let mut prev = self.prev.safe_lock();
        let (prev_per_pid, prev_total) = &*prev;
        let dt = total_ticks.saturating_sub(*prev_total);

        let mut entries: Vec<ProcessEntry> = Vec::new();
        let mut new_per_pid: HashMap<u32, u64> = HashMap::new();

        let proc_dir = match fs::read_dir("/proc") {
            Ok(d) => d,
            Err(_) => {
                return ProcessListResult {
                    processes: Vec::new(),
                    total_count: 0,
                    bloat_count: 0,
                    bloat_cpu_pct: 0.0,
                    bloat_rss_kb: 0,
                };
            }
        };

        for entry in proc_dir.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let pid: u32 = match name_str.parse() {
                Ok(p) => p,
                Err(_) => continue,
            };

            let stat_path = format!("/proc/{pid}/stat");
            let stat_str = match fs::read_to_string(&stat_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            // Parse /proc/PID/stat — comm is in parens, fields after closing paren
            let comm_start = match stat_str.find('(') {
                Some(i) => i + 1,
                None => continue,
            };
            let comm_end = match stat_str.rfind(')') {
                Some(i) => i,
                None => continue,
            };
            // A malformed line (no space after ')', or ')' before '(') is
            // skipped rather than allowed to panic on the slice bounds.
            let (Some(comm), Some(after_comm)) = (
                stat_str.get(comm_start..comm_end),
                stat_str.get(comm_end + 2..), // skip ") "
            ) else {
                continue;
            };
            let comm = comm.to_string();
            let fields: Vec<&str> = after_comm.split_whitespace().collect();
            // fields[0]=state, fields[11]=utime, fields[12]=stime, fields[21]=rss(pages)
            if fields.len() < 22 {
                continue;
            }
            let state_char = fields[0].to_string();
            let utime: u64 = fields[11].parse().unwrap_or(0);
            let stime: u64 = fields[12].parse().unwrap_or(0);
            let proc_ticks = utime + stime;
            let rss_pages: u64 = fields[21].parse().unwrap_or(0);
            let rss_kb = rss_pages * 4; // page size = 4K

            // CPU% delta
            let cpu_pct = if dt > 0 {
                let prev_ticks = prev_per_pid.get(&pid).copied().unwrap_or(proc_ticks);
                let dp = proc_ticks.saturating_sub(prev_ticks);
                let pct = (dp as f64 / dt as f64) * 100.0;
                (pct * 10.0).round() / 10.0
            } else {
                0.0
            };

            new_per_pid.insert(pid, proc_ticks);

            // Read cmdline for better name matching
            let cmdline_name = fs::read_to_string(format!("/proc/{pid}/cmdline"))
                .ok()
                .and_then(|s| {
                    let clean = s.replace('\0', " ");
                    let first = clean.split_whitespace().next()?.to_string();
                    let basename = first.rsplit('/').next().unwrap_or(&first).to_string();
                    if basename.is_empty() {
                        None
                    } else {
                        Some(basename)
                    }
                });

            let display_name = cmdline_name.as_deref().unwrap_or(&comm);
            let is_bloat = protected
                .as_ref()
                .is_some_and(|names| is_safe_optional_daemon(display_name, names));

            let state_desc = match state_char.as_str() {
                "R" => "running",
                "S" => "sleeping",
                "D" => "disk",
                "Z" => "zombie",
                "T" => "stopped",
                _ => "other",
            };

            entries.push(ProcessEntry {
                pid,
                name: display_name.to_string(),
                cpu_pct,
                rss_kb,
                state: state_desc.to_string(),
                is_bloat,
            });
        }

        // Update stored state
        *prev = (new_per_pid, total_ticks);

        // Sort by CPU% desc
        entries.sort_by(|a, b| {
            b.cpu_pct
                .partial_cmp(&a.cpu_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let total_count = entries.len();
        let bloat_count = entries.iter().filter(|e| e.is_bloat).count();
        let bloat_cpu_pct = entries
            .iter()
            .filter(|e| e.is_bloat)
            .map(|e| e.cpu_pct)
            .sum::<f64>();
        let bloat_cpu_pct = (bloat_cpu_pct * 10.0).round() / 10.0;
        let bloat_rss_kb: u64 = entries
            .iter()
            .filter(|e| e.is_bloat)
            .map(|e| e.rss_kb)
            .sum();

        // Keep top 50
        entries.truncate(50);

        ProcessListResult {
            processes: entries,
            total_count,
            bloat_count,
            bloat_cpu_pct,
            bloat_rss_kb,
        }
    }
}

fn read_total_cpu_ticks() -> u64 {
    let content = match fs::read_to_string("/proc/stat") {
        Ok(c) => c,
        Err(_) => return 0,
    };
    for line in content.lines() {
        if line.starts_with("cpu ") {
            return line
                .split_whitespace()
                .skip(1)
                .filter_map(|s| s.parse::<u64>().ok())
                .sum();
        }
    }
    0
}

/// Gracefully stop optional daemons. If `pids` is None, stop every running
/// allowlisted process. The firmware sync barrier is always authoritative.
pub fn kill_bloat(pids: Option<&[u32]>) -> Result<KillBloatResult, String> {
    let protected = protected_daemons(DAEMON_SYNC_CONFIG)?;
    let mut killed = Vec::new();
    let mut skipped = Vec::new();
    let mut freed_rss_kb: u64 = 0;

    let targets: Vec<(u32, String, u64)> = match pids {
        Some(pid_list) => pid_list
            .iter()
            .filter_map(|&pid| {
                let (name, rss) = read_proc_name_rss(pid)?;
                Some((pid, name, rss))
            })
            .collect(),
        None => {
            // Find all bloat processes
            let proc_dir = match fs::read_dir("/proc") {
                Ok(d) => d,
                Err(_) => {
                    return Ok(KillBloatResult {
                        killed,
                        skipped,
                        freed_rss_kb,
                    })
                }
            };
            proc_dir
                .flatten()
                .filter_map(|entry| {
                    let pid: u32 = entry.file_name().to_string_lossy().parse().ok()?;
                    let (name, rss) = read_proc_name_rss(pid)?;
                    if is_safe_optional_daemon(&name, &protected) {
                        Some((pid, name, rss))
                    } else {
                        None
                    }
                })
                .collect()
        }
    };

    for (pid, name, rss_kb) in targets {
        // Re-check the name read from /proc immediately before signalling it.
        // This protects explicit PID requests and PID reuse races alike.
        if !is_safe_optional_daemon(&name, &protected) {
            skipped.push(KilledProcess { pid, name });
            continue;
        }
        let ret = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        if ret == 0 {
            freed_rss_kb += rss_kb;
            killed.push(KilledProcess { pid, name });
        } else {
            skipped.push(KilledProcess { pid, name });
        }
    }

    Ok(KillBloatResult {
        killed,
        skipped,
        freed_rss_kb,
    })
}

fn read_proc_name_rss(pid: u32) -> Option<(String, u64)> {
    let stat_str = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let comm_end = stat_str.rfind(')')?;
    let after_comm = stat_str.get(comm_end + 2..)?;
    let fields: Vec<&str> = after_comm.split_whitespace().collect();
    let rss_pages: u64 = fields.get(21)?.parse().ok()?;

    let name = fs::read_to_string(format!("/proc/{pid}/cmdline"))
        .ok()
        .and_then(|s| {
            let clean = s.replace('\0', " ");
            let first = clean.split_whitespace().next()?.to_string();
            let basename = first.rsplit('/').next().unwrap_or(&first).to_string();
            if basename.is_empty() {
                None
            } else {
                Some(basename)
            }
        })
        .unwrap_or_else(|| {
            let comm_start = stat_str.find('(').unwrap_or(0) + 1;
            stat_str[comm_start..comm_end].to_string()
        });

    Some((name, rss_pages * 4))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These payloads are consumed directly by the dashboard, so their key sets
    /// are a contract. Renaming a field here means updating
    /// `web-app/src/data/api.ts` and `web-app/tools/mock_agent.py` in lockstep —
    /// silent drift between the two is what broke Home throughput and the
    /// process list before.
    fn keys(v: &serde_json::Value) -> Vec<String> {
        let mut k: Vec<String> = v.as_object().expect("object").keys().cloned().collect();
        k.sort();
        k
    }

    #[test]
    fn speed_snapshot_shape_is_stable() {
        let snap = SpeedSnapshot {
            rx_bytes: 1,
            tx_bytes: 2,
            rx_speed: 3.0,
            tx_speed: 4.0,
            max_rx_speed: 5.0,
            max_tx_speed: 6.0,
            elapsed_ms: 7,
        };
        let v = serde_json::to_value(&snap).unwrap();
        assert_eq!(
            keys(&v).iter().map(String::as_str).collect::<Vec<_>>(),
            [
                "elapsed_ms",
                "max_rx_speed",
                "max_tx_speed",
                "rx_bytes",
                "rx_speed",
                "tx_bytes",
                "tx_speed",
            ]
        );
    }

    fn counters(rx_bytes: u64, tx_bytes: u64) -> WanCounters {
        WanCounters {
            rx_bytes,
            tx_bytes,
            rx_speed: 7,
            tx_speed: 3,
            max_rx_speed: 1_000,
            max_tx_speed: 500,
        }
    }

    #[test]
    fn speed_is_averaged_over_the_sample_window() {
        let then = Instant::now();
        let now = then + Duration::from_secs(2);
        let snap = speed_snapshot(
            Some(&(then, counters(1_000, 100))),
            now,
            &counters(9_000, 700),
        );
        assert_eq!(snap.rx_speed, 4_000.0);
        assert_eq!(snap.tx_speed, 300.0);
        assert_eq!(snap.elapsed_ms, 2_000);
        // A window average above the modem's recorded peak raises the peak.
        assert_eq!(snap.max_rx_speed, 4_000.0);
        assert_eq!(snap.max_tx_speed, 500.0);
    }

    #[test]
    fn speed_falls_back_to_modem_rate_without_a_usable_window() {
        let then = Instant::now();
        let current = counters(9_000, 700);
        // First sample, stale sample, and counters reset by a reconnect.
        for previous in [
            None,
            Some((then, counters(1_000, 100))),
            Some((then + Duration::from_secs(29), counters(50_000, 100))),
        ] {
            let snap = speed_snapshot(previous.as_ref(), then + Duration::from_secs(30), &current);
            assert_eq!((snap.rx_speed, snap.tx_speed), (7.0, 3.0));
        }
    }

    #[test]
    fn process_list_shape_is_stable() {
        let result = ProcessListResult {
            processes: vec![ProcessEntry {
                pid: 1,
                name: "init".to_string(),
                cpu_pct: 0.5,
                rss_kb: 1200,
                state: "sleeping".to_string(),
                is_bloat: false,
            }],
            total_count: 1,
            bloat_count: 0,
            bloat_cpu_pct: 0.0,
            bloat_rss_kb: 0,
        };
        let v = serde_json::to_value(&result).unwrap();
        assert_eq!(
            keys(&v).iter().map(String::as_str).collect::<Vec<_>>(),
            [
                "bloat_count",
                "bloat_cpu_pct",
                "bloat_rss_kb",
                "processes",
                "total_count",
            ]
        );
        assert_eq!(
            keys(&v["processes"][0])
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["cpu_pct", "is_bloat", "name", "pid", "rss_kb", "state"]
        );
    }

    #[test]
    fn daemon_sync_parser_only_accepts_active_sync_rows() {
        let parsed = parse_daemon_sync_config(
            "# comment\nsync ALL zte_router\n sync  ALL  zte_topsw_wms \nstart ALL ignored\n",
        );
        assert_eq!(parsed.len(), 2);
        assert!(parsed.contains("zte_router"));
        assert!(parsed.contains("zte_topsw_wms"));
    }

    #[test]
    fn optional_daemon_must_not_cross_firmware_barrier() {
        let protected = HashSet::from(["zte_topsw_diag".to_string(), "zte_topsw_wms".to_string()]);
        assert!(!is_safe_optional_daemon("zte_topsw_diag", &protected));
        assert!(!is_safe_optional_daemon("zte_topsw_wms", &protected));
        assert!(is_safe_optional_daemon("zte_topsw_samba", &protected));
        assert!(!is_safe_optional_daemon(
            "zte_topsw_fota_result",
            &protected
        ));
        assert!(!is_safe_optional_daemon("zte_smart_manage", &protected));
    }

    fn empty_process_list() -> ProcessListResult {
        ProcessListResult {
            processes: Vec::new(),
            total_count: 0,
            bloat_count: 0,
            bloat_cpu_pct: 0.0,
            bloat_rss_kb: 0,
        }
    }

    #[test]
    fn concurrent_process_samples_share_one_scan() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let tracker = ProcessTracker::new();
        let scans = AtomicU32::new(0);
        std::thread::scope(|scope| {
            for _ in 0..6 {
                scope.spawn(|| {
                    let result = tracker.sample_with(|| {
                        scans.fetch_add(1, Ordering::Relaxed);
                        std::thread::sleep(Duration::from_millis(50));
                        ProcessListResult {
                            total_count: 7,
                            ..empty_process_list()
                        }
                    });
                    // Nobody had a list yet, so everyone waited for the one scan.
                    assert_eq!(result.total_count, 7);
                });
            }
        });
        assert_eq!(scans.load(Ordering::Relaxed), 1);
        // Within the TTL the scan is reused.
        let again = tracker.sample_with(|| panic!("rescanned within the TTL"));
        assert_eq!(again.total_count, 7);
    }

    #[test]
    fn wwandst_reply_is_shared_only_while_recent() {
        let speed = SpeedTracker::new();
        assert!(speed.recent_wwandst(Duration::from_secs(2)).is_none());
        let reply = serde_json::json!({"real_rx_bytes": 1, "day_rx_bytes": 2});
        *speed.last_reply.safe_lock() = Some((Instant::now(), reply.clone()));
        assert_eq!(speed.recent_wwandst(Duration::from_secs(2)), Some(reply));
        *speed.last_reply.safe_lock() = Some((
            Instant::now() - Duration::from_secs(3),
            serde_json::json!({}),
        ));
        assert!(speed.recent_wwandst(Duration::from_secs(2)).is_none());
    }
}
