use crate::process::BoundedCommand;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::at_cmd;
use crate::cell;
use crate::connection_logger;
use crate::device_ext;
use crate::esim;
use crate::handlers::{self, AppState};
use crate::network_ext;
use crate::router;
use crate::signal_logger;
use crate::sim;
use crate::sms;
use crate::usb;
use crate::wifi;

/// How long a worker blocks before re-checking whether the listener died.
/// Also bounds how long a rebuild waits for the other workers to drain.
const WORKER_POLL: Duration = Duration::from_millis(500);
const RETRY_MIN: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);

/// Largest request body accepted.
const MAX_BODY: usize = 1024 * 1024;
const TOO_LARGE: &str = "request body exceeds 1 MiB";
/// Total time a client gets to deliver a request body, however it trickles.
const BODY_DEADLINE: Duration = Duration::from_secs(10);
/// Body threads at once (each reads one body, then serves that request).
const MAX_BODY_READS: usize = 8;
/// tiny_http reads a body of at most this many bytes (without `Expect`)
/// before it hands the request over, so reading it cannot block a worker.
const TINY_HTTP_PREBUFFERED: usize = 1024;

/// Limits on reading request bodies. See `take_body`.
#[derive(Clone)]
struct BodyReads {
    in_flight: Arc<AtomicUsize>,
    limit: usize,
    deadline: Duration,
    /// Port of the listener, to tell its accepted sockets apart.
    listen_port: Option<u16>,
    watchdog: Watchdog,
}

pub fn start(threads: usize, state: Arc<AppState>) {
    // Seed the CPU tracker with a baseline
    state.cpu.seed();

    // tiny_http's accept thread exits permanently on its first accept() error
    // — plausible here via EMFILE, ENOBUFS under memory pressure, or interface
    // churn. That used to leave every worker blocked forever on a listener that
    // would never yield another request, with the process still alive and
    // nothing to restart it. Supervise instead: a worker that sees the failure
    // flags it, all workers drain, and we rebuild the listener.
    let mut retry = RETRY_MIN;
    // Shared across listener rebuilds: an abandoned body read outlives its
    // listener and still counts against the cap.
    let in_flight = Arc::new(AtomicUsize::new(0));
    let watchdog = Watchdog::new();
    loop {
        let bind = state.binding.address();
        let generation = state.binding.generation.load(Ordering::Acquire);
        let server = match Server::http(&bind) {
            Ok(s) => {
                retry = RETRY_MIN;
                Arc::new(s)
            }
            Err(e) => {
                eprintln!(
                    "[server] bind {bind} failed: {e}; retrying in {}s",
                    retry.as_secs()
                );
                std::thread::sleep(retry);
                retry = (retry * 2).min(RETRY_MAX);
                continue;
            }
        };

        let dead = Arc::new(AtomicBool::new(false));
        let mut handles = Vec::new();
        let reads = BodyReads {
            in_flight: Arc::clone(&in_flight),
            limit: MAX_BODY_READS,
            deadline: BODY_DEADLINE,
            listen_port: server.server_addr().to_ip().map(|a| a.port()),
            watchdog: watchdog.clone(),
        };

        for _ in 0..threads {
            let server = Arc::clone(&server);
            let state = Arc::clone(&state);
            let dead = Arc::clone(&dead);
            let reads = reads.clone();
            let worker = std::thread::Builder::new().name("http-worker".into());
            let spawned = worker.spawn(move || loop {
                if state.binding.generation.load(Ordering::Acquire) != generation {
                    dead.store(true, Ordering::Relaxed);
                }
                if dead.load(Ordering::Relaxed) {
                    return;
                }
                match server.recv_timeout(WORKER_POLL) {
                    // A panic outside `route` (which has its own guard) drops
                    // the request, and tiny_http answers it with a bare 500.
                    // Either way the worker keeps serving.
                    Ok(Some(request)) => {
                        let _ = crate::diag_log::contain("server", || {
                            handle_request(request, &state, &reads)
                        });
                    }
                    // Idle timeout — loop round and re-check `dead`.
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("[server] listener failed: {e}");
                        dead.store(true, Ordering::Relaxed);
                        return;
                    }
                }
            });
            match spawned {
                Ok(handle) => handles.push(handle),
                // With no workers at all the join below returns at once and
                // the listener is rebuilt after the retry delay.
                Err(e) => eprintln!("[server] cannot start worker: {e}"),
            }
        }

        for h in handles {
            let _ = h.join();
        }

        eprintln!("[server] listener down, rebuilding in {}s", retry.as_secs());
        std::thread::sleep(retry);
        retry = (retry * 2).min(RETRY_MAX);
    }
}

const DESTRUCTIVE_PATHS: &[&str] = &[
    "/api/device/reboot",
    "/api/device/shutdown",
    "/api/system/kill-bloat",
    "/api/esim/enable",
    "/api/esim/disable",
    "/api/esim/delete",
];

/// Actions a PIN session may not take: switching the cellular profile, and the
/// lock that guards a profile against activation.
const PASSWORD_PATHS: &[&str] = &[
    "/api/esim/enable",
    "/api/esim/disable",
    "/api/esim/lock",
    "/api/esim/download",
    "/api/esim/download/confirm",
    "/api/esim/download/cancel",
    "/api/esim/delete",
    "/api/esim/receipts/send",
];

fn cors_headers(origin: Option<&str>) -> Vec<Header> {
    let allowed = origin
        .and_then(|o| is_lan_origin(o).then_some(o))
        .unwrap_or("");
    vec![
        Header::from_bytes("Access-Control-Allow-Origin", allowed).unwrap(),
        Header::from_bytes(
            "Access-Control-Allow-Methods",
            "GET, POST, PUT, DELETE, OPTIONS",
        )
        .unwrap(),
        Header::from_bytes(
            "Access-Control-Allow-Headers",
            "Authorization, Content-Type, X-Confirm",
        )
        .unwrap(),
        Header::from_bytes("Access-Control-Max-Age", "86400").unwrap(),
    ]
}

