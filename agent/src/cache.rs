use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::util::MutexExt;

/// Longest a refresh is retried after consecutive failures. A source whose TTL
/// is already longer keeps its TTL.
pub const BACKOFF_CAP: Duration = Duration::from_secs(30);
/// How long an `Observed` reader with nothing to show waits for a refresh that
/// another caller already started. Matches the poll deadline (`ubus::POLL_DEADLINE`),
/// so a healthy first read is normally waited out.
const FIRST_SAMPLE_WAIT: Duration = Duration::from_secs(3);

/// Delay before the next refresh attempt: the TTL while the source is healthy,
/// doubling with each consecutive failure up to `max(ttl, BACKOFF_CAP)`.
fn retry_interval(ttl: Duration, failures: u32) -> Duration {
    let cap = ttl.max(BACKOFF_CAP);
    ttl.checked_mul(1u32 << failures.min(16))
        .unwrap_or(cap)
        .min(cap)
}

/// Shared state of `Cached` and `Observed`.
///
/// The mutex guards only bookkeeping: it is never held across `refresh`, so a
/// hung subprocess costs the one caller that runs it. One refresh runs at a
/// time (`refreshing`); everyone else is served the previous value, or waits
/// on `done` when there is none yet.
struct Core<T> {
    slot: Mutex<Slot<T>>,
    done: Condvar,
}

struct Slot<T> {
    /// When the last refresh finished, successfully or not.
    attempted: Option<Instant>,
    sampled: Option<Instant>,
    sampled_unix_ms: Option<u128>,
    value: Option<T>,
    error: Option<String>,
    failures: u32,
    refreshing: bool,
    /// Bumped by `invalidate`, so a refresh that straddles a mutation does not
    /// count as the re-read the invalidation asked for.
    generation: u64,
}

impl<T> Default for Core<T> {
    fn default() -> Self {
        Self {
            slot: Mutex::new(Slot {
                attempted: None,
                sampled: None,
                sampled_unix_ms: None,
                value: None,
                error: None,
                failures: 0,
                refreshing: false,
                generation: 0,
            }),
            done: Condvar::new(),
        }
    }
}

impl<T> Slot<T> {
    fn due(&self, ttl: Duration) -> bool {
        self.attempted
            .is_none_or(|at| at.elapsed() >= retry_interval(ttl, self.failures))
    }
}

/// Clears `refreshing` and wakes waiters even if `refresh` panics, so a
/// panicking source cannot wedge every later reader.
struct InFlight<'a, T> {
    core: &'a Core<T>,
    armed: bool,
}

impl<T> Drop for InFlight<'_, T> {
    fn drop(&mut self) {
        if self.armed {
            self.core.slot.safe_lock().refreshing = false;
            self.core.done.notify_all();
        }
    }
}

impl<T> Core<T> {
    /// Refresh if due (single-flight), then return the slot locked for the
    /// caller to read. `wait` bounds how long a caller with no value at all
    /// waits for someone else's refresh; `None` waits for it to finish (the
    /// refresh is itself bounded by its subprocess deadlines).
    fn read(
        &self,
        ttl: Duration,
        wait: Option<Duration>,
        refresh: impl FnOnce() -> Result<T, String>,
    ) -> MutexGuard<'_, Slot<T>> {
        let mut refresh = Some(refresh);
        let started = Instant::now();
        let mut slot = self.slot.safe_lock();
        loop {
            if !slot.due(ttl) {
                return slot;
            }
            if slot.refreshing {
                // Stale-while-refreshing: the last good value (reported stale
                // via its age) beats queueing behind a possibly hung source. A
                // source that has already failed has nothing worth waiting for.
                if slot.value.is_some() || slot.error.is_some() {
                    return slot;
                }
                let remaining = match wait {
                    Some(wait) => match wait.checked_sub(started.elapsed()) {
                        Some(left) if !left.is_zero() => left,
                        _ => return slot,
                    },
                    None => Duration::from_secs(1),
                };
                slot = self
                    .done
                    .wait_timeout(slot, remaining)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
                continue;
            }
            // Due and nobody refreshing. A caller refreshes at most once; if an
            // invalidation raced its refresh it serves what it has.
            let Some(run) = refresh.take() else {
                return slot;
            };
            slot.refreshing = true;
            let generation = slot.generation;
            drop(slot);
            let mut flight = InFlight {
                core: self,
                armed: true,
            };
            let result = run();
            slot = self.slot.safe_lock();
            flight.armed = false;
            let now = Instant::now();
            match result {
                Ok(value) => {
                    slot.value = Some(value);
                    slot.sampled = Some(now);
                    slot.sampled_unix_ms = Some(
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis(),
                    );
                    slot.error = None;
                    slot.failures = 0;
                }
                Err(error) => {
                    slot.error = Some(error);
                    slot.failures = slot.failures.saturating_add(1);
                }
            }
            // An invalidation during the refresh leaves the source due, so
            // the next reader re-reads after the mutation.
            slot.attempted = (slot.generation == generation).then_some(now);
            slot.refreshing = false;
            self.done.notify_all();
            if slot.attempted.is_none() {
                return slot;
            }
        }
    }

    fn invalidate(&self, clear: bool) {
        let mut slot = self.slot.safe_lock();
        slot.attempted = None;
        slot.generation = slot.generation.wrapping_add(1);
        if clear {
            slot.value = None;
            slot.sampled = None;
            slot.sampled_unix_ms = None;
        }
    }
}

