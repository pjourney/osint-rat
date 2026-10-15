//! Review lane: AI, playbook, and merge proposals wait here until Shift+A then y.

use uuid::Uuid;

use super::{App, PendingResolve, View};
use crate::model::{
    AgentAction, Proposal, ProposalStatus, Provenance, ResolutionSuggestion, ShortRef, Workspace,
};

/// How a Review row was produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewKind {
    /// An agent CLI proposed this Action.
    Agent,
    /// A playbook or watch set proposed this Action.
    Playbook,
    /// A scored or manual merge suggestion.
    Merge,
    /// An agent run with no pending write. Transcript only.
    History,
}

/// One row in the Review board.
#[derive(Clone, Debug)]
pub struct ReviewItem {
    pub kind: ReviewKind,
    pub proposal_id: Option<Uuid>,
    pub run_id: Option<Uuid>,
    pub merge: Option<PendingResolve>,
    pub title: String,
    pub group: String,
    pub preview: Vec<String>,
    pub citations: Vec<ReviewCitation>,
    pub rationale: String,
    /// Provenance that accept will store. `None` means the row is not a write.
    pub written_as: Option<Provenance>,
}

impl ReviewItem {
    pub fn has_missing_citation(&self) -> bool {
        self.citations.iter().any(|citation| !citation.present)
    }

    pub fn is_ai(&self) -> bool {
        self.written_as == Some(Provenance::Agent)
    }

    pub fn is_writable(&self) -> bool {
        self.written_as.is_some() && (self.proposal_id.is_some() || self.merge.is_some())
    }
}

/// A cited source on a Review item. Missing sources block accept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewCitation {
    pub reference: String,
    pub present: bool,
    pub label: String,
}

impl App {
    pub fn pending_review_count(&self) -> usize {
        self.listed_review_items()
            .iter()
            .filter(|item| item.is_writable())
            .count()
    }

    pub fn listed_review_items(&self) -> Vec<ReviewItem> {
        let Some(workspace) = self.workspace.as_ref() else {
            return Vec::new();
        };
        let mut items = Vec::new();
        for proposal in workspace
            .data
            .proposals
            .iter()
            .filter(|proposal| proposal.status == ProposalStatus::Pending)
        {
            items.push(review_item_from_proposal(workspace, proposal));
        }
        for pending in &self.pending_merges {
            items.push(review_item_from_merge(workspace, pending, "manual merge"));
        }
        for suggestion in self.listed_suggestions() {
            let already = items.iter().any(|item| {
                item.merge
                    .as_ref()
                    .is_some_and(|merge| merge.entity_ids == suggestion.entity_ids)
            });
            if already {
                continue;
            }
            items.push(review_item_from_suggestion(workspace, &suggestion));
        }
        for run in &workspace.data.agent_runs {
            if items
                .iter()
                .any(|item| item.run_id == Some(run.id) && item.kind != ReviewKind::History)
            {
                continue;
            }
            items.push(review_item_from_run(run));
        }
        items
    }

    pub fn selected_review_item(&self) -> Option<ReviewItem> {
        self.listed_review_items().into_iter().nth(self.selected)
    }

    pub fn armed_review_items(&self) -> Vec<ReviewItem> {
        let items = self.listed_review_items();
        if self.review_marks.is_empty() {
            return self.selected_review_item().into_iter().collect();
        }
        items
            .into_iter()
            .enumerate()
            .filter(|(index, _)| self.review_marks.contains(index))
            .map(|(_, item)| item)
            .collect()
    }

    pub(crate) fn open_review_lane(&mut self) {
        self.switch_view(View::Review);
        self.review_armed = false;
        self.review_marks.clear();
        let count = self.pending_review_count();
        self.status = if count == 0 {
            "Review is empty. Agent, playbook, and merge proposals land here.".into()
        } else {
            format!("{count} item(s) waiting. Shift+A then y writes. Enter never writes.")
        };
    }