fn is_lan_origin(origin: &str) -> bool {
    if !origin.starts_with("http://") {
        return false;
    }
    let host = &origin[7..];
    let host = host.split(':').next().unwrap_or(host);
    if host == "localhost" || host == "127.0.0.1" || host == "::1" {
        return true;
    }
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    let octets: Vec<u8> = parts.iter().filter_map(|p| p.parse().ok()).collect();
    if octets.len() != 4 {
        return false;
    }
    octets[0] == 10
        || (octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31)
        || (octets[0] == 192 && octets[1] == 168)
}

fn handle_request(request: Request, state: &Arc<AppState>, reads: &BodyReads) {
    // The body comes first, so that every later response (including 401s)
    // goes out with it consumed: tiny_http drains an unread body when it drops
    // a request. A body still on the socket is read, and the request served,
    // on a body thread; the worker goes straight back to `recv`.
    let state = Arc::clone(state);
    take_body(request, reads, move |request, body| {
        serve(request, &body, &state)
    });
}

/// Everything after the body: CORS, auth, downloads, routing.
fn serve(request: Request, body: &[u8], state: &AppState) {
    let method = request.method().clone();
    let url = request.url().to_string();
    let path = url.split('?').next().unwrap_or(&url).to_string();
    let origin = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Origin"))
        .map(|h| h.value.as_str().to_string());
    let origin_ref = origin.as_deref();
    let client_ip = request
        .remote_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_default();
    let user_agent = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("User-Agent"))
        .map(|h| h.value.as_str().to_string());
    let user_agent_ref = user_agent.as_deref();

    if method == Method::Options {
        let mut response = Response::empty(200);
        for h in cors_headers(origin_ref) {
            response = response.with_header(h);
        }
        let _ = request.respond(response);
        return;
    }

    // The LAN confirmation nonce authorises only the pending change. It lets
    // the browser prove connectivity without sending its session bearer token
    // to a new IP that might already be occupied by another host.
    let needs_auth =
        path != "/api/auth/login" && !(method == Method::Post && path == "/api/router/lan/confirm");
    if needs_auth {
        let session = request
            .headers()
            .iter()
            .find(|h| h.field.as_str().to_ascii_lowercase() == "authorization")
            .and_then(|h| h.value.as_str().strip_prefix("Bearer "))
            .and_then(|token| state.auth.validate(token));
        let authorized = session.is_some();

        if !state.auth.has_password() {
            respond(
                request,
                403,
                json!({"ok": false, "error": "no password configured. Set ZTE_AGENT_PASSWORD environment variable."}),
                origin_ref,
            );
            return;
        } else if !authorized {
            respond(
                request,
                401,
                json!({"ok": false, "error": "unauthorized"}),
                origin_ref,
            );
            return;
        } else if PASSWORD_PATHS.contains(&path.as_str())
            && session != Some(crate::auth::CredentialKind::Password)
        {
            respond(
                request,
                403,
                json!({"ok": false, "error": "this action needs a session signed in with the password, not the PIN"}),
                origin_ref,
            );
            return;
        }
    }

    if DESTRUCTIVE_PATHS.contains(&path.as_str()) {
        let confirmed = request
            .headers()
            .iter()
            .any(|h| h.field.equiv("X-Confirm") && h.value.as_str() == "true");
        if !confirmed {
            respond(
                request,
                400,
                json!({"ok": false, "error": "destructive action requires X-Confirm: true header"}),
                origin_ref,
            );
            return;
        }
    }

    if method == Method::Get {
        let download = match (&method, path.as_str()) {
            (&Method::Get, "/api/logger/signal/download") => {
                Some((signal_logger::LOG_PATH, "signal_log.csv"))
            }
            (&Method::Get, "/api/logger/connection/download") => {
                Some((connection_logger::LOG_PATH, "connection_log.csv"))
            }
            _ => None,
        };
        if let Some((path, name)) = download {
            match crate::logging::open_download(path) {
                Ok((file, len)) => {
                    use std::io::Read;
                    let mut response = Response::new(
                        tiny_http::StatusCode(200),
                        vec![],
                        file.take(len),
                        Some(len as usize),
                        None,
                    )
                    .with_header(
                        Header::from_bytes("Content-Type", "text/csv; charset=utf-8").unwrap(),
                    )
                    .with_header(
                        Header::from_bytes(
                            "Content-Disposition",
                            format!("attachment; filename=\"{name}\""),
                        )
                        .unwrap(),
                    )
                    .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
                    for header in cors_headers(origin_ref) {
                        response = response.with_header(header);
                    }
                    let _ = request.respond(response);
                }
                Err(_) => respond(
                    request,
                    404,
                    json!({"ok": false, "error": "no readable log file"}),
                    origin_ref,
                ),
            }
            return;
        }
    }

    let (status, body_json) =
        contain_route(|| route(&method, &path, state, body, &client_ip, user_agent_ref));
    respond(request, status, body_json, origin_ref);
}

/// Run a handler, turning a panic into a 500 envelope. Shared state stays
/// usable: every lock goes through `safe_lock()`, which ignores poisoning.
fn contain_route(route: impl FnOnce() -> (u16, Value)) -> (u16, Value) {
    crate::diag_log::contain("server", route)
        .unwrap_or_else(|| (500, json!({"ok": false, "error": "internal error"})))
}

#[derive(Debug, PartialEq)]
enum BodyKind {
    /// Nothing to read (or nothing we will read).
    Empty,
    /// Already in memory: tiny_http read it before handing the request over.
    Buffered,
    /// Still on the socket, at the client's pace.
    Streamed,
    /// Declared over `MAX_BODY`.
    TooLarge,
}

/// Mirrors how tiny_http 0.12 (`request::new_request`) picks the body reader:
/// it uses the first matching header, and ignores `Content-Length` when
/// `Transfer-Encoding` is present (`body_length` is then `None`).
fn body_kind(headers: &[Header], body_length: Option<usize>) -> BodyKind {
    let first = |name: &'static str| {
        headers
            .iter()
            .find(|h| h.field.equiv(name))
            .map(|h| h.value.as_str())
    };
    // An upgrade request's "body" is the raw socket: reading it would block
    // until the client closes. Nothing here accepts an upgrade.
    if first("Connection").is_some_and(|v| v.to_ascii_lowercase().contains("upgrade")) {
        return BodyKind::Empty;
    }
    match body_length {
        Some(0) => BodyKind::Empty,
        Some(n) if n > MAX_BODY => BodyKind::TooLarge,
        Some(n) if n <= TINY_HTTP_PREBUFFERED && first("Expect").is_none() => BodyKind::Buffered,
        Some(_) => BodyKind::Streamed,
        None if first("Transfer-Encoding").is_some() => BodyKind::Streamed,
        None => BodyKind::Empty,
    }
}

