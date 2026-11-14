//! The agent's own diagnostics log.
//!
//! The stock busybox `syslogd` runs with `-l 1` (emergency only) and has no
//! `logread` backend, so output piped to `logger` is discarded. Instead,
//! stdout/stderr are redirected into a pipe that a thread drains into a
//! size-capped file on `/data`, which survives reboots.
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::csv_utils::{device_datetime, now_secs};

pub const LOG_PATH: &str = "/data/local/tmp/zte-agent.log";
/// Per file; one rotated copy (`.1`) is kept, so at most twice this on flash.
const MAX_BYTES: u64 = 256 * 1024;

/// Redirect stdout/stderr into the log. On any failure output stays wherever
/// the startup script sent it.
pub fn init() {
    let Ok(log) = Log::open(PathBuf::from(LOG_PATH), MAX_BYTES) else {
        return;
    };
    let mut fds = [0; 2];
    // CLOEXEC on the pipe itself; dup2 onto 1/2 below yields plain fds, and
    // every subprocess the agent spawns sets its own stdout/stderr anyway.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return;
    }
    let (read_fd, write_fd) = (fds[0], fds[1]);
    for fd in fds {
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    let redirected = unsafe { libc::dup2(write_fd, 1) >= 0 && libc::dup2(write_fd, 2) >= 0 };
    unsafe { libc::close(write_fd) };
    let reader = unsafe { File::from_raw_fd(read_fd) };
    if !redirected {
        return;
    }
    // The drain must never stop: a full pipe would block every eprintln!, and
    // a closed one would make every eprintln! panic on EPIPE.
    let spawned = std::thread::Builder::new()
        .name(DRAIN_THREAD.into())
        .spawn(move || {
            let mut reader = BufReader::new(reader);
            let mut log = log;
            loop {
                let ended = panic::catch_unwind(AssertUnwindSafe(|| drain(&mut reader, &mut log)));
                if ended.is_ok() {
                    break; // EOF: nothing can write to the pipe any more.
                }
                // A panic while logging one line; keep draining.
            }
            silence_stdio();
        });
    if spawned.is_err() {
        silence_stdio();
    }
}

/// Name of the drain thread. The panic hook must not write to stderr from it:
/// with the pipe full, the drain would block on itself.
const DRAIN_THREAD: &str = "diag-log";

/// Point stdout/stderr at /dev/null, so output is dropped rather than failing
/// with EPIPE (which makes `eprintln!` panic) once the drain is gone.
fn silence_stdio() {
    if let Ok(null) = OpenOptions::new().write(true).open("/dev/null") {
        unsafe {
            libc::dup2(null.as_raw_fd(), 1);
            libc::dup2(null.as_raw_fd(), 2);
        }
    }
}

/// Copy lines into the log until EOF. Read errors are retried, not fatal.
fn drain(reader: &mut impl BufRead, log: &mut Log) {
    let mut line = Vec::new();
    loop {
        line.clear();
        // read_until, not lines(): invalid UTF-8 must not end the drain.
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => return,
            Ok(_) => log.line(String::from_utf8_lossy(&line).trim_end()),
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// Log every panic (message, location, thread) through stderr into the log.
/// Unwinding is unchanged: the panicking thread decides whether it recovers.
pub fn install_panic_hook() {
    panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed");
        if name == DRAIN_THREAD {
            return;
        }
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".into());
        let line = format!(
            "[panic] thread '{name}' panicked at {location}: {}\n",
            panic_message(info.payload())
        );
        // Not eprintln!: a failing write must not panic inside the hook.
        let _ = std::io::stderr().write_all(line.as_bytes());
    }));
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}

/// Run `f`, containing a panic. Returns `None` if it panicked; the hook has
/// already logged the message and location. State behind `safe_lock()` is
/// still usable afterwards (poisoning is ignored), so callers only need to
/// leave their own invariants sane.
pub fn contain<R>(what: &str, f: impl FnOnce() -> R) -> Option<R> {
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => Some(value),
        Err(_) => {
            let _ = writeln!(std::io::stderr(), "[{what}] recovered from panic");
            None
        }
    }
}

/// Keep a background loop alive: run `body` and, whenever it panics or
/// returns, run it again after `restart_delay`. Never returns.
pub fn supervise(what: &str, restart_delay: Duration, mut body: impl FnMut()) -> ! {
    loop {
        if contain(what, &mut body).is_some() {
            let _ = writeln!(std::io::stderr(), "[{what}] loop exited, restarting");
        }
        std::thread::sleep(restart_delay);
    }
}

struct Log {
    path: PathBuf,
    file: File,
    written: u64,
    max: u64,
    previous: Option<String>,
    repeats: u64,
}