/// A single value refreshed on demand, at most once per TTL.
///
/// Every `ubus`/`uci` read in this agent is a fork+exec, which is the dominant
/// cost of serving `/api/dashboard`. Wrapping each source in its own TTL means
/// the poll rate is decoupled from the refresh rate: a 3 s dashboard poll no
/// longer re-reads WAN state that changes hourly, and several clients polling
/// at once collapse into one refresh instead of multiplying the load.
pub struct Cached<T> {
    core: Core<T>,
}

impl<T: Clone> Cached<T> {
    /// Return the cached value if it is younger than `ttl`, otherwise call
    /// `refresh` and store the result.
    ///
    /// Concurrent callers share one refresh: while it runs they get the
    /// previous value, or wait for it when there is none yet.
    pub fn get_or_refresh(&self, ttl: Duration, refresh: impl FnOnce() -> T) -> T {
        let mut refresh = Some(refresh);
        let value = self
            .core
            .read(ttl, None, || match refresh.take() {
                Some(run) => Ok(run()),
                None => Err(String::new()),
            })
            .value
            .clone();
        match (value, refresh.take()) {
            (Some(value), _) => value,
            // Only reachable if an invalidation cleared the slot between a
            // waiter waking and reading it: compute this caller's own copy.
            (None, Some(run)) => run(),
            // Our refresh ran and stored Ok, and read() returns with the slot
            // still locked, so the value cannot be missing here.
            (None, None) => unreachable!("Cached refresh stored no value"),
        }
    }

    /// Force the next `get_or_refresh` to re-read. Call after a mutation that
    /// makes the cached copy wrong by definition.
    pub fn invalidate(&self) {
        self.core.invalidate(true);
    }
}

impl<T> Default for Cached<T> {
    fn default() -> Self {
        Self {
            core: Core::default(),
        }
    }
}

/// A source keeps its last successful sample when a refresh fails. Attempts
/// obey the TTL, and back off (doubling up to `BACKOFF_CAP`) while they keep
/// failing, so a dead source cannot trigger a subprocess storm.
pub struct Observed<T> {
    core: Core<T>,
}
#[derive(Clone, serde::Serialize)]
pub struct Freshness {
    pub sampled_at_ms: Option<u128>,
    pub age_ms: Option<u128>,
    pub ttl_ms: u128,
    pub stale: bool,
    pub error: Option<String>,
}
pub struct Sample<T> {
    pub value: Option<T>,
    pub freshness: Freshness,
}
impl<T> Default for Observed<T> {
    fn default() -> Self {
        Self {
            core: Core::default(),
        }
    }
}
impl<T: Clone> Observed<T> {
    pub fn read(&self, ttl: Duration, refresh: impl FnOnce() -> Result<T, String>) -> Sample<T> {
        self.read_within(ttl, FIRST_SAMPLE_WAIT, refresh)
    }