fn read_capped(reader: &mut dyn std::io::Read) -> std::io::Result<Vec<u8>> {
    let mut body = Vec::new();
    std::io::Read::read_to_end(
        &mut std::io::Read::take(reader, MAX_BODY as u64 + 1),
        &mut body,
    )?;
    Ok(body)
}

/// Take the request body without letting the client hold a worker, then call
/// `serve`.
///
/// tiny_http sets no socket timeouts, so a client that stops sending mid-body
/// (a phone leaving Wi-Fi) would block a plain read forever. Bodies already in
/// memory are read in place and served on the calling worker. A body still on
/// the socket is handed, with `serve`, to a body thread, and the worker
/// returns at once. At most `limit` body threads run (503 beyond). Each is
/// bounded by `deadline` in total, enforced by the `Watchdog`: when it passes,
/// the socket's read side is shut down, the read ends, and the client gets a
/// 408, whether the client stalled or is trickling bytes.
fn take_body<F>(mut request: Request, reads: &BodyReads, serve: F)
where
    F: FnOnce(Request, Vec<u8>) + Send + 'static,
{
    match body_kind(request.headers(), request.body_length()) {
        BodyKind::Empty => serve(request, Vec::new()),
        BodyKind::Buffered => match read_capped(request.as_reader()) {
            Ok(body) => serve(request, body),
            Err(e) => reject(request, reads, 400, format!("failed to read body: {e}")),
        },
        BodyKind::TooLarge => reject(request, reads, 413, TOO_LARGE),
        BodyKind::Streamed => spawn_body_thread(request, reads, serve),
    }
}

fn spawn_body_thread<F>(request: Request, reads: &BodyReads, serve: F)
where
    F: FnOnce(Request, Vec<u8>) + Send + 'static,
{
    let Some(permit) = Permit::acquire(&reads.in_flight, reads.limit) else {
        reject(
            request,
            reads,
            503,
            "too many uploads in progress; retry shortly",
        );
        return;
    };
    // The request goes over a channel rather than into the closure, so that
    // it is still ours to answer if the thread cannot be spawned.
    let (job_tx, job_rx) = mpsc::channel::<Request>();
    let thread_reads = reads.clone();
    let spawned = std::thread::Builder::new()
        .name("http-body".into())
        .spawn(move || {
            // Held until this thread is done with the request.
            let _permit = permit;
            let Ok(request) = job_rx.recv() else {
                return;
            };
            // Contained like a worker: a panic is logged, and the dropped
            // request is answered with a bare 500 by tiny_http.
            let _ = crate::diag_log::contain("server", || {
                if let Some((request, body)) = read_streamed(request, &thread_reads) {
                    serve(request, body);
                }
            });
        });
    if spawned.is_err() {
        reject(request, reads, 503, "cannot read request body right now");
        return;
    }
    if let Err(mpsc::SendError(request)) = job_tx.send(request) {
        reject(request, reads, 503, "cannot read request body right now");
    }
}

/// Read a body still on the socket, under the total deadline. Returns `None`
/// when the request has already been answered.
fn read_streamed(mut request: Request, reads: &BodyReads) -> Option<(Request, Vec<u8>)> {
    let watch = request.remote_addr().copied().map(|peer| {
        reads
            .watchdog
            .watch(peer, reads.listen_port, reads.deadline)
    });
    let started = Instant::now();
    let body = read_capped(request.as_reader());
    let expired = started.elapsed() >= reads.deadline || watch.as_ref().is_some_and(Watch::fired);
    // Deregister while we still hold the request: the watchdog only touches a
    // socket whose request (and so whose descriptors) is still alive.
    drop(watch);
    match body {
        _ if expired => reject(request, reads, 408, BODY_TIMEOUT),
        Ok(body) if body.len() <= MAX_BODY => return Some((request, body)),
        Ok(_) => reject(request, reads, 413, TOO_LARGE),
        Err(e) => reject(request, reads, 400, format!("failed to read body: {e}")),
    }
    None
}

/// One thread that cuts off body reads past their deadline, by shutting down
/// the read side of their socket (see `shutdown_read`).
#[derive(Clone)]
struct Watchdog {
    shared: Arc<Watched>,
}

#[derive(Default)]
struct Watched {
    entries: Mutex<Vec<WatchEntry>>,
    changed: Condvar,
    next_id: AtomicU64,
}

struct WatchEntry {
    id: u64,
    due: Instant,
    peer: SocketAddr,
    listen_port: Option<u16>,
    fired: Arc<AtomicBool>,
}

/// A registered body read. Dropping it deregisters.
struct Watch {
    shared: Arc<Watched>,
    id: u64,
    fired: Arc<AtomicBool>,
}

impl Watch {
    fn fired(&self) -> bool {
        self.fired.load(Ordering::Acquire)
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        let mut entries = self
            .shared
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        entries.retain(|entry| entry.id != self.id);
    }
}

impl Watchdog {
    fn new() -> Self {
        let shared = Arc::new(Watched::default());
        let weak = Arc::downgrade(&shared);
        let spawned = std::thread::Builder::new()
            .name("http-body-watchdog".into())
            .spawn(move || {
                // Exits once every Watchdog handle is gone (tests); in the
                // agent it lives as long as the process.
                while let Some(true) =
                    crate::diag_log::contain("http-body-watchdog", || Self::tick(&weak))
                {
                }
            });
        if let Err(e) = spawned {
            eprintln!("[server] cannot start body watchdog: {e}; uploads are bounded only by the in-flight cap");
        }
        Self { shared }
    }

