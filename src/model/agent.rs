//! Questions sent to a local agent CLI, and the structured changes the agent may propose.
//!
//! A [`Proposal`] stays pending until the analyst accepts or rejects it. Accepting an
//! [`AgentAction::AddFinding`] writes a finding with provenance `agent`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::common::Timestamp;
use super::ids::{RefKind, ShortRef};
use super::job::JobStatus;

/// One question sent to an agent and its response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentRun {
    pub id: Uuid,
    pub investigation_id: Uuid,
    pub seq: i64,
    /// Runs in the same thread are follow-ups; earlier turns are included as context.
    pub thread_id: Uuid,
    pub page_id: Option<Uuid>,
    pub agent_name: String,
    pub program: String,
    /// Arguments as executed, with secrets redacted and large prompt text elided.
    pub args: Vec<String>,
    pub question: String,
    pub response: String,
    pub status: JobStatus,
    pub error: Option<String>,
    /// Short reference of the item highlighted when the question was asked (`E-4`).
    pub focus: Option<String>,
    /// The exact context pack sent to the agent, stored as an artifact.
    pub context_artifact_id: Option<Uuid>,
    pub job_id: Option<Uuid>,
    pub model: Option<String>,
    pub cost_usd: Option<f64>,
    pub created_at: Timestamp,
    pub finished_at: Option<Timestamp>,
}

impl AgentRun {
    pub fn short_ref(&self) -> ShortRef {
        ShortRef::new(RefKind::AgentRun, self.seq)
    }
}

/// A structured change an agent proposes. Nothing is applied until the analyst accepts it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentAction {
    AddFinding {
        title: String,
        #[serde(default)]
        details: String,
        #[serde(default)]
        confidence: Option<String>,
        #[serde(default)]
        sources: Vec<String>,
    },
    AddNote {
        body: String,
    },
    CreatePage {
        title: String,
        #[serde(default, alias = "type")]
        page_type: Option<String>,
        #[serde(default)]
        target: Option<String>,
        #[serde(default)]
        description: Option<String>,
    },
    LinkEntity {
        from: String,
        to: String,
        #[serde(alias = "relationship", alias = "kind")]
        relation: String,
        #[serde(default)]
        sources: Vec<String>,
    },
    RunCollector {
        collector: String,
        target: String,
    },
    GenerateReport {
        #[serde(default)]
        format: Option<String>,
    },
}

impl AgentAction {
    pub fn name(&self) -> &'static str {
        match self {
            AgentAction::AddFinding { .. } => "ADD_FINDING",
            AgentAction::AddNote { .. } => "ADD_NOTE",
            AgentAction::CreatePage { .. } => "CREATE_PAGE",
            AgentAction::LinkEntity { .. } => "LINK_ENTITY",
            AgentAction::RunCollector { .. } => "RUN_COLLECTOR",
            AgentAction::GenerateReport { .. } => "GENERATE_REPORT",
        }
    }

    /// A one-line human description used in proposal lists.
    pub fn summary(&self) -> String {
        match self {
            AgentAction::AddFinding { title, .. } => format!("+ Finding: {title}"),
            AgentAction::AddNote { body } => {
                let first_line = body.lines().next().unwrap_or_default();
                format!("+ Note: {first_line}")
            }
            AgentAction::CreatePage {
                title, page_type, ..
            } => format!(
                "+ Page: {title} ({})",
                page_type.as_deref().unwrap_or("generic")
            ),
            AgentAction::LinkEntity {
                from, to, relation, ..
            } => {
                format!("+ Link: {from} --{relation}--> {to}")
            }
            AgentAction::RunCollector { collector, target } => {
                format!("▶ Run collector {collector} on {target}")
            }
            AgentAction::GenerateReport { format } => {
                format!(
                    "▶ Generate {} report",
                    format.as_deref().unwrap_or("markdown")
                )
            }
        }
    }

    /// Source references cited by the proposal.
    pub fn cited_sources(&self) -> &[String] {
        match self {
            AgentAction::AddFinding { sources, .. } | AgentAction::LinkEntity { sources, .. } => {
                sources
            }
            _ => &[],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Pending,
    Accepted,
    Rejected,
    /// Could not be applied (unknown references, invalid target, ...).
    Invalid,
}

impl ProposalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ProposalStatus::Pending => "pending",
            ProposalStatus::Accepted => "accepted",
            ProposalStatus::Rejected => "rejected",
            ProposalStatus::Invalid => "invalid",
        }
    }

    pub fn parse(value: &str) -> ProposalStatus {
        match value {
            "accepted" => ProposalStatus::Accepted,
            "rejected" => ProposalStatus::Rejected,
            "invalid" => ProposalStatus::Invalid,
            _ => ProposalStatus::Pending,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub id: Uuid,
    pub investigation_id: Uuid,
    pub agent_run_id: Uuid,
    pub seq: i64,
    pub action: AgentAction,
    pub status: ProposalStatus,
    pub error: Option<String>,
    pub created_at: Timestamp,
    pub decided_at: Option<Timestamp>,
}

impl Proposal {
    pub fn short_ref(&self) -> ShortRef {
        ShortRef::new(RefKind::Proposal, self.seq)
    }
}
