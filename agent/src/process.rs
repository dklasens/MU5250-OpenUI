use std::process::{Command, Output};
use std::time::Duration;

/// Default subprocess deadline.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(10);
const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;

pub trait BoundedCommand {
    fn bounded_output(&mut self) -> std::io::Result<Output>;
    /// `bounded_output` with a different deadline, for callers that must give
    /// up sooner (poll-driven reads on the dashboard heartbeat).
    fn bounded_output_within(&mut self, timeout: Duration) -> std::io::Result<Output>;
    /// For helpers that hold a resource they must release themselves: `input` on stdin, SIGTERM
    /// at `timeout`, then `grace` to exit before SIGKILL (see `process_runner::output_with_grace`).
    fn bounded_output_with_grace(
        &mut self,
        input: &[u8],
        timeout: Duration,
        grace: Duration,
        output_limit: usize,
    ) -> std::io::Result<Output>;
    /// A line-based conversation with a long-running helper (see `process_runner::LineSession`):
    /// `first_line` on stdin, control lines later, progress and the answer as stdout lines.
    fn bounded_session(
        &mut self,
        first_line: &[u8],
        timeout: Duration,
        grace: Duration,
    ) -> std::io::Result<process_runner::LineSession>;
}
impl BoundedCommand for Command {
    fn bounded_output(&mut self) -> std::io::Result<Output> {
        self.bounded_output_within(DEFAULT_DEADLINE)
    }
    fn bounded_output_within(&mut self, timeout: Duration) -> std::io::Result<Output> {
        process_runner::output(self, None, timeout, OUTPUT_LIMIT)
    }
    fn bounded_output_with_grace(
        &mut self,
        input: &[u8],
        timeout: Duration,
        grace: Duration,
        output_limit: usize,
    ) -> std::io::Result<Output> {
        process_runner::output_with_grace(self, Some(input), timeout, grace, output_limit)
    }
    fn bounded_session(
        &mut self,
        first_line: &[u8],
        timeout: Duration,
        grace: Duration,
    ) -> std::io::Result<process_runner::LineSession> {
        process_runner::LineSession::spawn(self, first_line, timeout, grace, 64 * 1024, 1024 * 1024)
    }
}
