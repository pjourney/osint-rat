//! Background work as stored on a case: collectors, external tools, agent calls, and reports.
//!
//! The live event stream is [`crate::jobs::JobEvent`]. This module is the row that remains
//! after the task finishes.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::common::Timestamp;
use super::ids::{RefKind, ShortRef};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// Accepted by the job manager and waiting for a free slot.
    Queued,
    /// The task is running. The Jobs screen shows its progress line.
    Running,
    /// Finished and applied (or, for an agent, the reply was stored).
    Succeeded,
    /// Finished with an error string the Jobs screen can show.
    Failed,
    /// Cancelled from the Jobs screen, or cancelled because os-int shut down.
    Cancelled,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Running => "running",
            JobStatus::Succeeded => "succeeded",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> JobStatus {
        match value {
            "queued" => JobStatus::Queued,
            "running" => JobStatus::Running,
            "succeeded" => JobStatus::Succeeded,
            "cancelled" => JobStatus::Cancelled,
            _ => JobStatus::Failed,
        }
    }

    pub fn is_active(self) -> bool {
        matches!(self, JobStatus::Queued | JobStatus::Running)
    }

    /// A one-character status symbol for compact displays.
    pub fn symbol(self) -> &'static str {
        match self {
            JobStatus::Queued => "…",
            JobStatus::Running => "●",
            JobStatus::Succeeded => "✓",
            JobStatus::Failed => "✗",
            JobStatus::Cancelled => "⊘",
        }
    }
}

impl fmt::Display for JobStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A persisted record of background work: collectors, tools, agent calls, reports.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobRecord {
    pub id: Uuid,
    pub investigation_id: Option<Uuid>,
    pub seq: i64,
    /// Machine-readable kind: `collector:dns`, `tool:whois`, `agent`, `report`, ...
    pub kind: String,
    /// Short label for status displays (`DNS`, `RDAP`, `Claude`).
    pub label: String,
    pub description: String,
    pub status: JobStatus,
    /// The exact program executed, when the job runs a subprocess.
    pub program: Option<String>,
    /// The exact arguments, with secrets redacted.
    pub args: Vec<String>,
    pub created_at: Timestamp,
    pub started_at: Option<Timestamp>,
    pub finished_at: Option<Timestamp>,
    pub progress: Option<String>,
    /// Bounded log tail (stdout or progress messages).
    pub log: String,
    /// Bounded stderr tail.
    pub stderr: String,
    pub result: Option<String>,
    pub error: Option<String>,
    pub page_id: Option<Uuid>,
}

impl JobRecord {
    pub fn short_ref(&self) -> ShortRef {
        ShortRef::new(RefKind::Job, self.seq)
    }

    /// Wall-clock duration so far (or in total, once finished), in seconds.
    pub fn elapsed_seconds(&self, now: Timestamp) -> i64 {
        let Some(started) = self.started_at else {
            return 0;
        };
        let end = self.finished_at.unwrap_or(now);
        (end - started).num_seconds().max(0)
    }
}
