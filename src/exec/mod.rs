//! Subprocess execution. Every external program os-int runs (tools, git, agents) goes through
//! `run`, which:
//!
//! * never uses a shell — the program and each argument are passed verbatim,
//! * enforces a timeout and supports cancellation,
//! * runs the child in its own process group (Unix) and kills the whole group on cancel,
//! * bounds the output kept in memory, and caps individual line lengths.
//!
//! Output is returned raw (lossy UTF-8). Callers must sanitize before display or storage.

pub mod redact;

use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const READ_CHUNK_SIZE: usize = 8 * 1024;
/// Lines longer than this are split; protects against minified files or binary output.
pub const MAX_LINE_BYTES: usize = 64 * 1024;
const LINE_CHANNEL_CAPACITY: usize = 256;
const TERMINATE_GRACE: Duration = Duration::from_secs(2);
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 1024 * 1024;

/// What to run.
#[derive(Clone, Debug)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub stdin: Option<Vec<u8>>,
    pub timeout: Duration,
    /// Bytes of stdout retained in `ExecOutcome::stdout` (the head) and of stderr (the tail).
    pub max_output_bytes: usize,
}

impl CommandSpec {
    pub fn new(program: impl Into<String>, args: Vec<String>, timeout: Duration) -> Self {
        Self {
            program: program.into(),
            args,
            cwd: None,
            env: Vec::new(),
            stdin: None,
            timeout,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputLine {
    pub stream: Stream,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecOutcome {
    pub exit_code: Option<i32>,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub duration: Duration,
}

#[derive(Debug)]
pub enum ExecError {
    NotFound(String),
    Spawn(String, std::io::Error),
    TimedOut(Duration),
    Cancelled,
    Io(std::io::Error),
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::NotFound(program) => write!(formatter, "{program:?} was not found on PATH"),
            ExecError::Spawn(program, error) => {
                write!(formatter, "could not start {program:?}: {error}")
            }
            ExecError::TimedOut(timeout) => {
                write!(formatter, "timed out after {}s", timeout.as_secs())
            }
            ExecError::Cancelled => write!(formatter, "cancelled"),
            ExecError::Io(error) => write!(formatter, "I/O error: {error}"),
        }
    }
}

impl std::error::Error for ExecError {}

/// Runs a command to completion, streaming each output line to `on_line`.
pub async fn run(
    spec: &CommandSpec,
    cancel: &CancellationToken,
    mut on_line: impl FnMut(&OutputLine),
) -> Result<ExecOutcome, ExecError> {
    let started = Instant::now();
    let mut child = spawn(spec)?;
    write_stdin(&mut child, spec.stdin.clone());
    let mut lines = start_output_pumps(&mut child);
    let mut collector = OutputCollector::new(spec.max_output_bytes);
    let deadline = tokio::time::sleep(spec.timeout);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                terminate(&mut child).await;
                return Err(ExecError::Cancelled);
            }
            _ = &mut deadline => {
                terminate(&mut child).await;
                return Err(ExecError::TimedOut(spec.timeout));
            }
            line = lines.recv() => match line {
                Some(line) => {
                    on_line(&line);
                    collector.push(line);
                }
                None => break,
            },
        }
    }
    let status = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            terminate(&mut child).await;
            return Err(ExecError::Cancelled);
        }
        _ = &mut deadline => {
            terminate(&mut child).await;
            return Err(ExecError::TimedOut(spec.timeout));
        }
        status = child.wait() => status.map_err(ExecError::Io)?,
    };
    cleanup_process_group(&child);
    Ok(collector.finish(status.code(), status.success(), started.elapsed()))
}

fn spawn(spec: &CommandSpec) -> Result<Child, ExecError> {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .stdin(if spec.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    #[cfg(unix)]
    command.process_group(0);
    command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ExecError::NotFound(spec.program.clone())
        } else {
            ExecError::Spawn(spec.program.clone(), error)
        }
    })
}

/// Writes stdin on a separate task so a child that produces output before reading all of its
/// input cannot deadlock us. Dropping the handle closes the pipe.
fn write_stdin(child: &mut Child, input: Option<Vec<u8>>) {
    let (Some(mut stdin), Some(bytes)) = (child.stdin.take(), input) else {
        return;
    };
    tokio::spawn(async move {
        if let Err(error) = stdin.write_all(&bytes).await {
            tracing::debug!(%error, "child closed stdin early");
        }
        let _ = stdin.shutdown().await;
    });
}