    fn watch(&self, peer: SocketAddr, listen_port: Option<u16>, deadline: Duration) -> Watch {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let fired = Arc::new(AtomicBool::new(false));
        let mut entries = self
            .shared
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        entries.push(WatchEntry {
            id,
            due: Instant::now() + deadline,
            peer,
            listen_port,
            fired: Arc::clone(&fired),
        });
        self.shared.changed.notify_all();
        Watch {
            shared: Arc::clone(&self.shared),
            id,
            fired,
        }
    }

    /// Cut off overdue reads, then sleep until the next deadline (at most 1 s,
    /// to notice shutdown). Returns false when the watchdog is no longer used.
    fn tick(weak: &Weak<Watched>) -> bool {
        let Some(shared) = weak.upgrade() else {
            return false;
        };
        let mut entries = shared.entries.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        // Under the lock: the owning body thread cannot deregister, and so
        // cannot have dropped its request, while its socket is shut down.
        entries.retain(|entry| {
            if entry.due > now {
                return true;
            }
            entry.fired.store(true, Ordering::Release);
            shutdown_read(entry.peer, entry.listen_port);
            false
        });
        let next = entries
            .iter()
            .map(|entry| entry.due.saturating_duration_since(now))
            .min()
            .unwrap_or(Duration::from_secs(1))
            .min(Duration::from_secs(1));
        drop(
            shared
                .changed
                .wait_timeout(entries, next)
                .unwrap_or_else(|e| e.into_inner()),
        );
        true
    }
}

const BODY_TIMEOUT: &str = "request body not received in time";

/// Answer a request whose body may be partly unread.
fn reject(request: Request, reads: &BodyReads, status: u16, error: impl Into<String>) {
    // When tiny_http drops a request it reads and discards the rest of the
    // body, which blocks for as long as the client stalls. With the read side
    // shut down that drain ends at once; the response is still written. It
    // also ends the connection, which is what a misbehaving upload deserves.
    if let Some(peer) = request.remote_addr().copied() {
        shutdown_read(peer, reads.listen_port);
    }
    let origin = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Origin"))
        .map(|h| h.value.as_str().to_string());
    respond(
        request,
        status,
        json!({"ok": false, "error": error.into()}),
        origin.as_deref(),
    );
}

/// One slot of `BodyReads::limit`, released on drop.
struct Permit(Arc<AtomicUsize>);

