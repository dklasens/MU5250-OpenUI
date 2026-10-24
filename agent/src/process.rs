use std::process::{Command, Output};
use std::time::Duration;

pub trait BoundedCommand {
    fn bounded_output(&mut self) -> std::io::Result<Output>;
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
        process_runner::output(self, None, Duration::from_secs(10), 2 * 1024 * 1024)
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