fn start_output_pumps(child: &mut Child) -> mpsc::Receiver<OutputLine> {
    let (sender, receiver) = mpsc::channel(LINE_CHANNEL_CAPACITY);
    if let Some(stdout) = child.stdout.take() {
        tokio::spawn(pump_lines(stdout, Stream::Stdout, sender.clone()));
    }
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(pump_lines(stderr, Stream::Stderr, sender));
    }
    receiver
}

async fn pump_lines(
    mut reader: impl AsyncRead + Unpin,
    stream: Stream,
    sender: mpsc::Sender<OutputLine>,
) {
    let mut pending: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; READ_CHUNK_SIZE];
    loop {
        let read = match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        pending.extend_from_slice(&chunk[..read]);
        for line in drain_complete_lines(&mut pending) {
            if sender
                .send(OutputLine { stream, text: line })
                .await
                .is_err()
            {
                return;
            }
        }
    }
    if !pending.is_empty() {
        let text = decode_line(&pending);
        let _ = sender.send(OutputLine { stream, text }).await;
    }
}

/// Splits complete lines off the front of `pending`, also splitting overlong lines.
pub fn drain_complete_lines(pending: &mut Vec<u8>) -> Vec<String> {
    let mut lines = Vec::new();
    loop {
        if let Some(position) = pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = pending.drain(..=position).collect();
            lines.push(decode_line(&line[..line.len() - 1]));
        } else if pending.len() > MAX_LINE_BYTES {
            let line: Vec<u8> = pending.drain(..MAX_LINE_BYTES).collect();
            lines.push(decode_line(&line));
        } else {
            return lines;
        }
    }
}

fn decode_line(bytes: &[u8]) -> String {
    let trimmed = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    String::from_utf8_lossy(trimmed).into_owned()
}

/// Keeps the head of stdout and the tail of stderr within a byte budget.
struct OutputCollector {
    limit: usize,
    stdout: String,
    stderr: String,
    stdout_truncated: bool,
    stderr_truncated: bool,
}

impl OutputCollector {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        }
    }

    fn push(&mut self, line: OutputLine) {
        match line.stream {
            Stream::Stdout => {
                if self.stdout.len() + line.text.len() + 1 > self.limit {
                    self.stdout_truncated = true;
                    return;
                }
                self.stdout.push_str(&line.text);
                self.stdout.push('\n');
            }
            Stream::Stderr => {
                self.stderr.push_str(&line.text);
                self.stderr.push('\n');
                if self.stderr.len() > self.limit {
                    let cut = self.stderr.len() - self.limit;
                    let boundary = (cut..self.stderr.len())
                        .find(|index| self.stderr.is_char_boundary(*index))
                        .unwrap_or(self.stderr.len());
                    self.stderr.drain(..boundary);
                    self.stderr_truncated = true;
                }
            }
        }
    }

    fn finish(self, exit_code: Option<i32>, success: bool, duration: Duration) -> ExecOutcome {
        ExecOutcome {
            exit_code,
            success,
            stdout: self.stdout,
            stderr: self.stderr,
            stdout_truncated: self.stdout_truncated,
            stderr_truncated: self.stderr_truncated,
            duration,
        }
    }
}

