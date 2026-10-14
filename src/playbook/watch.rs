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
    digest: &str,
) -> Vec<AgentAction> {
    if !digest_changed && summary.changed_observations == 0 {
        return Vec::new();
    }
    let mut actions = Vec::new();
    actions.push(AgentAction::AddFinding {
        title: format!("Observation changed during watch of {}", playbook.id),
        details: format!(
            "{} changed observation(s). digest={digest}. Accept to record this as an analyst finding.",
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
        let actual = change_proposals(&playbook, &ApplySummary::default(), &[], false, "abc");
        assert!(actual.is_empty());
    }

    #[test]
    fn distinct_change_digests_with_the_same_count_open_new_notices() {
        let playbook = bundled("domain-infra").expect("bundled");
        let summary = ApplySummary {
            changed_observations: 1,
            ..ApplySummary::default()
        };
        let first = change_proposals(&playbook, &summary, &[], true, "digest-one");
        let second = change_proposals(&playbook, &summary, &[], true, "digest-two");
        let first_notice = first
            .iter()
            .find(|action| matches!(action, AgentAction::AddFinding { .. }))
            .expect("AddFinding");
        let second_notice = second
            .iter()
            .find(|action| matches!(action, AgentAction::AddFinding { .. }))
            .expect("AddFinding");
        assert_ne!(
            first_notice.identity_key(),
            second_notice.identity_key(),
            "a distinct change with the same observation count must not dedupe"
        );
        assert!(
            first_notice.identity_key().1.contains("digest=digest-one"),
            "identity must include the change digest"
        );
    }
}
