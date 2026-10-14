//! Automation helpers: collect-only plans and change proposals.

use crate::model::{
    AgentAction, EvidenceBatch, PlannedAction, Playbook, PlaybookPlan, PlaybookStep, Target,
};
use crate::storage::ApplySummary;
use crate::tools::ToolRegistry;

use super::PlaybookError;
use super::plan::plan_playbook;
use crate::jobs::JobContext;

/// Dry-run for a watch set: collect steps plus a change-proposal placeholder.
pub fn automation_plan(
    playbook: &Playbook,
    seed: &Target,
    tools: &ToolRegistry,
    job: &JobContext,
) -> Result<PlaybookPlan, PlaybookError> {
    let mut plan = plan_playbook(playbook, seed, tools, true, job)?;
    plan.actions.retain(|action| {
        matches!(
            action,
            PlannedAction::Collect { .. } | PlannedAction::Propose { .. }
        )
    });
    Ok(plan)
}

/// Pending Actions opened when a watch pass sees observation changes.
/// Collector diagnostic findings are not written on this path; they become proposals.
pub fn change_proposals(
    playbook: &Playbook,
    summary: &ApplySummary,
    batches: &[EvidenceBatch],
    digest_changed: bool,
) -> Vec<AgentAction> {
    if !digest_changed && summary.changed_observations == 0 {
        return Vec::new();
    }
    let mut actions = Vec::new();
    actions.push(AgentAction::AddFinding {
        title: format!("Observation changed during watch of {}", playbook.id),
        details: format!(
            "{} changed observation(s). Accept to record this as an analyst finding.",
            summary.changed_observations
        ),
        confidence: Some("medium".into()),
        sources: Vec::new(),
    });
    for batch in batches {
        for finding in &batch.findings {
            actions.push(AgentAction::AssertFindingTemplate {
                title: finding.title.clone(),
                details: finding.details.clone(),
            });
        }
    }
    for step in &playbook.steps {
        if let PlaybookStep::AssertFindingTemplate { title, details } = step {
            actions.push(AgentAction::AssertFindingTemplate {
                title: title.clone(),
                details: details.clone(),
            });
        }
    }
    actions.truncate(crate::model::MAX_PLAYBOOK_PROPOSALS);
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playbook::bundled;
    use crate::storage::ApplySummary;

    #[test]
    fn unchanged_digest_and_observations_propose_nothing() {
        let playbook = bundled("domain-infra").expect("bundled");
        let actual = change_proposals(&playbook, &ApplySummary::default(), &[], false);
        assert!(actual.is_empty());
    }
}