impl Permit {
    fn acquire(in_flight: &Arc<AtomicUsize>, limit: usize) -> Option<Self> {
        // A plain compare-exchange loop: `fetch_update` is deprecated on newer
        // toolchains and its replacement is missing on older ones.
        let mut current = in_flight.load(Ordering::Acquire);
        loop {
            if current >= limit {
                return None;
            }
            match in_flight.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(Self(Arc::clone(in_flight))),
                Err(seen) => current = seen,
            }
        }
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Shut down the read side of this process's socket(s) connected to `peer`
/// (and, if known, accepted on `listen_port`). Returns whether one was found.
///
/// tiny_http exposes neither its sockets nor timeouts, so the socket is found
/// by its addresses among our descriptors. A blocked read on it then returns
/// EOF. The caller still holds the request, so the descriptor cannot have
/// been closed and reused in the meantime.
fn shutdown_read(peer: SocketAddr, listen_port: Option<u16>) -> bool {
    let Ok(dir) = std::fs::read_dir("/proc/self/fd").or_else(|_| std::fs::read_dir("/dev/fd"))
    else {
        return false;
    };
    let fds: Vec<libc::c_int> = dir
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .collect();
    let mut found = false;
    for fd in fds {
        let Some(remote) = socket_addr(fd, true) else {
            continue;
        };
        if remote.ip() != peer.ip() || remote.port() != peer.port() {
            continue;
        }
        if listen_port.is_some() && socket_addr(fd, false).map(|a| a.port()) != listen_port {
            continue;
        }
        if unsafe { libc::shutdown(fd, libc::SHUT_RD) } == 0 {
            found = true;
        }
    }
    found
}

/// `getpeername` (`peer`) or `getsockname` of an IP socket.
fn socket_addr(fd: libc::c_int, peer: bool) -> Option<SocketAddr> {
    // SAFETY: plain syscalls on a caller-supplied descriptor (EBADF/ENOTSOCK
    // are just errors) writing into a zeroed, correctly sized sockaddr_storage.
    let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    let address = std::ptr::addr_of_mut!(storage).cast::<libc::sockaddr>();
    let rc = unsafe {
        if peer {
            libc::getpeername(fd, address, &mut len)
        } else {
            libc::getsockname(fd, address, &mut len)
        }
    };
    if rc != 0 {
        return None;
    }
    match libc::c_int::from(storage.ss_family) {
        libc::AF_INET => {
            // SAFETY: ss_family says the storage holds a sockaddr_in.
            let v4 = unsafe { &*std::ptr::addr_of!(storage).cast::<libc::sockaddr_in>() };
            let ip = Ipv4Addr::from(u32::from_be(v4.sin_addr.s_addr));
            Some(SocketAddr::new(IpAddr::V4(ip), u16::from_be(v4.sin_port)))
        }
        libc::AF_INET6 => {
            // SAFETY: ss_family says the storage holds a sockaddr_in6.
            let v6 = unsafe { &*std::ptr::addr_of!(storage).cast::<libc::sockaddr_in6>() };
            let ip = Ipv6Addr::from(v6.sin6_addr.s6_addr);
            Some(SocketAddr::new(IpAddr::V6(ip), u16::from_be(v6.sin6_port)))
        }
        _ => None,
    }
}

pub fn route(
    method: &Method,
    path: &str,
    state: &AppState,
    body: &[u8],
    client_ip: &str,
    user_agent: Option<&str>,
) -> (u16, Value) {
    match (method, path) {
        // Auth
        (&Method::Post, "/api/auth/login") => handlers::login(state, body, client_ip, user_agent),
        // Batch — the dashboard's heartbeat; feeds Home, Signal and Modem/Data
        (&Method::Get, "/api/dashboard") => handlers::dashboard(state),
        // Device / system
        (&Method::Get, "/api/device") => handlers::device(state),
        (&Method::Get, "/api/cpu") => handlers::cpu(state),
        (&Method::Get, "/api/memory") => handlers::memory(state),
        (&Method::Get, "/api/system/top") => handlers::system_top(state),
        (&Method::Post, "/api/system/kill-bloat") => handlers::system_kill_bloat(state, body),
        (&Method::Post, "/api/system/restart-agent") => device_ext::agent_restart(state),
        (&Method::Post, "/api/device/reboot") => device_ext::device_reboot(state),
        (&Method::Post, "/api/device/shutdown") => device_ext::device_shutdown(state),
        (&Method::Get, "/api/device/battery-info") => network_ext::network_battery_ubus(state),
        (&Method::Get, "/api/device/thermal/all") => device_ext::device_thermal_all(state),
        (&Method::Get, "/api/device/battery/detail") => device_ext::device_battery_detail(state),
        (&Method::Get, "/api/device/charger") => device_ext::device_charger(state),
        (&Method::Get, "/api/device/charge-control") => device_ext::charge_control_get(state),
        (&Method::Put, "/api/device/charge-control") => device_ext::charge_control_set(state, body),
        // Network
        (&Method::Get, "/api/network/clients") => network_ext::network_clients(state),
        // WiFi
        (&Method::Get, "/api/wifi/status") => wifi::wifi_status(state),
        (&Method::Put, "/api/wifi/settings") => wifi::wifi_set(state, body),
        // Modem
        (&Method::Put, "/api/data-usage/reset-day") => {
            handlers::data_usage_reset_day_set(state, body)
        }
        (&Method::Get, "/api/modem/capabilities") => cell::modem_capabilities(state),
        (&Method::Put, "/api/modem/network-mode") => cell::modem_network_mode_set(state, body),
        // SMS
        (&Method::Get, "/api/sms/capabilities") => sms::sms_capabilities(state),
        (&Method::Post, "/api/sms/list") => sms::sms_list(state, body),
        (&Method::Post, "/api/sms/send") => sms::sms_send(state, body),
        (&Method::Post, "/api/sms/delete") => sms::sms_delete(state, body),
        (&Method::Post, "/api/sms/read") => sms::sms_mark_read(state, body),
        // SIM
        (&Method::Get, "/api/sim/info") => sim::sim_info(state),
        (&Method::Get, "/api/sim/imei") => sim::sim_imei(state),
        // eSIM profiles (SIM Management)
        (&Method::Get, "/api/esim/profiles") => state.esim.profiles(),
        (&Method::Post, "/api/esim/refresh") => state.esim.refresh(),
        (&Method::Post, "/api/esim/enable") => state.esim.switch(esim::Kind::Enable, body),
        (&Method::Post, "/api/esim/disable") => state.esim.switch(esim::Kind::Disable, body),
        (&Method::Post, "/api/esim/nickname") => state.esim.nickname(body),
        (&Method::Post, "/api/esim/lock") => state.esim.lock(body),
        (&Method::Post, "/api/esim/download") => state.esim.download(body),
        (&Method::Post, "/api/esim/download/confirm") => state.esim.download_confirm(body),
        (&Method::Post, "/api/esim/download/cancel") => state.esim.download_cancel(body),
        (&Method::Post, "/api/esim/delete") => state.esim.switch(esim::Kind::Delete, body),
        (&Method::Post, "/api/esim/receipts/send") => state.esim.receipts_send(),
        // Cell / band lock
        (&Method::Post, "/api/cell/lock/nr") => cell::cell_lock_nr(state, body),
        (&Method::Post, "/api/cell/lock/lte") => cell::cell_lock_lte(state, body),
        (&Method::Post, "/api/cell/lock/reset") => cell::cell_lock_reset(state),
        (&Method::Post, "/api/cell/band/nr") => cell::cell_band_nr(state, body),
        (&Method::Post, "/api/cell/band/lte") => cell::cell_band_lte(state, body),
        (&Method::Post, "/api/cell/band/reset") => cell::cell_band_reset(state),
        // Router
        (&Method::Get, "/api/router/dns") => router::router_dns_get(state),
        (&Method::Put, "/api/router/dns") => router::router_dns_set(state, body),
        (&Method::Get, "/api/router/lan") => router::router_lan_get(state),
        (&Method::Put, "/api/router/lan") => router::router_lan_set(state, body),
        (&Method::Post, "/api/router/lan/confirm") => router::router_lan_confirm(state, body),
        (&Method::Get, "/api/router/apn/mode") => router::router_apn_mode_get(state),
        (&Method::Put, "/api/router/apn/mode") => router::router_apn_mode_set(state, body),
        (&Method::Get, "/api/router/apn/profiles") => router::router_apn_profiles_get(state),
        (&Method::Post, "/api/router/apn/profiles") => router::router_apn_profiles_add(state, body),
        (&Method::Post, "/api/router/apn/profiles/delete") => {
            router::router_apn_profiles_delete(state, body)
        }
        (&Method::Post, "/api/router/apn/profiles/activate") => {
            router::router_apn_profiles_activate(state, body)
        }
        // USB
        (&Method::Get, "/api/usb/status") => usb::usb_status(state),
        (&Method::Put, "/api/usb/mode") => usb::usb_mode_set(state, body),
        (&Method::Put, "/api/usb/default") => usb::usb_default_set(state, body),
        (&Method::Put, "/api/usb/powerbank") => usb::usb_powerbank_set(state, body),
        // TTL clamping
        (&Method::Get, "/api/ttl/status") => ttl_status(),
        (&Method::Put, "/api/ttl/set") => ttl_set(body),
        (&Method::Delete, "/api/ttl/clear") => ttl_clear(),
        // AT console
        (&Method::Post, "/api/at/send") => at_console(state, body),
        (&Method::Get, "/api/at/port") => at_port(state),
        // Signal logger
        (&Method::Post, "/api/logger/signal/start") => signal_logger::start_logging(state, body),
        (&Method::Post, "/api/logger/signal/stop") => signal_logger::stop_logging(state),
        (&Method::Get, "/api/logger/signal/status") => signal_logger::status(state),
        // Connection logger
        (&Method::Post, "/api/logger/connection/start") => {
            connection_logger::start_logging(state, body)
        }
        (&Method::Post, "/api/logger/connection/stop") => connection_logger::stop_logging(state),
        (&Method::Get, "/api/logger/connection/status") => connection_logger::status(state),
        // Fallback
        _ => (404, json!({"ok": false, "error": "not found"})),
    }
}

// --- AT console ---

fn at_console(state: &AppState, body: &[u8]) -> (u16, Value) {
    let parsed: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({"ok": false, "error": "invalid JSON"})),
    };
    let command = match parsed["command"].as_str() {
        Some(c) if !c.is_empty() => c,
        _ => return (400, json!({"ok": false, "error": "missing 'command'"})),
    };
    let timeout = parsed["timeout"].as_u64().unwrap_or(2).min(30);

    if !is_at_command_allowed(command) {
        return (
            403,
            json!({"ok": false, "error": "command not allowed. Only read-only AT commands are permitted."}),
        );
    }

    match at_cmd::send(&state.at_port, command, timeout) {
        Ok(resp) => (200, json!({"ok": true, "data": {"response": resp.trim()}})),
        Err(e) => (503, json!({"ok": false, "error": e})),
    }
}

