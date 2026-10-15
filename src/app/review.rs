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
    /// Proposal short-ref (`X-n`) when this row is a pending Action.
    pub short_ref: Option<String>,
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

    /// Exact `X-n` short-ref or full UUID. Never substring-matches (`X-1` ≠ `X-10`).
    pub fn matches_reference(&self, raw: &str) -> bool {
        let needle = raw.trim();
        if needle.is_empty() {
            return false;
        }
        if self.short_ref.as_deref() == Some(needle) {
            return true;
        }
        self.proposal_id
            .is_some_and(|id| id.to_string().eq_ignore_ascii_case(needle))
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
            items.push(review_item_from_proposal(
                workspace,
                proposal,
                self.config.ui.ascii,
            ));
        }
        for pending in &self.pending_merges {
            items.push(review_item_from_merge(
                workspace,
                pending,
                "manual merge",
                self.config.ui.ascii,
            ));
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
            items.push(review_item_from_suggestion(
                workspace,
                &suggestion,
                self.config.ui.ascii,
            ));
        }
        for run in &workspace.data.agent_runs {
            if items
                .iter()
                .any(|item| item.run_id == Some(run.id) && item.kind != ReviewKind::History)
            {
                continue;
            }
            items.push(review_item_from_run(run, self.config.ui.ascii));
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
        self.status = confirm_status(&items, self.config.ui.ascii);
        self.motion.arm_confirm();
    }

    pub(crate) fn disarm_review(&mut self) {
        if self.review_armed {
            self.review_armed = false;
            self.status = "Disarmed. Nothing written.".into();
            self.motion.disarm_confirm();
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
        self.status = format!(
            "Agent run {reference} {} transcript in Inspector.",
            crate::tui::chrome_dash(self.config.ui.ascii)
        );
    }

    pub fn confirm_accept_text(&self) -> String {
        confirm_status(&self.armed_review_items(), self.config.ui.ascii)
    }

    pub fn confirm_bar_text(&self) -> String {
        confirm_bar(&self.armed_review_items(), self.config.ui.ascii)
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

fn review_item_from_proposal(
    workspace: &Workspace,
    proposal: &Proposal,
    ascii: bool,
) -> ReviewItem {
    let run = workspace.agent_run(proposal.agent_run_id);
    let playbook = run
        .map(|run| run.agent_name == crate::storage::PLAYBOOK_AGENT_NAME)
        .unwrap_or(false);
    let kind = if playbook {
        ReviewKind::Playbook
    } else {
        ReviewKind::Agent
    };
    let sep = if ascii { "-" } else { "·" };
    let group = run
        .map(|run| {
            format!(
                "{} {} {sep} {}",
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
        short_ref: Some(proposal.short_ref().to_string()),
        title: proposal.action.summary_with(ascii),
        group,
        preview: preview_lines(&proposal.action, kind, ascii),
        citations: citations_for(workspace, proposal.action.cited_sources()),
        rationale: rationale_for(&proposal.action),
        written_as: Some(written_provenance(kind, &proposal.action)),
    }
}

fn review_item_from_merge(
    workspace: &Workspace,
    pending: &PendingResolve,
    rule: &str,
    ascii: bool,
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
    let approx = if ascii { "~" } else { "≈" };
    ReviewItem {
        kind: ReviewKind::Merge,
        proposal_id: None,
        run_id: None,
        merge: Some(pending.clone()),
        short_ref: None,
        title: format!("Merge {}", labels.join(" + ")),
        group: format!("{approx} {rule}"),
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
    ascii: bool,
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
        ascii,
    );
    item.title = format!("{}  score {}", item.title, suggestion.score);
    item
}

fn review_item_from_run(run: &crate::model::AgentRun, ascii: bool) -> ReviewItem {
    let sep = if ascii { "-" } else { "·" };
    ReviewItem {
        kind: ReviewKind::History,
        proposal_id: None,
        run_id: Some(run.id),
        merge: None,
        short_ref: None,
        title: run.question.clone(),
        group: format!("{} {sep} {}", run.short_ref(), run.agent_name),
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

fn ai_mark(ascii: bool) -> &'static str {
    if ascii { "*" } else { "◆" }
}

fn confirm_status(items: &[ReviewItem], ascii: bool) -> String {
    let mark = ai_mark(ascii);
    let ai = items.iter().filter(|item| item.is_ai()).count();
    let analyst = items
        .iter()
        .filter(|item| item.written_as == Some(Provenance::Analyst))
        .count();
    match (items.len(), ai, analyst) {
        (1, 1, _) => {
            format!(
                "Armed {} accept 1 AI item. It keeps {mark} agent provenance. y confirm. Any other key cancels.",
                crate::tui::chrome_dash(ascii)
            )
        }
        (1, _, 1) => {
            format!(
                "Armed {} accept 1 analyst item. It keeps analyst provenance. y confirm. Any other key cancels.",
                crate::tui::chrome_dash(ascii)
            )
        }
        (count, ai, 0) if ai == count => format!(
            "Armed {} accept {count} AI item(s). They keep {mark} agent provenance. y confirm. Any other key cancels.",
            crate::tui::chrome_dash(ascii)
        ),
        (count, 0, analyst) if analyst == count => format!(
            "Armed {} accept {count} analyst item(s). They keep analyst provenance. y confirm. Any other key cancels.",
            crate::tui::chrome_dash(ascii)
        ),
        (count, ai, analyst) => format!(
            "Armed {} accept {count} item(s): {ai} AI keep {mark}, {analyst} analyst. y confirm. Any other key cancels.",
            crate::tui::chrome_dash(ascii)
        ),
    }
}

fn confirm_bar(items: &[ReviewItem], ascii: bool) -> String {
    let mark = ai_mark(ascii);
    let ai = items.iter().filter(|item| item.is_ai()).count();
    let analyst = items
        .iter()
        .filter(|item| item.written_as == Some(Provenance::Analyst))
        .count();
    match (items.len(), ai, analyst) {
        (1, 1, _) => format!(
            " Accept 1 AI item? It keeps {mark} agent provenance.   y confirm   any other key cancels "
        ),
        (1, _, 1) => {
            " Accept 1 analyst item? It keeps analyst provenance.   y confirm   any other key cancels "
                .into()
        }
        (count, ai, 0) if ai == count => format!(
            " Accept {count} AI item(s)? They keep {mark} agent provenance.   y confirm   any other key cancels "
        ),
        (count, 0, analyst) if analyst == count => format!(
            " Accept {count} analyst item(s)? They keep analyst provenance.   y confirm   any other key cancels "
        ),
        (count, ai, analyst) => format!(
            " Accept {count} item(s): {ai} AI keep {mark}, {analyst} analyst.   y confirm   any other key cancels "
        ),
    }
}

/// Resolve a cited source the same way Review preview and apply do.
pub fn resolve_source_citation(workspace: &Workspace, raw: &str) -> Option<Uuid> {
    resolve_citation(workspace, raw)
}

fn preview_lines(action: &AgentAction, kind: ReviewKind, ascii: bool) -> Vec<String> {
    let mark = ai_mark(ascii);
    let arrow = if ascii { "->" } else { "─▶" };
    let analyst = if ascii { "A" } else { "✎" };
    match action {
        AgentAction::AddFinding {
            title,
            details,
            confidence,
            ..
        } => vec![
            format!("+ Finding  {mark} AI"),
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
                (analyst, "analyst (playbook template)")
            } else {
                (mark, "agent (kept forever)")
            };
            vec![
                format!("+ Finding template  {glyph} {label}"),
                format!("  {title}"),
                format!("  {}", details.lines().next().unwrap_or_default()),
            ]
        }
        AgentAction::AddNote { body } => vec![
            format!("+ Note  {mark} AI"),
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
            format!("+ Link  {mark} AI"),
            format!("  {from} {arrow} {relation}  {to}"),
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
            format!("+ Resolve  {mark} AI"),
            format!("  {}", entities.join(", ")),
            format!("  {reason}"),
        ],
        AgentAction::ProposeUnresolve { entity, reason } => vec![
            format!("+ Unresolve  {mark} AI"),
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

#[cfg(test)]
mod tests {
    use super::{ReviewItem, ReviewKind};

    fn item(short_ref: &str, id: uuid::Uuid) -> ReviewItem {
        ReviewItem {
            kind: ReviewKind::Agent,
            proposal_id: Some(id),
            run_id: None,
            merge: None,
            short_ref: Some(short_ref.into()),
            title: "finding".into(),
            group: format!("{short_ref} Q-1 · agent"),
            preview: Vec::new(),
            citations: Vec::new(),
            rationale: String::new(),
            written_as: Some(crate::model::Provenance::Agent),
        }
    }

    #[test]
    fn review_ref_x1_does_not_select_x10() {
        let ten = item("X-10", uuid::Uuid::new_v4());
        let one = item("X-1", uuid::Uuid::new_v4());
        assert!(one.matches_reference("X-1"));
        assert!(!one.matches_reference("X-10"));
        assert!(ten.matches_reference("X-10"));
        assert!(
            !ten.matches_reference("X-1"),
            "X-1 must not select X-10 via contains"
        );
        assert!(!ten.matches_reference("X"));
        assert!(one.matches_reference(&one.proposal_id.unwrap().to_string()));
    }

    #[test]
    fn ascii_confirm_copy_has_no_em_dash_or_middle_dot() {
        let one = item("X-1", uuid::Uuid::new_v4());
        let status = super::confirm_status(std::slice::from_ref(&one), true);
        let bar = super::confirm_bar(&[one], true);
        for text in [&status, &bar] {
            assert!(!text.contains('—'), "{text}");
            assert!(!text.contains('·'), "{text}");
            assert!(!text.contains('◆'), "{text}");
        }
        assert!(status.contains("Armed -"), "{status}");
        assert!(bar.contains('*'), "{bar}");
    }

    #[test]
    fn ascii_merge_group_does_not_use_almost_equal() {
        let pending = crate::app::PendingResolve {
            entity_ids: vec![uuid::Uuid::new_v4(), uuid::Uuid::new_v4()],
            canonical_id: uuid::Uuid::new_v4(),
            reason: "same value".into(),
        };
        let timestamp = crate::model::now();
        let workspace = crate::model::Workspace::new(crate::model::InvestigationData::empty(
            crate::model::Investigation {
                id: uuid::Uuid::new_v4(),
                name: "case".into(),
                description: String::new(),
                tags: Vec::new(),
                marking: crate::model::Marking::Unmarked,
                created_at: timestamp,
                updated_at: timestamp,
            },
        ));
        let item = super::review_item_from_merge(&workspace, &pending, "same_value", true);
        assert!(!item.group.contains('≈'), "{}", item.group);
        assert!(item.group.starts_with("~ "), "{}", item.group);
        let unicode = super::review_item_from_merge(&workspace, &pending, "same_value", false);
        assert!(unicode.group.starts_with("≈ "), "{}", unicode.group);
    }
}