    pub(crate) fn arm_review(&mut self) {
        if self.view != View::Review {
            return;
        }
        let items = self.armed_review_items();
        if items.is_empty() || items.iter().any(|item| !item.is_writable()) {
            self.status = "Nothing to accept.".into();
            self.review_armed = false;
            return;
        }
        if items.iter().any(ReviewItem::has_missing_citation) {
            self.status = "A citation points at a missing source. Acceptance is blocked.".into();
            self.review_armed = false;
            return;
        }
        self.review_armed = true;
        self.status = confirm_status(&items);
    }

    pub(crate) fn disarm_review(&mut self) {
        if self.review_armed {
            self.review_armed = false;
            self.status = "Disarmed. Nothing written.".into();
        }
    }

    pub(crate) fn open_agent_history(&mut self) {
        self.open_review_lane();
        let Some(workspace) = self.workspace.as_ref() else {
            return;
        };
        let Some(run) = self
            .agent_run_id
            .and_then(|id| workspace.agent_run(id))
            .or_else(|| workspace.data.agent_runs.last())
        else {
            self.status = "No agent runs yet. Press a to ask.".into();
            return;
        };
        let run_id = run.id;
        let thread_id = run.thread_id;
        let reference = run.short_ref().to_string();
        self.agent_run_id = Some(run_id);
        self.agent_thread = Some(thread_id);
        if let Some(index) = self
            .listed_review_items()
            .iter()
            .position(|item| item.run_id == Some(run_id))
        {
            self.selected = index;
        }
        self.status = format!("Agent run {reference} — transcript in Inspector.");
    }

    pub fn confirm_accept_text(&self) -> String {
        confirm_status(&self.armed_review_items())
    }

    pub fn confirm_bar_text(&self) -> String {
        let items = self.armed_review_items();
        let ai = items.iter().filter(|item| item.is_ai()).count();
        let analyst = items
            .iter()
            .filter(|item| item.written_as == Some(Provenance::Analyst))
            .count();
        match (items.len(), ai, analyst) {
            (1, 1, _) => {
                " Accept 1 AI item? It keeps ◆ agent provenance.   y confirm   any other key cancels "
                    .into()
            }
            (1, _, 1) => {
                " Accept 1 analyst item? It keeps analyst provenance.   y confirm   any other key cancels "
                    .into()
            }
            (count, ai, 0) if ai == count => format!(
                " Accept {count} AI item(s)? They keep ◆ agent provenance.   y confirm   any other key cancels "
            ),
            (count, 0, analyst) if analyst == count => format!(
                " Accept {count} analyst item(s)? They keep analyst provenance.   y confirm   any other key cancels "
            ),
            (count, ai, analyst) => format!(
                " Accept {count} item(s): {ai} AI keep ◆, {analyst} analyst.   y confirm   any other key cancels "
            ),
        }
    }

    pub(crate) fn toggle_review_mark(&mut self) {
        if self.view != View::Review {
            return;
        }
        if self.review_marks.contains(&self.selected) {
            self.review_marks.retain(|index| *index != self.selected);
        } else {
            self.review_marks.push(self.selected);
        }
        self.status = format!("Marked {} Review item(s).", self.review_marks.len());
    }
}

fn review_item_from_proposal(workspace: &Workspace, proposal: &Proposal) -> ReviewItem {
    let run = workspace.agent_run(proposal.agent_run_id);
    let playbook = run
        .map(|run| run.agent_name == crate::storage::PLAYBOOK_AGENT_NAME)
        .unwrap_or(false);
    let kind = if playbook {
        ReviewKind::Playbook
    } else {
        ReviewKind::Agent
    };
    let group = run
        .map(|run| {
            format!(
                "{} {} · {}",
                proposal.short_ref(),
                run.short_ref(),
                run.agent_name
            )
        })
        .unwrap_or_else(|| proposal.short_ref().to_string());
    ReviewItem {
        kind,
        proposal_id: Some(proposal.id),
        run_id: Some(proposal.agent_run_id),
        merge: None,
        title: proposal.action.summary(),
        group,
        preview: preview_lines(&proposal.action, kind),
        citations: citations_for(workspace, proposal.action.cited_sources()),
        rationale: rationale_for(&proposal.action),
        written_as: Some(written_provenance(kind, &proposal.action)),
    }
}

