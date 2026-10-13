//! Background jobs: collectors, external tools, agent calls, and report generation.
//!
//! Jobs run on Tokio tasks and report back through a bounded channel. Long-running work
//! never touches UI state directly; the UI applies results when `JobEvent::Finished` arrives.

mod manager;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub use manager::JobManager;

use crate::agent::AgentOutcome;
use crate::collectors::CollectionOutput;

/// Capacity of the job event channel. Log lines are dropped (and counted) rather than
/// blocking a job when the UI falls behind; results are never dropped.
pub const EVENT_CHANNEL_CAPACITY: usize = 1024;

/// A message from a running job to the UI task.
///
/// Log lines may be dropped when the channel is full. [`JobEvent::Finished`] is not dropped:
/// the sender waits until the UI has room for the result.
#[derive(Debug)]
pub enum JobEvent {
    /// The task has started. The row moves from queued to running.
    Started { job_id: Uuid },
    /// One line of output. `is_error` selects the stderr tail.
    Log {
        job_id: Uuid,
        line: String,
        is_error: bool,
    },
    /// A short status string, such as the URL being fetched.
    Progress { job_id: Uuid, message: String },
    /// Records the exact program and arguments a job is executing (already redacted).
    Command {
        job_id: Uuid,
        program: String,
        args: Vec<String>,
    },
    /// Streaming agent output, appended to the reply shown on the Agent screen.
    AgentDelta { job_id: Uuid, text: String },
    /// A phase label from the agent CLI (`thinking`, a tool name, and similar).
    AgentStatus { job_id: Uuid, status: String },
    /// The task finished. `outcome` carries the batch, the agent reply, the report, or the error.
    Finished { job_id: Uuid, outcome: JobOutcome },
}

#[derive(Debug)]
pub enum JobOutput {
    Collection(CollectionOutput),
    Agent(AgentOutcome),
    Report(ReportOutput),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportOutput {
    /// Path relative to the investigation directory.
    pub relative_path: String,
    pub format: String,
    pub size_bytes: u64,
    pub sha256: String,
}

/// How a job ended. The UI applies a successful collection in one SQLite transaction.
#[derive(Debug)]
pub enum JobOutcome {
    /// The work finished and produced a batch, an agent reply, or a report.
    Succeeded(Box<JobOutput>),
    /// A user-facing error. The job row stores this string.
    Failed(String),
    /// The analyst cancelled the job, or shutdown cancelled it.
    Cancelled,
}

/// Handle passed to running work for reporting progress and observing cancellation.
#[derive(Clone, Debug)]
pub struct JobContext {
    pub job_id: Uuid,
    pub cancel: CancellationToken,
    sender: Option<mpsc::Sender<JobEvent>>,
    echo_to_stderr: bool,
}

impl JobContext {
    pub(crate) fn attached(
        job_id: Uuid,
        cancel: CancellationToken,
        sender: mpsc::Sender<JobEvent>,
    ) -> Self {
        Self {
            job_id,
            cancel,
            sender: Some(sender),
            echo_to_stderr: false,
        }
    }

    /// A context for running work outside the TUI (CLI commands, tests).
    pub fn detached(echo_to_stderr: bool) -> Self {
        Self {
            job_id: Uuid::new_v4(),
            cancel: CancellationToken::new(),
            sender: None,
            echo_to_stderr,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Sends an event without waiting. Returns `false` if it could not be delivered.
    pub fn try_send(&self, event: JobEvent) -> bool {
        match &self.sender {
            Some(sender) => sender.try_send(event).is_ok(),
            None => false,
        }
    }

    pub fn log(&self, line: impl Into<String>) {
        let line = line.into();
        tracing::debug!(job = %self.job_id, "{line}");
        if self.echo_to_stderr {
            eprintln!("  {}", crate::sanitize::sanitize_line(&line));
        }
        self.try_send(JobEvent::Log {
            job_id: self.job_id,
            line,
            is_error: false,
        });
    }

    pub fn error_line(&self, line: impl Into<String>) {
        let line = line.into();
        if self.echo_to_stderr {
            eprintln!("  ! {}", crate::sanitize::sanitize_line(&line));
        }
        self.try_send(JobEvent::Log {
            job_id: self.job_id,
            line,
            is_error: true,
        });
    }

    pub fn progress(&self, message: impl Into<String>) {
        let message = message.into();
        if self.echo_to_stderr {
            eprintln!("» {}", crate::sanitize::sanitize_line(&message));
        }
        self.try_send(JobEvent::Progress {
            job_id: self.job_id,
            message,
        });
    }

    pub fn command(&self, program: &str, args: &[String]) {
        let display_args = crate::exec::redact::display_args(args);
        tracing::info!(job = %self.job_id, program, args = ?display_args, "executing");
        if self.echo_to_stderr {
            eprintln!("$ {}", crate::exec::redact::display_command(program, args));
        }
        self.try_send(JobEvent::Command {
            job_id: self.job_id,
            program: program.to_string(),
            args: display_args,
        });
    }
}

/// Coalesces streaming text so that a full channel never loses agent output: text that could
/// not be sent stays pending and is sent with the next chunk.
#[derive(Debug, Default)]
pub struct DeltaBuffer {
    pending: String,
}

impl DeltaBuffer {
    pub fn push(&mut self, context: &JobContext, text: &str) {
        self.pending.push_str(text);
        self.flush(context);
    }

    pub fn flush(&mut self, context: &JobContext) {
        if self.pending.is_empty() {
            return;
        }
        let event = JobEvent::AgentDelta {
            job_id: context.job_id,
            text: std::mem::take(&mut self.pending),
        };
        if let Some(sender) = &context.sender
            && let Err(mpsc::error::TrySendError::Full(JobEvent::AgentDelta { text, .. })) =
                sender.try_send(event)
        {
            self.pending = text;
        }
    }
}