    fn read_within(
        &self,
        ttl: Duration,
        wait: Duration,
        refresh: impl FnOnce() -> Result<T, String>,
    ) -> Sample<T> {
        let slot = self.core.read(ttl, Some(wait), refresh);
        let age = slot.sampled.map(|at| at.elapsed());
        let error = match (&slot.error, &slot.value) {
            (Some(error), _) => Some(error.clone()),
            // Gave up waiting for someone else's first refresh.
            (None, None) if slot.refreshing => Some("first sample still in progress".into()),
            _ => None,
        };
        Sample {
            value: slot.value.clone(),
            freshness: Freshness {
                sampled_at_ms: slot.sampled_unix_ms,
                age_ms: age.map(|d| d.as_millis()),
                ttl_ms: ttl.as_millis(),
                stale: error.is_some() || age.is_none_or(|age| age > ttl),
                error,
            },
        }
    }
    pub fn invalidate(&self) {
        self.core.invalidate(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn serves_from_cache_within_ttl() {
        let calls = AtomicU32::new(0);
        let cached: Cached<u32> = Cached::default();
        let refresh = || {
            calls.fetch_add(1, Ordering::Relaxed);
            7
        };

        assert_eq!(cached.get_or_refresh(Duration::from_secs(60), refresh), 7);
        assert_eq!(cached.get_or_refresh(Duration::from_secs(60), refresh), 7);
        assert_eq!(
            calls.load(Ordering::Relaxed),
            1,
            "second call should hit cache"
        );
    }

    #[test]
    fn refreshes_once_ttl_elapses() {
        let calls = AtomicU32::new(0);
        let cached: Cached<u32> = Cached::default();
        let refresh = || calls.fetch_add(1, Ordering::Relaxed);

        cached.get_or_refresh(Duration::from_secs(60), refresh);
        // A zero TTL always counts as stale.
        cached.get_or_refresh(Duration::ZERO, refresh);
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn cached_serves_previous_value_while_another_caller_refreshes() {
        let cached: Cached<u32> = Cached::default();
        cached.get_or_refresh(Duration::ZERO, || 1);
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        std::thread::scope(|scope| {
            let cached = &cached;
            let slow = scope.spawn(move || {
                cached.get_or_refresh(Duration::ZERO, || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    2
                })
            });
            entered_rx.recv().unwrap();
            let started = Instant::now();
            let value = cached.get_or_refresh(Duration::ZERO, || panic!("single-flight"));
            assert_eq!(value, 1);
            assert!(started.elapsed() < Duration::from_secs(1));
            release_tx.send(()).unwrap();
            assert_eq!(slow.join().unwrap(), 2);
        });
        assert_eq!(cached.get_or_refresh(Duration::from_secs(60), || 3), 2);
    }

    #[test]
    fn invalidate_during_refresh_forces_a_fresh_read() {
        let cached: Cached<u32> = Cached::default();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        std::thread::scope(|scope| {
            let cached = &cached;
            let slow = scope.spawn(move || {
                cached.get_or_refresh(Duration::from_secs(60), || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    1 // read before the mutation
                })
            });
            entered_rx.recv().unwrap();
            cached.invalidate();
            release_tx.send(()).unwrap();
            assert_eq!(slow.join().unwrap(), 1);
        });
        // The straddling read does not satisfy the invalidation.
        assert_eq!(cached.get_or_refresh(Duration::from_secs(60), || 2), 2);
    }
}

#[cfg(test)]
mod observation_tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc;

