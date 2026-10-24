//! Bounded subprocess I/O. A deadline covers stdin, execution and pipe draining.
use std::io::{self, Read, Write};
use std::process::{Child, Command, Output, Stdio};
#[cfg(windows)]
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub const DEFAULT_OUTPUT_LIMIT: usize = 4 * 1024 * 1024;

#[cfg(windows)]
enum Completed {
    Input(io::Result<()>),
    Stdout(io::Result<Vec<u8>>),
    Stderr(io::Result<Vec<u8>>),
}

#[cfg(windows)]
fn read_bounded(mut reader: impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    (&mut reader)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        Err(io::Error::other("subprocess output limit exceeded"))
    } else {
        Ok(bytes)
    }
}

/// The caller supplies arguments as data; this function never builds shell text.
/// Long-lived daemons must explicitly detach/redirect their standard streams.
/// Commands must not attempt to escape their process group/job.
#[cfg(windows)]
pub fn output(
    command: &mut Command,
    input: Option<&[u8]>,
    timeout: Duration,
    output_limit: usize,
) -> io::Result<Output> {
    let deadline = Instant::now() + timeout;
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn()?;
    let group = ProcessGroup::new(&child);
    let group = match group {
        Ok(group) => group,
        Err(error) => {
            let _ = child.kill();
            // Job assignment failure must not bypass the caller's deadline.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return Err(error);
        }
    };
    let (tx, rx) = mpsc::channel();
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let writer = input.map(|bytes| (child.stdin.take().expect("piped stdin"), bytes.to_vec()));
    let mut readers = Vec::new();
    let out_tx = tx.clone();
    readers.push(std::thread::spawn(move || {
        let _ = out_tx.send(Completed::Stdout(read_bounded(stdout, output_limit)));
    }));
    let err_tx = tx.clone();
    readers.push(std::thread::spawn(move || {
        let _ = err_tx.send(Completed::Stderr(read_bounded(stderr, output_limit)));
    }));
    let mut input_done = writer.is_none();
    if let Some((mut stdin, bytes)) = writer {
        readers.push(std::thread::spawn(move || {
            let result = stdin.write_all(&bytes);
            drop(stdin); // EOF must reach the child before waiting for its exit.
            let _ = tx.send(Completed::Input(result));
        }));
    }
    let mut stdout = None;
    let mut stderr = None;
    let mut status = None;
    let result = (|| loop {
        while let Ok(done) = rx.try_recv() {
            match done {
                Completed::Input(result) => {
                    result?;
                    input_done = true;
                }
                Completed::Stdout(result) => stdout = Some(result?),
                Completed::Stderr(result) => stderr = Some(result?),
            }
        }
        if status.is_none() {
            status = child.try_wait()?;
        }
        if let (true, Some(stdout), Some(stderr), Some(status)) =
            (input_done, stdout.as_mut(), stderr.as_mut(), status)
        {
            return Ok(Output {
                status,
                stdout: std::mem::take(stdout),
                stderr: std::mem::take(stderr),
            });
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "subprocess deadline exceeded",
            ));
        }
        std::thread::sleep(
            Duration::from_millis(5).min(deadline.saturating_duration_since(Instant::now())),
        );
    })();
    if result.is_err() {
        group.kill();
        let _ = child.kill();
        // Do not turn an I/O deadline into an unbounded wait on a child stuck in
        // kernel I/O. A reaper retains ownership until that child becomes reapable.
        let cleanup = Instant::now() + Duration::from_millis(250);
        while matches!(child.try_wait(), Ok(None)) && Instant::now() < cleanup {
            std::thread::sleep(Duration::from_millis(5));
        }
        if matches!(child.try_wait(), Ok(None)) {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
    // On success all three completion messages arrived. On failure the group
    // was terminated; never join a pipe thread until it is known to be done.
    for reader in readers {
        if reader.is_finished() {
            let _ = reader.join();
        }
    }
    result
}

/// Unix uses nonblocking pipes and poll, without spawning helper threads per
/// ubus call on the modem. Every pipe and the child share the same deadline.
#[cfg(unix)]
pub fn output(
    command: &mut Command,
    input: Option<&[u8]>,
    timeout: Duration,
    limit: usize,
) -> io::Result<Output> {
    output_with_grace(command, input, timeout, Duration::ZERO, limit)
}

/// The error text when a child ignored SIGTERM for its whole grace period and
/// had to be killed.
pub const KILLED_AFTER_GRACE: &str = "subprocess killed after its grace period";

/// Like [`output`], but at the deadline the process group gets SIGTERM and up
/// to `grace` to finish, with its output still collected, before SIGKILL. For
/// children that must release something themselves before exiting, such as a
/// SIM logical channel. A child that exits within the grace period returns its
/// output normally; one that outlives it fails with [`KILLED_AFTER_GRACE`].
#[cfg(unix)]
pub fn output_with_grace(
    command: &mut Command,
    input: Option<&[u8]>,
    timeout: Duration,
    grace: Duration,
    limit: usize,
) -> io::Result<Output> {
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    let deadline = Instant::now() + timeout;
    command
        .process_group(0)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let group = ProcessGroup::new(&child)?;
    let mut stdin = child.stdin.take();
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut offset = 0;
    let mut status = None;
    let input = input.unwrap_or_default();
    let result = (|| {
        for fd in [
            stdin.as_ref().map(AsRawFd::as_raw_fd),
            stdout.as_ref().map(AsRawFd::as_raw_fd),
            stderr.as_ref().map(AsRawFd::as_raw_fd),
        ]
        .into_iter()
        .flatten()
        {
            unsafe {
                let flags = libc::fcntl(fd, libc::F_GETFL);
                if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
        }
        let mut stop_by: Option<Instant> = None;
        loop {
            let now = Instant::now();
            match stop_by {
                None if now >= deadline => {
                    if grace.is_zero() {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "subprocess deadline exceeded",
                        ));
                    }
                    group.terminate();
                    stdin.take();
                    stop_by = Some(now + grace);
                }
                Some(limit) if now >= limit => {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, KILLED_AFTER_GRACE));
                }
                _ => {}
            }
            if let Some(writer) = stdin.as_mut() {
                if offset < input.len() {
                    match writer.write(&input[offset..input.len().min(offset + 65536)]) {
                        Ok(n) => offset += n,
                        Err(e)
                            if matches!(
                                e.kind(),
                                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                            ) => {}
                        Err(e) => return Err(e),
                    }
                }
                if offset == input.len() {
                    stdin.take();
                }
            }
            drain(&mut stdout, &mut out, limit)?;
            drain(&mut stderr, &mut err, limit)?;
            if status.is_none() {
                status = child.try_wait()?;
            }
            if let Some(status) = status {
                if stdin.is_none() && stdout.is_none() && stderr.is_none() {
                    return Ok(Output {
                        status,
                        stdout: out,
                        stderr: err,
                    });
                }
            }
            let mut fds = [
                libc::pollfd {
                    fd: stdin.as_ref().map_or(-1, AsRawFd::as_raw_fd),
                    events: libc::POLLOUT,
                    revents: 0,
                },
                libc::pollfd {
                    fd: stdout.as_ref().map_or(-1, AsRawFd::as_raw_fd),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: stderr.as_ref().map_or(-1, AsRawFd::as_raw_fd),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let wait = stop_by
                .unwrap_or(deadline)
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(10) as i32;
            let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, wait) };
            if result < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                return Err(io::Error::last_os_error());
            }
        }
    })();
    if result.is_err() {
        group.kill();
        let _ = child.kill();
        let cleanup = Instant::now() + Duration::from_millis(250);
        while matches!(child.try_wait(), Ok(None)) && Instant::now() < cleanup {
            std::thread::sleep(Duration::from_millis(5));
        }
        if matches!(child.try_wait(), Ok(None)) {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
    result
}

/// What [`LineSession::next_line`] saw.
#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
pub enum LineEvent {
    Line(String),
    /// Nothing arrived before the caller's `until`.
    Wait,
    /// The child closed its output (it exited, or is exiting).
    Eof,
}

/// A long-running child the caller talks to in lines: the request and later control lines on
/// stdin, progress and the answer on stdout. Stderr is inherited. One deadline covers the whole
/// session; when it passes the process group gets SIGTERM and `grace` to finish before SIGKILL,
/// as with [`output_with_grace`]. Lines and the total output are capped.
#[cfg(unix)]
pub struct LineSession {
    child: Child,
    group: ProcessGroup,
    stdin: Option<std::process::ChildStdin>,
    lines: std::sync::mpsc::Receiver<io::Result<String>>,
    deadline: Instant,
    grace: Duration,
    stop_by: Option<Instant>,
    done: bool,
}

#[cfg(unix)]
impl LineSession {
    pub fn spawn(
        command: &mut Command,
        first_line: &[u8],
        timeout: Duration,
        grace: Duration,
        line_limit: usize,
        total_limit: usize,
    ) -> io::Result<Self> {
        use std::io::BufRead;
        use std::os::unix::process::CommandExt;
        let deadline = Instant::now() + timeout;
        command
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = command.spawn()?;
        let group = ProcessGroup::new(&child)?;
        let mut stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(stdout);
            let mut total = 0usize;
            loop {
                let mut line = String::new();
                match (&mut reader)
                    .take(line_limit as u64 + 1)
                    .read_line(&mut line)
                {
                    Ok(0) => return,
                    Ok(n) => {
                        total += n;
                        if n > line_limit || total > total_limit {
                            let _ =
                                tx.send(Err(io::Error::other("subprocess output limit exceeded")));
                            return;
                        }
                        let line = line.trim_end_matches(['\r', '\n']).to_string();
                        if tx.send(Ok(line)).is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        return;
                    }
                }
            }
        });
        if let Some(writer) = stdin.as_mut() {
            if let Err(e) = writer.write_all(first_line).and_then(|()| writer.flush()) {
                group.kill();
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        }
        Ok(LineSession {
            child,
            group,
            stdin,
            lines,
            deadline,
            grace,
            stop_by: None,
            done: false,
        })
    }

    /// Sends one control line. Fails once stdin is closed or the child is gone.
    pub fn send_line(&mut self, line: &str) -> io::Result<()> {
        let writer = self
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "stdin closed"))?;
        writer.write_all(line.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()
    }

    /// Closes stdin: the child sees end of input.
    pub fn close_stdin(&mut self) {
        self.stdin.take();
    }

    /// The next stdout line, waiting until `until` at most. When the session deadline passes
    /// the child is asked to stop (SIGTERM) and its remaining lines still arrive; after the
    /// grace period it is killed.
    pub fn next_line(&mut self, until: Instant) -> io::Result<LineEvent> {
        loop {
            let now = Instant::now();
            match self.stop_by {
                None if now >= self.deadline => {
                    self.group.terminate();
                    self.stdin.take();
                    self.stop_by = Some(now + self.grace);
                }
                Some(limit) if now >= limit => {
                    self.group.kill();
                    return Err(io::Error::new(io::ErrorKind::TimedOut, KILLED_AFTER_GRACE));
                }
                _ => {}
            }
            let limit = self.stop_by.unwrap_or(self.deadline).min(until);
            let wait = limit
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(200));
            match self.lines.recv_timeout(wait) {
                Ok(Ok(line)) => return Ok(LineEvent::Line(line)),
                Ok(Err(e)) => {
                    self.group.kill();
                    return Err(e);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(LineEvent::Eof),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if Instant::now() >= until && until < self.stop_by.unwrap_or(self.deadline) {
                        return Ok(LineEvent::Wait);
                    }
                }
            }
        }
    }

    /// Waits for the child to exit (stdin closed first), under the same deadline and grace.
    pub fn finish(mut self) -> io::Result<std::process::ExitStatus> {
        self.stdin.take();
        loop {
            if let Some(status) = self.child.try_wait()? {
                self.done = true;
                return Ok(status);
            }
            let now = Instant::now();
            match self.stop_by {
                None if now >= self.deadline => {
                    self.group.terminate();
                    self.stop_by = Some(now + self.grace);
                }
                Some(limit) if now >= limit => {
                    self.group.kill();
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    self.done = true;
                    return Err(io::Error::new(io::ErrorKind::TimedOut, KILLED_AFTER_GRACE));
                }
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[cfg(unix)]
impl Drop for LineSession {
    /// A session dropped early is stopped the same way: SIGTERM, grace, then SIGKILL.
    fn drop(&mut self) {
        if self.done {
            return;
        }
        self.stdin.take();
        if matches!(self.child.try_wait(), Ok(None)) {
            self.group.terminate();
            let limit = Instant::now() + self.grace;
            while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < limit {
                std::thread::sleep(Duration::from_millis(20));
            }
            if matches!(self.child.try_wait(), Ok(None)) {
                self.group.kill();
                let _ = self.child.kill();
            }
        }
        let _ = self.child.wait();
    }
}

#[cfg(unix)]
fn drain<R: Read>(reader: &mut Option<R>, output: &mut Vec<u8>, limit: usize) -> io::Result<()> {
    if let Some(stream) = reader.as_mut() {
        let mut buffer = [0; 8192];
        // Limit work per turn so an endless stream cannot starve the deadline
        // or the other output pipe.
        for _ in 0..8 {
            match stream.read(&mut buffer) {
                Ok(0) => {
                    reader.take();
                    break;
                }
                Ok(n) => {
                    if output.len().saturating_add(n) > limit {
                        return Err(io::Error::other("subprocess output limit exceeded"));
                    }
                    output.extend_from_slice(&buffer[..n]);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
struct ProcessGroup(i32);
#[cfg(unix)]
impl ProcessGroup {
    fn new(child: &Child) -> io::Result<Self> {
        Ok(Self(child.id() as i32))
    }
    fn kill(&self) {
        unsafe {
            libc::kill(-self.0, libc::SIGKILL);
        }
    }
    fn terminate(&self) {
        unsafe {
            libc::kill(-self.0, libc::SIGTERM);
        }
    }
}

#[cfg(windows)]
struct ProcessGroup(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl ProcessGroup {
    fn new(child: &Child) -> io::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let job = Self(handle);
            // Explicit termination on failure. Successful commands may launch
            // an intentional ADB server, so closing the job must not kill it.
            if AssignProcessToJobObject(handle, child.as_raw_handle()) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(job)
        }
    }
    fn kill(&self) {
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0, 1);
        }
    }
}
#[cfg(windows)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn run(script: &str, input: Option<&[u8]>, limit: usize) -> io::Result<Output> {
        output(
            Command::new("sh").args(["-c", script]),
            input,
            Duration::from_millis(150),
            limit,
        )
    }
    #[test]
    fn blocked_stdin_is_inside_deadline() {
        let start = Instant::now();
        let error = run("sleep 5", Some(&vec![0; 4 * 1024 * 1024]), 1024).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn inherited_output_pipe_is_inside_deadline() {
        let start = Instant::now();
        assert_eq!(
            run("sleep 5 & exit 0", None, 1024).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn collects_both_pipes_and_delivers_eof() {
        let result = run("cat; printf error >&2", Some(b"hello"), 1024).unwrap();
        assert!(result.status.success());
        assert_eq!(result.stdout, b"hello");
        assert_eq!(result.stderr, b"error");
    }
    #[test]
    fn a_child_that_honours_sigterm_reports_its_own_output() {
        let start = Instant::now();
        let result = output_with_grace(
            Command::new("sh").args(["-c", "trap 'echo closed; exit 3' TERM; sleep 5 & wait"]),
            None,
            Duration::from_millis(150),
            Duration::from_secs(2),
            1024,
        )
        .unwrap();
        assert_eq!(result.stdout, b"closed\n");
        assert_eq!(result.status.code(), Some(3));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn a_child_that_ignores_sigterm_is_killed_after_its_grace() {
        let start = Instant::now();
        let error = output_with_grace(
            Command::new("sh").args(["-c", "trap '' TERM; sleep 5"]),
            None,
            Duration::from_millis(100),
            Duration::from_millis(200),
            1024,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), KILLED_AFTER_GRACE);
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    fn session(script: &str, timeout_ms: u64) -> LineSession {
        LineSession::spawn(
            Command::new("sh").args(["-c", script]),
            b"request\n",
            Duration::from_millis(timeout_ms),
            Duration::from_millis(500),
            256,
            4096,
        )
        .unwrap()
    }
    #[test]
    fn a_session_exchanges_lines_and_finishes() {
        let mut s = session(
            "read r; echo \"got $r\"; echo progress; read c; echo \"ctl $c\"; exit 4",
            5000,
        );
        let soon = || Instant::now() + Duration::from_secs(2);
        assert_eq!(
            s.next_line(soon()).unwrap(),
            LineEvent::Line("got request".into())
        );
        assert_eq!(
            s.next_line(soon()).unwrap(),
            LineEvent::Line("progress".into())
        );
        assert_eq!(
            s.next_line(Instant::now() + Duration::from_millis(100))
                .unwrap(),
            LineEvent::Wait,
            "the child is waiting for a control line"
        );
        s.send_line("install").unwrap();
        assert_eq!(
            s.next_line(soon()).unwrap(),
            LineEvent::Line("ctl install".into())
        );
        assert_eq!(s.next_line(soon()).unwrap(), LineEvent::Eof);
        assert_eq!(s.finish().unwrap().code(), Some(4));
    }
    #[test]
    fn closing_stdin_reaches_the_child_as_end_of_input() {
        let mut s = session("read r; if read c; then echo more; else echo eof; fi", 5000);
        s.close_stdin();
        let line = s
            .next_line(Instant::now() + Duration::from_secs(2))
            .unwrap();
        assert_eq!(line, LineEvent::Line("eof".into()));
    }
    #[test]
    fn a_session_past_its_deadline_gets_sigterm_then_its_last_words() {
        let start = Instant::now();
        let mut s = session(
            "trap 'echo stopped; exit 0' TERM; read r; sleep 5 & wait",
            200,
        );
        let far = Instant::now() + Duration::from_secs(10);
        assert_eq!(s.next_line(far).unwrap(), LineEvent::Line("stopped".into()));
        assert_eq!(s.next_line(far).unwrap(), LineEvent::Eof);
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn a_session_that_ignores_sigterm_is_killed_after_grace() {
        let mut s = session("trap '' TERM; read r; sleep 5", 100);
        let err = s
            .next_line(Instant::now() + Duration::from_secs(10))
            .unwrap_err();
        assert_eq!(err.to_string(), KILLED_AFTER_GRACE);
    }
    #[test]
    fn session_lines_are_capped() {
        let mut s = session("read r; head -c 1000 /dev/zero | tr '\\0' x; echo", 5000);
        let err = s
            .next_line(Instant::now() + Duration::from_secs(2))
            .unwrap_err();
        assert!(err.to_string().contains("output limit"));
    }
    #[test]
    fn terminates_excess_output() {
        assert!(run("yes", None, 1024)
            .unwrap_err()
            .to_string()
            .contains("output limit"));
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    fn command(script: &str) -> Command {
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
        command
    }
    #[test]
    fn blocked_stdin_obeys_windows_deadline() {
        let start = Instant::now();
        let result = output(
            &mut command("Start-Sleep -Seconds 10"),
            Some(&vec![0; 4 * 1024 * 1024]),
            Duration::from_millis(250),
            1024,
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn windows_output_is_bounded() {
        let result = output(
            &mut command("[Console]::Write(('x' * 1000000))"),
            None,
            Duration::from_secs(15),
            1024,
        );
        assert!(result.unwrap_err().to_string().contains("output limit"));
    }
    #[test]
    fn windows_input_eof_and_both_outputs_are_collected() {
        let result = output(
            &mut command(
                "[Console]::Write([Console]::In.ReadToEnd()); [Console]::Error.Write('error')",
            ),
            Some(b"hello"),
            Duration::from_secs(15),
            1024,
        )
        .unwrap();
        assert!(result.status.success());
        assert_eq!(result.stdout, b"hello");
        assert_eq!(result.stderr, b"error");
    }
}