const AT_BLOCKED_PREFIXES: &[&str] = &[
    "AT+CFUN",
    "AT^",
    "AT$QCRMCALL",
    "AT+CLCK",
    "AT+CMGD",
    "AT+CMGF=1;+CMGS",
    "AT+CGDCONT=",
    "AT+CGACT=",
];

const AT_ALLOWED_EXACT: &[&str] = &[
    "AT",
    "ATI",
    "AT+CSQ",
    "AT+COPS?",
    "AT+COPS=?",
    "AT+CGDCONT?",
    "AT+CREG?",
    "AT+CGREG?",
    "AT+CEREG?",
    "AT+CGPADDR",
    "AT+CGACT?",
    "AT+CLAC",
    "AT+CGSN",
    "AT+CGMI",
    "AT+CGMM",
    "AT+CGMR",
    "AT+QENG=\"SERVINGCELL\"",
    "AT+QNWINFO",
    "AT+QRSRP",
    "AT+QRSRQ",
    "AT+QINISTAT",
    "AT+QSPN",
    "AT+QCIDINCOMING",
    "AT+CGCONTRDP",
];

fn is_at_command_allowed(cmd: &str) -> bool {
    let upper = cmd.trim().to_uppercase();
    if upper.is_empty() {
        return false;
    }
    for prefix in AT_BLOCKED_PREFIXES {
        if upper.starts_with(prefix) {
            return false;
        }
    }
    AT_ALLOWED_EXACT.contains(&upper.as_str())
}

/// GET /api/at/port — report the detected AT serial port (if any)
fn at_port(state: &AppState) -> (u16, Value) {
    match state.at_port.detect_serialized() {
        Some(port) => (
            200,
            json!({"ok": true, "data": {"port": port, "available": true}}),
        ),
        None => (
            200,
            json!({"ok": true, "data": {"port": null, "available": false}}),
        ),
    }
}

// --- TTL handlers ---

/// The mangle rules only change through `ttl_set`/`ttl_clear` (which invalidate
/// this) or a boot-time `start_ttl.sh`, so a short cache spares two forks per
/// read without ever showing a stale clamp after a change made here.
const TTL_STATUS_TTL: std::time::Duration = std::time::Duration::from_secs(10);
static TTL_STATUS: std::sync::LazyLock<crate::cache::Cached<Value>> =
    std::sync::LazyLock::new(crate::cache::Cached::default);

fn ttl_status() -> (u16, Value) {
    let data = TTL_STATUS.get_or_refresh(TTL_STATUS_TTL, read_ttl_status);
    (200, json!({"ok": true, "data": data}))
}

fn read_ttl_status() -> Value {
    let ipv4 = std::process::Command::new("iptables")
        .args(["-t", "mangle", "-L", "PREROUTING", "-n"])
        .bounded_output();
    let ipv6 = std::process::Command::new("ip6tables")
        .args(["-t", "mangle", "-L", "PREROUTING", "-n"])
        .bounded_output();
    let mut active = false;
    let mut ttl_value: u32 = 0;
    if let Ok(out) = &ipv4 {
        let s = String::from_utf8_lossy(&out.stdout);
        for line in s.lines() {
            if line.contains("TTL set to") {
                active = true;
                if let Some(v) = line.rsplit("TTL set to ").next() {
                    ttl_value = v.trim().parse().unwrap_or(0);
                }
                break;
            }
        }
    }
    let mut hl_active = false;
    if let Ok(out) = &ipv6 {
        let s = String::from_utf8_lossy(&out.stdout);
        for line in s.lines() {
            if line.contains("HL set to") {
                hl_active = true;
                if ttl_value == 0 {
                    if let Some(v) = line.rsplit("HL set to ").next() {
                        ttl_value = v.trim().parse().unwrap_or(0);
                    }
                }
                break;
            }
        }
    }
    json!({
        "active": active,
        "ipv6_active": hl_active,
        "ttl_value": ttl_value,
    })
}

