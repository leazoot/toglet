//! The NDJSON line transport over one app server subprocess.
//!
//! An illegal frame gets no answer while the server stays alive, so every read has a deadline;
//! lines arrive through a channel because the standard library cannot time out a pipe read.

use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::Duration;

use super::process::{AppServerProcess, CodexBinary};
use crate::codex_home::ServerHome;
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

pub struct AppServerClient {
    // Declaration order is drop order and it matters here: the subprocess must be gone before
    // the home it was started against is dropped, because an isolated home deletes itself.
    process: AppServerProcess,
    lines: Receiver<std::io::Result<String>>,
    reader: Option<JoinHandle<()>>,
    home: ServerHome,
}

impl AppServerClient {
    /// Starts an app server against `home` and takes ownership of both.
    pub fn start(binary: &CodexBinary, home: impl Into<ServerHome>) -> Result<Self> {
        let home = home.into();
        let phase = home.phase();
        let (process, stdout) = AppServerProcess::spawn(binary, home.path(), phase)?;

        let (sender, lines) = mpsc::channel();
        // The reader thread blocks on the pipe so the caller can block on the channel with a
        // deadline; it ends by itself when the pipe closes at process exit.
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });

        Ok(Self {
            process,
            lines,
            reader: Some(reader),
            home,
        })
    }

    pub fn home(&self) -> &ServerHome {
        &self.home
    }

    /// The subprocess id. For the client probe's exclusion list and nothing else.
    pub fn pid(&self) -> u32 {
        self.process.id()
    }

    /// Writes one NDJSON frame.
    pub fn send_line(&mut self, line: &str) -> Result<()> {
        let phase = self.process.phase();
        let stdin = self
            .process
            .stdin()
            .ok_or_else(|| crashed(phase, "the app server input stream is already closed"))?;

        stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
            .and_then(|()| stdin.flush())
            .map_err(|error| {
                crashed(phase, "writing to the app server failed").with_detail(&error.to_string())
            })
    }

    /// Reads one NDJSON frame, or gives up after `timeout`.
    pub fn recv_line(&mut self, timeout: Duration) -> Result<String> {
        let phase = self.process.phase();
        match self.lines.recv_timeout(timeout) {
            Ok(Ok(line)) => Ok(line),
            Ok(Err(error)) => Err(crashed(phase, "reading from the app server failed")
                .with_detail(&error.to_string())),
            Err(RecvTimeoutError::Timeout) => Err(unresponsive(phase)),
            // The reader thread reached end of stream, which means the process closed stdout.
            Err(RecvTimeoutError::Disconnected) => {
                Err(crashed(phase, "the app server closed its output stream"))
            }
        }
    }

    /// Shuts the subprocess down and reports whether it exited cleanly.
    pub fn shutdown(mut self) -> Result<()> {
        self.stop()
    }

    /// Idempotent, so both `shutdown` and `Drop` can call it.
    fn stop(&mut self) -> Result<()> {
        let result = self.process.finish();
        // Joined only after the process is reaped, by which point the pipe is closed and the
        // thread has returned. Joining earlier would block until the server happened to stop.
        if let Some(reader) = self.reader.take() {
            drop(reader.join());
        }
        result
    }
}

impl Drop for AppServerClient {
    fn drop(&mut self) {
        // `AppServerProcess` records its own failures; this only has to make sure the reader
        // thread is joined rather than detached.
        drop(self.stop());
    }
}

fn crashed(phase: Phase, detail: &str) -> TogletError {
    TogletError::new(ErrorCode::AppServerCrashed, phase, true, UserAction::Retry)
        .with_detail(detail)
}

fn unresponsive(phase: Phase) -> TogletError {
    TogletError::new(
        ErrorCode::AppServerUnresponsive,
        phase,
        true,
        UserAction::Retry,
    )
    .with_detail("the app server accepted the request and did not answer")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Well above a cold start plus handshake (~0.4 s) or a quota read (~2.5 s) on a slow machine.
    const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

    /// Drives the real `codex app-server` against an empty isolated home: needs Codex installed,
    /// but no account or network.
    fn start() -> AppServerClient {
        let binary = CodexBinary::resolve(Phase::ReadQuota)
            .expect("Codex must be installed to run the app server tests");
        let home = crate::codex_home::IsolatedHome::create(Phase::ReadQuota)
            .expect("isolated home is created");
        AppServerClient::start(&binary, home).expect("the app server starts")
    }

    #[test]
    fn starts_handshakes_and_shuts_down_without_being_terminated() {
        let mut client = start();

        client
            .send_line(
                r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"toglet","title":"Toglet","version":"0.1.0"},"capabilities":{"experimentalApi":false}}}"#,
            )
            .expect("the request is written");
        let reply = client.recv_line(REPLY_TIMEOUT).expect("the server answers");

        assert!(
            reply.contains("userAgent"),
            "unexpected handshake reply shape"
        );
        // A clean exit code proves stdin closing was enough and nothing had to be terminated.
        client.shutdown().expect("the app server exits cleanly");
    }

    #[test]
    fn an_unanswered_request_times_out_instead_of_hanging() {
        let mut client = start();
        // An illegal frame produces no reply and the process stays alive.
        client.send_line("not json").expect("the frame is written");

        let error = client
            .recv_line(Duration::from_millis(500))
            .expect_err("an unanswered request must not block");

        assert_eq!(error.code(), ErrorCode::AppServerUnresponsive);
        assert_eq!(error.phase(), Phase::ReadQuota);
        assert!(error.retryable());
        client
            .shutdown()
            .expect("the app server still exits cleanly");
    }

    #[test]
    fn the_subprocess_is_reaped_when_the_caller_drops_the_client() {
        let client = start();
        let home = client.home().path().to_path_buf();

        drop(client);

        // The home is removed only after the process has been waited on, so its absence is
        // evidence that the subprocess was reaped and not merely abandoned.
        assert!(!home.exists());
    }

    #[test]
    fn shutting_down_twice_is_not_an_error() {
        let mut client = start();

        client.stop().expect("first shutdown succeeds");
        client.stop().expect("a repeated shutdown is a no-op");
    }
}