fn review_item_from_merge(
    workspace: &Workspace,
    pending: &PendingResolve,
    rule: &str,
) -> ReviewItem {
    let labels = pending
        .entity_ids
        .iter()
        .filter_map(|id| {
            workspace
                .entity(*id)
                .map(|entity| format!("{} {}", entity.short_ref(), entity.display_name()))
        })
        .collect::<Vec<_>>();
    let canonical = workspace
        .entity(pending.canonical_id)
        .map(|entity| format!("{} {}", entity.short_ref(), entity.display_name()))
        .unwrap_or_else(|| pending.canonical_id.to_string());
    ReviewItem {
        kind: ReviewKind::Merge,
        proposal_id: None,
        run_id: None,
        merge: Some(pending.clone()),
        title: format!("Merge {}", labels.join(" + ")),
        group: format!("≈ {rule}"),
        preview: vec![
            "+ Resolution bag".into(),
            format!("  canonical  {canonical}"),
            format!("  members    {}", labels.join(", ")),
            "  provenance  analyst (reversible)".into(),
        ],
        citations: Vec::new(),
        rationale: pending.reason.clone(),
        written_as: Some(Provenance::Analyst),
    }
}

fn review_item_from_suggestion(
    workspace: &Workspace,
    suggestion: &ResolutionSuggestion,
) -> ReviewItem {
    let pending = PendingResolve {
        entity_ids: suggestion.entity_ids.clone(),
        canonical_id: suggestion.entity_ids.first().copied().unwrap_or_default(),
        reason: suggestion.reason.clone(),
    };
    let mut item = review_item_from_merge(
        workspace,
        &pending,
        &format!("{} score {}", suggestion.rule.as_str(), suggestion.score),
    );
    item.title = format!("{}  score {}", item.title, suggestion.score);
    item
}

fn review_item_from_run(run: &crate::model::AgentRun) -> ReviewItem {
    ReviewItem {
        kind: ReviewKind::History,
        proposal_id: None,
        run_id: Some(run.id),
        merge: None,
        title: run.question.clone(),
        group: format!("{} · {}", run.short_ref(), run.agent_name),
        preview: vec!["Transcript only. Nothing to accept.".into()],
        citations: Vec::new(),
        rationale: String::new(),
        written_as: None,
    }
}

fn written_provenance(kind: ReviewKind, action: &AgentAction) -> Provenance {
    match (kind, action) {
        (ReviewKind::Playbook, AgentAction::AssertFindingTemplate { .. }) => Provenance::Analyst,
        (ReviewKind::Merge, _) => Provenance::Analyst,
        _ => Provenance::Agent,
    }
}

fn confirm_status(items: &[ReviewItem]) -> String {
    let ai = items.iter().filter(|item| item.is_ai()).count();
    let analyst = items
        .iter()
        .filter(|item| item.written_as == Some(Provenance::Analyst))
        .count();
    match (items.len(), ai, analyst) {
        (1, 1, _) => {
            "Armed — accept 1 AI item. It keeps ◆ agent provenance. y confirm. Any other key cancels."
                .into()
        }
        (1, _, 1) => {
            "Armed — accept 1 analyst item. It keeps analyst provenance. y confirm. Any other key cancels."
                .into()
        }
        (count, ai, 0) if ai == count => format!(
            "Armed — accept {count} AI item(s). They keep ◆ agent provenance. y confirm. Any other key cancels."
        ),
        (count, 0, analyst) if analyst == count => format!(
            "Armed — accept {count} analyst item(s). They keep analyst provenance. y confirm. Any other key cancels."
        ),
        (count, ai, analyst) => format!(
            "Armed — accept {count} item(s): {ai} AI keep ◆, {analyst} analyst. y confirm. Any other key cancels."
        ),
    }
}

/// Resolve a cited source the same way Review preview and apply do.
pub fn resolve_source_citation(workspace: &Workspace, raw: &str) -> Option<Uuid> {
    resolve_citation(workspace, raw)
}