fn ttl_set(body: &[u8]) -> (u16, Value) {
    let val: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return (400, json!({"ok": false, "error": "invalid JSON"})),
    };
    let ttl = match val.get("ttl").and_then(|v| v.as_u64()) {
        Some(v) if (1..=255).contains(&v) => v as u32,
        _ => return (400, json!({"ok": false, "error": "ttl must be 1-255"})),
    };
    // Clear existing rules first
    let _ = std::process::Command::new("sh").args(["-c",
        "iptables -t mangle -S PREROUTING 2>/dev/null | grep 'TTL --ttl-set' | while read -r rule; do iptables -t mangle $(echo \"$rule\" | sed 's/-A/-D/'); done"
    ]).bounded_output();
    let _ = std::process::Command::new("sh").args(["-c",
        "ip6tables -t mangle -S PREROUTING 2>/dev/null | grep 'HL --hl-set' | while read -r rule; do ip6tables -t mangle $(echo \"$rule\" | sed 's/-A/-D/'); done"
    ]).bounded_output();
    // Add new rules
    let r4 = std::process::Command::new("iptables")
        .args([
            "-t",
            "mangle",
            "-A",
            "PREROUTING",
            "-i",
            "br-lan",
            "-j",
            "TTL",
            "--ttl-set",
            &ttl.to_string(),
        ])
        .bounded_output();
    let r6 = std::process::Command::new("ip6tables")
        .args([
            "-t",
            "mangle",
            "-A",
            "PREROUTING",
            "-i",
            "br-lan",
            "-j",
            "HL",
            "--hl-set",
            &ttl.to_string(),
        ])
        .bounded_output();
    let ok4 = r4.map(|o| o.status.success()).unwrap_or(false);
    let ok6 = r6.map(|o| o.status.success()).unwrap_or(false);
    // The rules changed (or were at least cleared) whatever the outcome.
    TTL_STATUS.invalidate();
    // Persist to start_ttl.sh
    let script = format!(
        "#!/bin/sh\niptables  -t mangle -C PREROUTING -i br-lan -j TTL --ttl-set {ttl} 2>/dev/null ||   iptables  -t mangle -A PREROUTING -i br-lan -j TTL --ttl-set {ttl}\nip6tables -t mangle -C PREROUTING -i br-lan -j HL  --hl-set  {ttl} 2>/dev/null ||   ip6tables -t mangle -A PREROUTING -i br-lan -j HL  --hl-set  {ttl}\n"
    );
    let _ = std::fs::write("/data/local/tmp/start_ttl.sh", script);
    if ok4 || ok6 {
        (
            200,
            json!({"ok": true, "data": {"ttl": ttl, "ipv4": ok4, "ipv6": ok6}}),
        )
    } else {
        (
            500,
            json!({"ok": false, "error": format!("ipv4={ok4} ipv6={ok6}")}),
        )
    }
}

fn ttl_clear() -> (u16, Value) {
    let _ = std::process::Command::new("sh").args(["-c",
        "iptables -t mangle -S PREROUTING 2>/dev/null | grep 'TTL --ttl-set' | while read -r rule; do iptables -t mangle $(echo \"$rule\" | sed 's/-A/-D/'); done"
    ]).bounded_output();
    let _ = std::process::Command::new("sh").args(["-c",
        "ip6tables -t mangle -S PREROUTING 2>/dev/null | grep 'HL --hl-set' | while read -r rule; do ip6tables -t mangle $(echo \"$rule\" | sed 's/-A/-D/'); done"
    ]).bounded_output();
    TTL_STATUS.invalidate();
    // Remove persistence script content (keep file but make it a no-op)
    let _ = std::fs::write(
        "/data/local/tmp/start_ttl.sh",
        "#!/bin/sh\n# TTL disabled\n",
    );
    (200, json!({"ok": true}))
}

fn respond(request: Request, status: u16, body: Value, origin: Option<&str>) {
    let body_str = serde_json::to_string(&body).unwrap_or_default();
    let content_type = Header::from_bytes("Content-Type", "application/json").unwrap();
    let mut response = Response::from_string(body_str)
        .with_status_code(status)
        .with_header(content_type);
    for h in cors_headers(origin) {
        response = response.with_header(h);
    }
    let _ = request.respond(response);
}

