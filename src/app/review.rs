//! Review lane: AI, playbook, and merge proposals wait here until Shift+A then y.

use uuid::Uuid;

use super::{App, Overlay, PendingResolve, View};
use crate::model::{
    AgentAction, Proposal, ProposalStatus, ResolutionSuggestion, ShortRef, Workspace,
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
}

impl ReviewItem {
    pub fn has_missing_citation(&self) -> bool {
        self.citations.iter().any(|citation| !citation.present)
    }

    pub fn is_ai(&self) -> bool {
        matches!(self.kind, ReviewKind::Agent | ReviewKind::Playbook)
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
        self.listed_review_items().len()
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
        if self.view != View::Review && !matches!(self.overlay, Overlay::Proposals { .. }) {
            return;
        }
        if let Overlay::Proposals { armed, .. } = &mut self.overlay {
            *armed = true;
        }
        if self.view == View::Review {
            let items = self.armed_review_items();
            if items.is_empty() {
                self.status = "Nothing to accept.".into();
                return;
            }
            if items.iter().any(ReviewItem::has_missing_citation) {
                self.status =
                    "A citation points at a missing source. Acceptance is blocked.".into();
                self.review_armed = false;
                return;
            }
            self.review_armed = true;
            let ai = items.iter().filter(|item| item.is_ai()).count();
            self.status = format!(
                "Armed — accept {} item(s), {ai} AI. y confirm. Any other key cancels.",
                items.len()
            );
        }
    }

    pub(crate) fn disarm_review(&mut self) {
        if let Overlay::Proposals { armed, .. } = &mut self.overlay {
            *armed = false;
        }
        if self.review_armed {
            self.review_armed = false;
            self.status = "Disarmed. Nothing written.".into();
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
        preview: preview_lines(&proposal.action),
        citations: citations_for(workspace, proposal.action.cited_sources()),
        rationale: rationale_for(&proposal.action),
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

fn preview_lines(action: &AgentAction) -> Vec<String> {
    match action {
        AgentAction::AddFinding {
            title,
            details,
            confidence,
            ..
        } => vec![
            "+ Finding  ◆ AI".into(),
            format!("  {title}"),
            format!("  severity  {}", confidence.as_deref().unwrap_or("medium")),
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
        AgentAction::AssertFindingTemplate { title, details } => vec![
            "+ Finding template  ◆ playbook".into(),
            format!("  {title}"),
            format!("  {}", details.lines().next().unwrap_or_default()),
        ],
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
            let present = resolve_citation(workspace, reference);
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
