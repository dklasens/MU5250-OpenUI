use std::io::BufRead;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;

use serde_json::Value;

use crate::util::MutexExt;

const RESTART_DELAY_SECS: u64 = 5;
const IGNORED_EVENTS: &[&str] = &["zwrt_deviceui_event.touchstatus"];
/// Bounded so a subscriber that stops draining cannot grow its queue without
/// limit on a device that runs for months. Excess events are dropped.
const CHANNEL_CAPACITY: usize = 64;

struct Subscriber {
    event: String,
    tx: mpsc::SyncSender<Value>,
    dropped: AtomicU64,
}

pub struct EventBus {
    subscribers: Mutex<Vec<Subscriber>>,
}

impl EventBus {
    pub fn new() -> Self {
        EventBus {
            subscribers: Mutex::new(Vec::new()),
        }
    }

    /// Register interest in a specific ubus event. Returns a receiver
    /// that will get the event payload each time it fires.
    pub fn subscribe(&self, event_name: &str) -> mpsc::Receiver<Value> {
        let (tx, rx) = mpsc::sync_channel(CHANNEL_CAPACITY);
        let mut subs = self.subscribers.safe_lock();
        subs.push(Subscriber {
            event: event_name.to_string(),
            tx,
            dropped: AtomicU64::new(0),
        });
        rx
    }

    /// Spawn the listener thread. Consumes self into an Arc-compatible form.
    /// Call this after all subscriptions are registered.
    ///
    /// The loop is supervised: a panic (say, in dispatch) restarts it after
    /// the usual delay instead of silently ending event delivery.
    pub fn start(self) {
        let spawned = std::thread::Builder::new()
            .name("event-bus".into())
            .spawn(move || {
                crate::diag_log::supervise(
                    "event_bus",
                    std::time::Duration::from_secs(RESTART_DELAY_SECS),
                    || self.run_loop(),
                )
            });
        if let Err(e) = spawned {
            eprintln!("[event_bus] failed to start listener thread: {e}");
        }
    }

    fn run_loop(&self) {
        loop {
            match spawn_ubus_listen() {
                Ok(child) => {
                    // Killed and reaped on drop, including when unwinding.
                    let mut child = Listener(child);
                    eprintln!("[event_bus] ubus listen started (pid {})", child.0.id());
                    if let Some(stdout) = child.0.stdout.take() {
                        self.read_events(stdout);
                    }
                    drop(child);
                    eprintln!(
                        "[event_bus] ubus listen exited, restarting in {RESTART_DELAY_SECS}s"
                    );
                }
                Err(e) => {
                    eprintln!("[event_bus] failed to spawn ubus listen: {e}");
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(RESTART_DELAY_SECS));
        }
    }

    fn read_events(&self, stdout: std::process::ChildStdout) {
        let reader = std::io::BufReader::new(stdout);
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break, // pipe closed
            };

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // ubus listen outputs: { "event_name": { ...payload... } }
            let parsed: Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let obj = match parsed.as_object() {
                Some(o) => o,
                None => continue,
            };

            // Extract single top-level key as event name
            for (event_name, payload) in obj {
                if IGNORED_EVENTS.contains(&event_name.as_str()) {
                    continue;
                }

                self.dispatch(event_name, payload);
            }
        }
    }

    fn dispatch(&self, event_name: &str, payload: &Value) {
        let subs = self.subscribers.safe_lock();
        for sub in subs.iter() {
            if sub.event == event_name {
                // Non-blocking: drop the event if the receiver is backed up or
                // gone, so a stalled consumer can never block this listener
                // thread or grow its queue without bound.
                if sub.tx.try_send(payload.clone()).is_err()
                    && sub.dropped.fetch_add(1, Ordering::Relaxed) == 0
                {
                    eprintln!("[event_bus] {event_name}: subscriber not draining, dropping events");
                }
            }
        }
    }
}

/// A `ubus listen` child that is killed (if still running) and reaped when
/// dropped, so a restart never leaves a stray listener or a zombie behind.
struct Listener(Child);

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_ubus_listen() -> Result<Child, String> {
    Command::new("ubus")
        .arg("listen")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn ubus listen: {e}"))
}