#[cfg(test)]
mod body_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Instant;

    fn headers(list: &[(&str, &str)]) -> Vec<Header> {
        list.iter()
            .map(|(k, v)| Header::from_bytes(*k, *v).unwrap())
            .collect()
    }

    #[test]
    fn body_kind_mirrors_tiny_http_reader_choice() {
        use BodyKind::*;
        assert_eq!(body_kind(&[], None), Empty);
        assert_eq!(body_kind(&[], Some(0)), Empty);
        assert_eq!(body_kind(&[], Some(1024)), Buffered);
        assert_eq!(body_kind(&[], Some(1025)), Streamed);
        assert_eq!(body_kind(&[], Some(MAX_BODY)), Streamed);
        assert_eq!(body_kind(&[], Some(MAX_BODY + 1)), TooLarge);
        let expect = headers(&[("Expect", "100-continue")]);
        assert_eq!(body_kind(&expect, Some(10)), Streamed);
        let chunked = headers(&[("Transfer-Encoding", "chunked")]);
        assert_eq!(body_kind(&chunked, None), Streamed);
        let upgrade = headers(&[("Connection", "Upgrade"), ("Transfer-Encoding", "chunked")]);
        assert_eq!(body_kind(&upgrade, None), Empty);
        // tiny_http only looks at the first Connection header.
        let second = headers(&[("Connection", "keep-alive"), ("Connection", "upgrade")]);
        assert_eq!(body_kind(&second, Some(5000)), Streamed);
    }

    #[test]
    fn permits_are_capped_and_released() {
        let in_flight = Arc::new(AtomicUsize::new(0));
        let a = Permit::acquire(&in_flight, 2).unwrap();
        let _b = Permit::acquire(&in_flight, 2).unwrap();
        assert!(Permit::acquire(&in_flight, 2).is_none());
        drop(a);
        assert!(Permit::acquire(&in_flight, 2).is_some());
        assert_eq!(in_flight.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_panicking_handler_becomes_a_500_envelope() {
        let (status, body) = contain_route(|| panic!("handler bug"));
        assert_eq!(status, 500);
        assert_eq!(body, json!({"ok": false, "error": "internal error"}));
        assert_eq!(contain_route(|| (200, json!({"ok": true}))).0, 200);
    }

    /// A loopback server with `workers` workers running `take_body`. Every
    /// request that reaches `serve` reports its body length and gets a 200.
    struct Harness {
        addr: SocketAddr,
        reads: BodyReads,
        served: mpsc::Receiver<usize>,
    }

    fn harness(workers: usize, limit: usize, deadline: Duration) -> Harness {
        let server = Arc::new(Server::http("127.0.0.1:0").expect("bind loopback"));
        let addr = server.server_addr().to_ip().unwrap();
        let reads = BodyReads {
            in_flight: Arc::new(AtomicUsize::new(0)),
            limit,
            deadline,
            listen_port: Some(addr.port()),
            watchdog: Watchdog::new(),
        };
        let (tx, served) = mpsc::channel();
        for _ in 0..workers {
            let server = Arc::clone(&server);
            let reads = reads.clone();
            let tx = tx.clone();
            std::thread::spawn(move || {
                while let Ok(request) = server.recv() {
                    let tx = tx.clone();
                    take_body(request, &reads, move |request, body| {
                        let _ = tx.send(body.len());
                        respond(request, 200, json!({"ok": true}), None);
                    });
                }
            });
        }
        Harness {
            addr,
            reads,
            served,
        }
    }

    fn connect(addr: SocketAddr, head: &str, body: &[u8]) -> TcpStream {
        let mut client = TcpStream::connect(addr).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        client.write_all(head.as_bytes()).unwrap();
        client.write_all(body).unwrap();
        client
    }

    /// Everything the server sends until it closes (or 5 s pass).
    fn response(client: &mut TcpStream) -> String {
        let mut text = Vec::new();
        let _ = client.read_to_end(&mut text);
        String::from_utf8_lossy(&text).into_owned()
    }

    fn wait_for_zero(in_flight: &AtomicUsize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while in_flight.load(Ordering::Acquire) != 0 {
            assert!(Instant::now() < deadline, "body thread never finished");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    const STALLED: &str = "POST /x HTTP/1.1\r\nHost: a\r\nContent-Length: 5000\r\n\r\n";

    #[test]
    fn stalled_uploads_do_not_occupy_workers() {
        let deadline = Duration::from_millis(800);
        let h = harness(2, 8, deadline);
        // Twice as many stalled uploads as workers: each declares 5000
        // bytes, sends 10, then goes quiet with the socket open.
        let stalled: Vec<(Instant, TcpStream)> = (0..4)
            .map(|_| (Instant::now(), connect(h.addr, STALLED, b"0123456789")))
            .collect();
        std::thread::sleep(Duration::from_millis(100));
        let asked = Instant::now();
        let mut get = connect(
            h.addr,
            "GET /api/dashboard HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n",
            b"",
        );
        let reply = response(&mut get);
        let took = asked.elapsed();
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        assert!(took < Duration::from_millis(300), "GET took {took:?}");
        assert_eq!(h.served.recv_timeout(Duration::from_secs(1)).unwrap(), 0);
        for (opened, mut client) in stalled {
            let reply = response(&mut client);
            let after = opened.elapsed();
            assert!(reply.starts_with("HTTP/1.1 408"), "{reply}");
            assert!(reply.contains(BODY_TIMEOUT));
            assert!(after >= deadline, "cut early: {after:?}");
            assert!(
                after < deadline + Duration::from_millis(700),
                "cut late: {after:?}"
            );
        }
        // Cut off by the watchdog, not abandoned: every body thread is gone.
        wait_for_zero(&h.reads.in_flight);
        assert!(h.served.try_recv().is_err(), "a stalled upload was served");
    }

    #[test]
    fn slow_trickle_is_cut_at_the_total_deadline() {
        let deadline = Duration::from_millis(600);
        let h = harness(1, 8, deadline);
        let opened = Instant::now();
        let mut client = connect(h.addr, STALLED, b"");
        let mut writer = client.try_clone().unwrap();
        // One byte every 100 ms: never idle long, but 5000 bytes would take
        // over eight minutes.
        std::thread::spawn(move || {
            for _ in 0..50 {
                if writer.write_all(b"x").is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let reply = response(&mut client);
        let after = opened.elapsed();
        assert!(reply.starts_with("HTTP/1.1 408"), "{reply}");
        assert!(after >= deadline, "cut early: {after:?}");
        assert!(
            after < deadline + Duration::from_millis(700),
            "cut late: {after:?}"
        );
        wait_for_zero(&h.reads.in_flight);
    }

    #[test]
    fn uploads_beyond_the_in_flight_cap_are_refused_at_once() {
        let h = harness(1, 0, Duration::from_secs(30));
        let asked = Instant::now();
        let mut client = connect(h.addr, STALLED, b"");
        assert!(response(&mut client).starts_with("HTTP/1.1 503"));
        assert!(asked.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn oversized_declared_body_is_refused_without_reading_it() {
        let h = harness(1, 8, Duration::from_secs(30));
        let asked = Instant::now();
        let mut client = connect(
            h.addr,
            "POST /x HTTP/1.1\r\nHost: a\r\nContent-Length: 2000000\r\n\r\n",
            b"",
        );
        let reply = response(&mut client);
        assert!(reply.starts_with("HTTP/1.1 413"), "{reply}");
        assert!(reply.contains(TOO_LARGE));
        assert!(asked.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn complete_bodies_are_delivered() {
        let h = harness(1, 8, Duration::from_secs(5));
        // Streamed (over tiny_http's 1 KiB prebuffer), chunked, and buffered.
        let big = vec![b'x'; 3000];
        let cases: [(&str, &[u8], usize); 3] = [
            (
                "POST /x HTTP/1.1\r\nHost: a\r\nConnection: close\r\nContent-Length: 3000\r\n\r\n",
                &big,
                3000,
            ),
            (
                "POST /x HTTP/1.1\r\nHost: a\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n",
                b"5\r\nhello\r\n0\r\n\r\n",
                5,
            ),
            (
                "POST /x HTTP/1.1\r\nHost: a\r\nConnection: close\r\nContent-Length: 2\r\n\r\n",
                b"{}",
                2,
            ),
        ];
        for (head, body, len) in cases {
            let mut client = connect(h.addr, head, body);
            assert!(response(&mut client).starts_with("HTTP/1.1 200"));
            assert_eq!(h.served.recv_timeout(Duration::from_secs(5)).unwrap(), len);
        }
        wait_for_zero(&h.reads.in_flight);
    }
}

#[cfg(test)]
mod tests {
    use super::is_at_command_allowed;

    #[test]
    fn at_allowlist_is_exact_and_read_only() {
        assert!(is_at_command_allowed("AT"));
        assert!(is_at_command_allowed("at+csq"));
        assert!(is_at_command_allowed("AT+QENG=\"servingcell\""));
        assert!(!is_at_command_allowed("AT+CSQ=1"));
        assert!(!is_at_command_allowed("AT+FOO"));
        assert!(!is_at_command_allowed("AT+CFUN=1"));
        assert!(!is_at_command_allowed("AT^RESET"));
    }
}