fn preview_lines(action: &AgentAction, kind: ReviewKind) -> Vec<String> {
    match action {
        AgentAction::AddFinding {
            title,
            details,
            confidence,
            ..
        } => vec![
            "+ Finding  ◆ AI".into(),
            format!("  {title}"),
            format!(
                "  confidence  {}",
                confidence.as_deref().unwrap_or("medium")
            ),
            "  provenance agent (kept forever)".into(),
            if details.is_empty() {
                String::new()
            } else {
                format!("  {}", details.lines().next().unwrap_or_default())
            },
        ]
        .into_iter()
        .filter(|line| !line.is_empty())
        .collect(),
        AgentAction::AssertFindingTemplate { title, details } => {
            let (glyph, label) = if kind == ReviewKind::Playbook {
                ("✎", "analyst (playbook template)")
            } else {
                ("◆", "agent (kept forever)")
            };
            vec![
                format!("+ Finding template  {glyph} {label}"),
                format!("  {title}"),
                format!("  {}", details.lines().next().unwrap_or_default()),
            ]
        }
        AgentAction::AddNote { body } => vec![
            "+ Note  ◆ AI".into(),
            format!("  {}", body.lines().next().unwrap_or_default()),
            "  provenance agent (kept forever)".into(),
        ],
        AgentAction::CreatePage {
            title,
            page_type,
            target,
            ..
        } => vec![
            "+ Target".into(),
            format!("  {title} ({})", page_type.as_deref().unwrap_or("generic")),
            format!("  {}", target.as_deref().unwrap_or("-")),
        ],
        AgentAction::LinkEntity {
            from, to, relation, ..
        } => vec![
            "+ Link  ◆ AI".into(),
            format!("  {from} ─▶ {relation}  {to}"),
            "  provenance agent (kept forever)".into(),
        ],
        AgentAction::RunCollector { collector, target } => {
            vec![
                "+ Collector run".into(),
                format!("  {collector} on {target}"),
            ]
        }
        AgentAction::GenerateReport { format } => vec![
            "+ Report".into(),
            format!("  {}", format.as_deref().unwrap_or("markdown")),
        ],
        AgentAction::ProposeResolve { entities, reason } => vec![
            "+ Resolve  ◆ AI".into(),
            format!("  {}", entities.join(", ")),
            format!("  {reason}"),
        ],
        AgentAction::ProposeUnresolve { entity, reason } => vec![
            "+ Unresolve  ◆ AI".into(),
            format!("  {entity}"),
            format!("  {reason}"),
        ],
        AgentAction::RunPlaybook { playbook } => {
            vec!["+ Playbook".into(), format!("  {playbook}")]
        }
    }
}

fn rationale_for(action: &AgentAction) -> String {
    match action {
        AgentAction::AddFinding { details, .. } => details.clone(),
        AgentAction::AssertFindingTemplate { details, .. } => details.clone(),
        AgentAction::AddNote { body } => body.clone(),
        AgentAction::ProposeResolve { reason, .. }
        | AgentAction::ProposeUnresolve { reason, .. } => reason.clone(),
        _ => String::new(),
    }
}

fn citations_for(workspace: &Workspace, cited: &[String]) -> Vec<ReviewCitation> {
    cited
        .iter()
        .map(|reference| {
            let present = resolve_source_citation(workspace, reference);
            let label = present
                .and_then(|id| workspace.source(id))
                .map(|source| source.title.clone())
                .unwrap_or_else(|| "missing source".into());
            ReviewCitation {
                reference: reference.clone(),
                present: present.is_some(),
                label,
            }
        })
        .collect()
}

fn resolve_citation(workspace: &Workspace, raw: &str) -> Option<Uuid> {
    if let Ok(reference) = raw.parse::<ShortRef>() {
        return workspace.resolve_ref(reference).or_else(|| {
            workspace
                .data
                .sources
                .iter()
                .find(|source| source.short_ref() == reference)
                .map(|source| source.id)
        });
    }
    workspace
        .data
        .sources
        .iter()
        .find(|source| source.short_ref().to_string() == raw || source.title == raw)
        .map(|source| source.id)
}