impl Log {
    fn open(path: PathBuf, max: u64) -> std::io::Result<Self> {
        let file = open_append(&path)?;
        let written = file.metadata()?.len();
        Ok(Self {
            path,
            file,
            written,
            max,
            previous: None,
            repeats: 0,
        })
    }

    /// Consecutive identical lines are counted, not written, so an error
    /// repeated by a periodic task cannot fill the file.
    fn line(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.previous.as_deref() == Some(text) {
            self.repeats += 1;
            return;
        }
        if self.repeats > 0 {
            let note = format!("[log] previous message repeated {} times", self.repeats);
            self.write(&note);
            self.repeats = 0;
        }
        self.write(text);
        self.previous = Some(text.to_string());
    }

    fn write(&mut self, text: &str) {
        let entry = format!(
            "{} up={}s {text}\n",
            device_datetime(now_secs()),
            uptime_secs()
        );
        if self.written + entry.len() as u64 > self.max {
            self.rotate();
        }
        // Errors (e.g. /data full) drop the line; the drain keeps running.
        if self.file.write_all(entry.as_bytes()).is_ok() {
            self.written += entry.len() as u64;
        }
    }

    fn rotate(&mut self) {
        let mut rotated = self.path.clone().into_os_string();
        rotated.push(".1");
        let _ = fs::rename(&self.path, rotated);
        if let Ok(file) = open_append(&self.path) {
            self.file = file;
            self.written = 0;
        }
    }
}

fn open_append(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
}

/// Seconds since boot. The wall clock can be wrong until NITZ sync, so each
/// line carries both.
fn uptime_secs() -> u64 {
    fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split('.').next()?.parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_log(max: u64) -> (PathBuf, Log) {
        let dir =
            std::env::temp_dir().join(format!("zte-agent-log-test-{}-{}", std::process::id(), max));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agent.log");
        let log = Log::open(path.clone(), max).unwrap();
        (path, log)
    }

    #[test]
    fn repeated_lines_are_collapsed_into_a_count() {
        let (path, mut log) = temp_log(64 * 1024);
        for _ in 0..5 {
            log.line("[charge_policy] charger control state is unavailable");
        }
        log.line("[server] listener down");
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("charger control state").count(), 1);
        assert!(text.contains("previous message repeated 4 times"));
        assert!(text.ends_with("[server] listener down\n"));
    }

    #[test]
    fn drain_survives_read_errors_and_invalid_utf8() {
        struct Flaky {
            chunks: Vec<std::io::Result<Vec<u8>>>,
        }
        impl std::io::Read for Flaky {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.chunks.is_empty() {
                    return Ok(0);
                }
                let chunk = self.chunks.remove(0)?;
                buf[..chunk.len()].copy_from_slice(&chunk);
                Ok(chunk.len())
            }
        }
        let (path, mut log) = temp_log(64 * 1024 + 1);
        let mut reader = BufReader::new(Flaky {
            chunks: vec![
                Ok(b"before\n".to_vec()),
                Err(std::io::Error::other("transient")),
                Err(ErrorKind::Interrupted.into()),
                Ok(b"bad \xff utf8\nafter\n".to_vec()),
            ],
        });
        drain(&mut reader, &mut log);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("before"));
        assert!(text.contains("bad \u{fffd} utf8"));
        assert!(text.ends_with("after\n"), "{text}");
    }

    #[test]
    fn contain_turns_a_panic_into_none() {
        assert_eq!(contain("test", || 7), Some(7));
        assert_eq!(contain("test", || -> u8 { panic!("boom") }), None);
        let payload: Box<dyn std::any::Any + Send> = Box::new(String::from("owned"));
        assert_eq!(panic_message(payload.as_ref()), "owned");
        let payload: Box<dyn std::any::Any + Send> = Box::new("static");
        assert_eq!(panic_message(payload.as_ref()), "static");
        let payload: Box<dyn std::any::Any + Send> = Box::new(5u8);
        assert_eq!(panic_message(payload.as_ref()), "non-string panic payload");
    }

    #[test]
    fn rotation_bounds_the_file_and_keeps_one_previous_copy() {
        let (path, mut log) = temp_log(256);
        for i in 0..40 {
            log.line(&format!("line {i}"));
        }
        let current = fs::metadata(&path).unwrap().len();
        let mut rotated = path.clone().into_os_string();
        rotated.push(".1");
        assert!(current <= 256, "{current}");
        assert!(fs::metadata(&rotated).unwrap().len() <= 256);
        assert!(fs::read_to_string(&path).unwrap().contains("line 39"));
    }
}