    #[test]
    fn failed_refresh_retains_sample_and_throttles_retries() {
        let source = Observed::default();
        assert_eq!(source.read(Duration::ZERO, || Ok(42)).value, Some(42));
        let failed = source.read(Duration::ZERO, || Err("radio offline".into()));
        assert_eq!(failed.value, Some(42));
        assert!(failed.freshness.stale);
        assert_eq!(failed.freshness.error.as_deref(), Some("radio offline"));
        source.read(Duration::from_secs(60), || panic!("retry must respect TTL"));
        source.invalidate();
        let recovered = source.read(Duration::from_secs(60), || Ok(43));
        assert_eq!(recovered.value, Some(43));
        assert!(!recovered.freshness.stale);
    }
    #[test]
    fn first_failure_is_missing_rather_than_a_fabricated_reading() {
        let source: Observed<u32> = Observed::default();
        let sample = source.read(Duration::ZERO, || Err("unavailable".into()));
        assert!(sample.value.is_none());
        assert!(sample.freshness.sampled_at_ms.is_none());
        assert!(sample.freshness.stale);
    }
    #[test]
    fn concurrent_consumers_share_one_refresh() {
        let source = std::sync::Arc::new(Observed::default());
        let calls = std::sync::Arc::new(AtomicU32::new(0));
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    let sample = source.read(Duration::from_secs(60), || {
                        calls.fetch_add(1, Ordering::Relaxed);
                        std::thread::sleep(Duration::from_millis(50));
                        Ok(42)
                    });
                    // Nobody had a value, so everyone waited for the one refresh.
                    assert_eq!(sample.value, Some(42));
                });
            }
        });
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn stale_value_is_served_immediately_while_a_refresh_hangs() {
        let source = Observed::default();
        source.read(Duration::ZERO, || Ok(1));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        std::thread::scope(|scope| {
            let source = &source;
            let hung = scope.spawn(move || {
                source.read(Duration::ZERO, || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(2)
                })
            });
            entered_rx.recv().unwrap();
            let started = Instant::now();
            let sample = source.read(Duration::ZERO, || panic!("must not start a second refresh"));
            assert!(started.elapsed() < Duration::from_millis(500));
            assert_eq!(sample.value, Some(1));
            assert!(sample.freshness.stale);
            assert!(sample.freshness.sampled_at_ms.is_some());
            release_tx.send(()).unwrap();
            assert_eq!(hung.join().unwrap().value, Some(2));
        });
    }

    #[test]
    fn reader_without_a_value_waits_only_a_bounded_time() {
        let source: Observed<u32> = Observed::default();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        std::thread::scope(|scope| {
            let source = &source;
            let hung = scope.spawn(move || {
                source.read(Duration::from_secs(60), || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(7)
                })
            });
            entered_rx.recv().unwrap();
            let started = Instant::now();
            let sample =
                source.read_within(Duration::from_secs(60), Duration::from_millis(100), || {
                    panic!("must not start a second refresh")
                });
            let waited = started.elapsed();
            assert!(waited >= Duration::from_millis(100), "{waited:?}");
            assert!(waited < Duration::from_secs(2), "{waited:?}");
            assert!(sample.value.is_none());
            assert!(sample.freshness.stale);
            assert!(sample.freshness.error.is_some());
            release_tx.send(()).unwrap();
            hung.join().unwrap();
        });
        assert_eq!(
            source.read(Duration::from_secs(60), || Ok(8)).value,
            Some(7)
        );
    }

    #[test]
    fn retry_interval_doubles_from_ttl_to_cap() {
        let ttl = Duration::from_secs(1);
        let steps: Vec<u64> = (0..8).map(|f| retry_interval(ttl, f).as_secs()).collect();
        assert_eq!(steps, [1, 2, 4, 8, 16, 30, 30, 30]);
        // A TTL above the cap is never shortened.
        assert_eq!(retry_interval(Duration::from_secs(300), 3).as_secs(), 300);
        assert_eq!(retry_interval(Duration::ZERO, 5), Duration::ZERO);
        assert_eq!(retry_interval(ttl, u32::MAX).as_secs(), 30);
    }

    #[test]
    fn consecutive_failures_back_off_and_success_resets() {
        let ttl = Duration::from_millis(100);
        let source: Observed<u32> = Observed::default();
        let calls = AtomicU32::new(0);
        let fail = || {
            calls.fetch_add(1, Ordering::Relaxed);
            Err("hung".to_string())
        };
        source.read(ttl, fail); // failures = 1 → next attempt after 200 ms
        std::thread::sleep(Duration::from_millis(120));
        source.read(ttl, fail);
        assert_eq!(calls.load(Ordering::Relaxed), 1, "backoff ignored");
        std::thread::sleep(Duration::from_millis(120));
        source.read(ttl, fail); // 240 ms > 200 ms: retried, failures = 2
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        source.invalidate(); // an invalidation forces one attempt regardless
        assert_eq!(source.read(ttl, || Ok(5)).value, Some(5));
        std::thread::sleep(Duration::from_millis(120));
        // Success reset the backoff: a plain TTL later it refreshes again.
        assert_eq!(source.read(ttl, || Ok(6)).value, Some(6));
    }

    #[test]
    fn panicking_refresh_does_not_wedge_the_source() {
        let source: Observed<u32> = Observed::default();
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            source.read(Duration::from_secs(60), || panic!("refresh blew up"))
        }));
        assert!(caught.is_err());
        let started = Instant::now();
        assert_eq!(
            source.read(Duration::from_secs(60), || Ok(1)).value,
            Some(1)
        );
        assert!(started.elapsed() < Duration::from_millis(500));
    }
}
