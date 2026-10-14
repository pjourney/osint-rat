//! Playbook documents, planned Actions, and local run/watch records.
//!
//! Playbooks are YAML. Parsing lives in [`crate::playbook`]. This module is the typed
//! document and the outputs an executor returns. Automations propose; they do not
//! silently write findings. `RUN_PLAYBOOK` from an agent may only name a bundled id.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::agent::AgentAction;
use super::batch::EvidenceBatch;
use super::common::Timestamp;
use super::job::JobStatus;
use super::target::TargetKind;

/// On-disk / sidecar playbook envelope. Distinct from the investigation archive
/// [`crate::exchange::FORMAT_VERSION`].
pub const PLAYBOOK_FORMAT: &str = "os-int-playbook";
/// Current playbook document version. Newer documents are refused.
pub const PLAYBOOK_FORMAT_VERSION: u32 = 1;

/// Hard cap on an untrusted playbook file.
pub const MAX_PLAYBOOK_BYTES: u64 = 64 * 1024;
/// Hard cap on steps after parse. Documents cannot raise this.
pub const MAX_PLAYBOOK_STEPS: usize = 32;
/// Hard cap on `collect` steps.
pub const MAX_COLLECT_STEPS: usize = 16;
/// Hard cap on `run_tool` steps.
pub const MAX_TOOL_STEPS: usize = 8;
/// Hard cap on pending proposals created by one run.
pub const MAX_PLAYBOOK_PROPOSALS: usize = 16;

/// Where a parsed playbook came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaybookOrigin {
    Bundled,
    File(String),
}

/// Object type the playbook expects as its seed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaybookSeed {
    pub kind: TargetKind,
}

/// One step in a playbook. Nested playbooks are not a step type (loop guard).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaybookStep {
    Collect {
        collector: String,
    },
    SearchAround {
        hops: u32,
        link_types: Vec<String>,
        entity_kinds: Vec<String>,
    },
    RunTool {
        tool: String,
    },
    AssertFindingTemplate {
        title: String,
        details: String,
    },
    ProposeResolve {
        reason: String,
    },
}

impl PlaybookStep {
    pub fn kind_name(&self) -> &'static str {
        match self {
            PlaybookStep::Collect { .. } => "collect",
            PlaybookStep::SearchAround { .. } => "search_around",
            PlaybookStep::RunTool { .. } => "run_tool",
            PlaybookStep::AssertFindingTemplate { .. } => "assert_finding_template",
            PlaybookStep::ProposeResolve { .. } => "propose_resolve",
        }
    }
}

/// A validated playbook document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Playbook {
    pub id: String,
    pub name: String,
    pub description: String,
    pub seed: PlaybookSeed,
    pub steps: Vec<PlaybookStep>,
    pub origin: PlaybookOrigin,
}

impl Playbook {
    pub fn collect_ids(&self) -> Vec<&str> {
        self.steps
            .iter()
            .filter_map(|step| match step {
                PlaybookStep::Collect { collector } => Some(collector.as_str()),
                _ => None,
            })
            .collect()
    }

    pub fn tool_ids(&self) -> Vec<&str> {
        self.steps
            .iter()
            .filter_map(|step| match step {
                PlaybookStep::RunTool { tool } => Some(tool.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// One line of a dry-run plan.
#[derive(Clone, Debug, PartialEq)]
pub enum PlannedAction {
    Collect { collector: String },
    RunTool { tool: String },
    SearchAround { hops: u32 },
    Propose { action: AgentAction },
}

impl PlannedAction {
    pub fn describe(&self) -> String {
        match self {
            PlannedAction::Collect { collector } => format!("collect {collector}"),
            PlannedAction::RunTool { tool } => format!("run_tool {tool} (passive only)"),
            PlannedAction::SearchAround { hops } => format!("search_around hops={hops}"),
            PlannedAction::Propose { action } => format!("propose {}", action.summary()),
        }
    }
}

/// Dry-run plan: what a playbook would do without applying.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaybookPlan {
    pub playbook_id: String,
    pub playbook_name: String,
    pub seed_kind: String,
    pub seed_value: String,
    pub automation: bool,
    pub actions: Vec<PlannedAction>,
    pub warnings: Vec<String>,
}

impl PlaybookPlan {
    pub fn describe(&self) -> String {
        let mode = if self.automation {
            "automation dry-run"
        } else {
            "playbook dry-run"
        };
        let mut lines = vec![format!(
            "{mode}: {} ({}) on {} {}",
            self.playbook_name, self.playbook_id, self.seed_kind, self.seed_value
        )];
        if self.actions.is_empty() {
            lines.push("  (no Actions planned)".into());
        }
        for action in &self.actions {
            lines.push(format!("  - {}", action.describe()));
        }
        for warning in &self.warnings {
            lines.push(format!("  ! {warning}"));
        }
        lines.push("Nothing was applied.".into());
        lines.join("\n")
    }
}

/// Result of running collect/tool steps (still unapplied until storage commits).
#[derive(Clone, Debug, PartialEq)]
pub struct PlaybookOutput {
    pub playbook: Playbook,
    pub playbook_id: String,
    pub playbook_name: String,
    pub seed_kind: String,
    pub seed_value: String,
    pub dry_run: bool,
    pub automation: bool,
    pub batches: Vec<EvidenceBatch>,
    pub proposals: Vec<AgentAction>,
    pub plan: PlaybookPlan,
    pub search_summary: Option<String>,
    pub warnings: Vec<String>,
    pub digest: String,
}

/// A persisted playbook run. Local workbench record; not part of the archive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaybookRun {
    pub id: Uuid,
    pub investigation_id: Uuid,
    pub seq: i64,
    pub playbook_id: String,
    pub playbook_name: String,
    pub seed_kind: String,
    pub seed_value: String,
    pub dry_run: bool,
    pub automation: bool,
    pub status: JobStatus,
    pub summary: String,
    pub error: Option<String>,
    pub created_at: Timestamp,
    pub finished_at: Option<Timestamp>,
}

/// A watch set: re-run a playbook's collect steps and propose Actions on changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Automation {
    pub id: Uuid,
    pub investigation_id: Uuid,
    pub seq: i64,
    pub playbook_id: String,
    pub seed_kind: String,
    pub seed_value: String,
    pub interval_seconds: Option<i64>,
    pub enabled: bool,
    pub last_run_at: Option<Timestamp>,
    pub last_digest: Option<String>,
    pub created_at: Timestamp,
}