/// Asks the process group to terminate, then kills it after a grace period.
async fn terminate(child: &mut Child) {
    signal_group(child, Signal::Terminate);
    if tokio::time::timeout(TERMINATE_GRACE, child.wait())
        .await
        .is_err()
    {
        signal_group(child, Signal::Kill);
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    cleanup_process_group(child);
}

/// Terminates helper processes that outlived the main child.
fn cleanup_process_group(child: &Child) {
    signal_group(child, Signal::Kill);
}

#[derive(Clone, Copy)]
enum Signal {
    Terminate,
    Kill,
}

#[cfg(unix)]
fn signal_group(child: &Child, signal: Signal) {
    let Some(pid) = child.id() else {
        return;
    };
    let Ok(group) = libc::pid_t::try_from(pid) else {
        return;
    };
    let number = match signal {
        Signal::Terminate => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
    };
    // SAFETY: `killpg` has no memory-safety preconditions; the group was created for this
    // child via `process_group(0)`, so its ID equals the child's PID.
    unsafe {
        libc::killpg(group, number);
    }
}

#[cfg(not(unix))]
fn signal_group(child: &Child, _signal: Signal) {
    let _ = child;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell_available() -> bool {
        std::path::Path::new("/bin/sh").exists()
    }

    #[test]
    fn drains_lines_and_splits_overlong_ones() {
        let mut input = b"one\r\ntwo\npartial".to_vec();
        let actual = drain_complete_lines(&mut input);
        assert_eq!(actual, vec!["one".to_string(), "two".to_string()]);
        assert_eq!(input, b"partial");
        let mut long = vec![b'a'; MAX_LINE_BYTES + 10];
        let split = drain_complete_lines(&mut long);
        assert_eq!(split.len(), 1);
        assert_eq!(long.len(), 10);
    }

    #[tokio::test]
    async fn runs_program_with_literal_arguments() {
        if !shell_available() {
            return;
        }
        let input_args = vec![
            "-c".to_string(),
            "printf '%s\\n' \"$1\"".to_string(),
            "sh".to_string(),
            "$(whoami); rm -rf /".to_string(),
        ];
        let spec = CommandSpec::new("/bin/sh", input_args, Duration::from_secs(5));
        let mut streamed = Vec::new();
        let actual = run(&spec, &CancellationToken::new(), |line| {
            streamed.push(line.text.clone())
        })
        .await
        .expect("runs");
        assert!(actual.success);
        assert_eq!(actual.stdout, "$(whoami); rm -rf /\n");
        assert_eq!(streamed, vec!["$(whoami); rm -rf /".to_string()]);
    }

    #[tokio::test]
    async fn passes_stdin_and_reports_exit_codes() {
        if !shell_available() {
            return;
        }
        let mut spec = CommandSpec::new(
            "/bin/sh",
            vec!["-c".into(), "cat; exit 3".into()],
            Duration::from_secs(5),
        );
        spec.stdin = Some(b"from stdin".to_vec());
        let actual = run(&spec, &CancellationToken::new(), |_| {})
            .await
            .expect("runs");
        assert_eq!(actual.exit_code, Some(3));
        assert!(!actual.success);
        assert_eq!(actual.stdout, "from stdin\n");
    }

    #[tokio::test]
    async fn missing_programs_are_reported() {
        let spec = CommandSpec::new(
            "os-int-definitely-missing-program",
            Vec::new(),
            Duration::from_secs(5),
        );
        let actual = run(&spec, &CancellationToken::new(), |_| {}).await;
        assert!(matches!(actual, Err(ExecError::NotFound(_))));
    }

    #[tokio::test]
    async fn timeouts_kill_the_process() {
        if !shell_available() {
            return;
        }
        let spec = CommandSpec::new(
            "/bin/sh",
            vec!["-c".into(), "sleep 30".into()],
            Duration::from_millis(200),
        );
        let started = Instant::now();
        let actual = run(&spec, &CancellationToken::new(), |_| {}).await;
        assert!(matches!(actual, Err(ExecError::TimedOut(_))));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn cancellation_kills_the_process_group() {
        if !shell_available() {
            return;
        }
        let spec = CommandSpec::new(
            "/bin/sh",
            vec!["-c".into(), "sleep 30 & sleep 30; wait".into()],
            Duration::from_secs(60),
        );
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            trigger.cancel();
        });
        let started = Instant::now();
        let actual = run(&spec, &cancel, |_| {}).await;
        assert!(matches!(actual, Err(ExecError::Cancelled)));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn output_is_bounded() {
        if !shell_available() {
            return;
        }
        let mut spec = CommandSpec::new(
            "/bin/sh",
            vec![
                "-c".into(),
                "i=0; while [ $i -lt 200 ]; do echo line-$i; i=$((i+1)); done".into(),
            ],
            Duration::from_secs(5),
        );
        spec.max_output_bytes = 100;
        let actual = run(&spec, &CancellationToken::new(), |_| {})
            .await
            .expect("runs");
        assert!(actual.stdout.len() <= 100);
        assert!(actual.stdout_truncated);
    }
}
