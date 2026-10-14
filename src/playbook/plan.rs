//! Dry-run planning: list Actions without applying them.

use crate::jobs::JobContext;
use crate::model::{
    AgentAction, MAX_PLAYBOOK_PROPOSALS, PlannedAction, Playbook, PlaybookPlan, PlaybookStep,
    Target,
};
use crate::tools::{Activity, ToolRegistry};

use super::PlaybookError;

/// Build a dry-run plan. Missing optional tools become warnings; active tools are errors.
pub fn plan_playbook(
    playbook: &Playbook,
    seed: &Target,
    tools: &ToolRegistry,
    automation: bool,
    job: &JobContext,
) -> Result<PlaybookPlan, PlaybookError> {
    if seed.kind() != playbook.seed.kind {
        return Err(PlaybookError::new(format!(
            "playbook {} seeds a {}, not a {}",
            playbook.id,
            playbook.seed.kind,
            seed.kind()
        )));
    }
    if job.is_cancelled() {
        return Err(PlaybookError::new("cancelled"));
    }
    let mut actions = Vec::new();
    let mut warnings = Vec::new();
    let mut proposal_count = 0usize;
    for step in &playbook.steps {
        if job.is_cancelled() {
            return Err(PlaybookError::new("cancelled"));
        }
        match step {
            PlaybookStep::Collect { collector } => {
                actions.push(PlannedAction::Collect {
                    collector: collector.clone(),
                });
            }
            PlaybookStep::RunTool { tool } => {
                if automation {
                    warnings.push(format!("watch skips run_tool {tool} (collect steps only)"));
                    continue;
                }
                match tools.find(tool) {
                    None => warnings.push(format!(
                        "tool {tool} is not registered; run_tool step would be skipped"
                    )),
                    Some(entry) if entry.definition.activity == Activity::Active => {
                        return Err(PlaybookError::new(format!(
                            "{} is marked active; playbooks never execute active tools",
                            entry.definition.name
                        )));
                    }
                    Some(entry)
                        if entry
                            .problem
                            .as_deref()
                            .is_some_and(|problem| problem.contains("active")) =>
                    {
                        return Err(PlaybookError::new(format!(
                            "{} is marked active; playbooks never execute active tools",
                            entry.definition.name
                        )));
                    }
                    Some(entry) if !entry.is_available() => {
                        warnings.push(format!(
                            "tool {tool} is not available ({}); run_tool step would be skipped",
                            entry.problem.as_deref().unwrap_or("not on PATH")
                        ));
                    }
                    Some(_) => actions.push(PlannedAction::RunTool { tool: tool.clone() }),
                }
            }
            PlaybookStep::SearchAround { hops, .. } => {
                if automation {
                    warnings.push("watch skips search_around (collect steps only)".into());
                    continue;
                }
                actions.push(PlannedAction::SearchAround { hops: *hops });
            }
            PlaybookStep::AssertFindingTemplate { title, details } => {
                proposal_count += 1;
                if proposal_count > MAX_PLAYBOOK_PROPOSALS {
                    return Err(PlaybookError::new(format!(
                        "playbook would open more than {MAX_PLAYBOOK_PROPOSALS} proposals"
                    )));
                }
                actions.push(PlannedAction::Propose {
                    action: AgentAction::AssertFindingTemplate {
                        title: title.clone(),
                        details: details.clone(),
                    },
                });
            }
            PlaybookStep::ProposeResolve { reason } => {
                if automation {
                    warnings.push(
                        "watch does not auto-merge; propose_resolve stays a later analyst Action"
                            .into(),
                    );
                    continue;
                }
                proposal_count += 1;
                if proposal_count > MAX_PLAYBOOK_PROPOSALS {
                    return Err(PlaybookError::new(format!(
                        "playbook would open more than {MAX_PLAYBOOK_PROPOSALS} proposals"
                    )));
                }
                actions.push(PlannedAction::Propose {
                    action: AgentAction::ProposeResolve {
                        entities: Vec::new(),
                        reason: reason.clone(),
                    },
                });
            }
        }
    }
    if automation {
        actions.push(PlannedAction::Propose {
            action: AgentAction::AddFinding {
                title: format!("Watch {} for observation changes", playbook.id),
                details: "Opened only when collect steps see a new content hash. Not applied until accept.".into(),
                confidence: Some("medium".into()),
                sources: Vec::new(),
            },
        });
    }
    Ok(PlaybookPlan {
        playbook_id: playbook.id.clone(),
        playbook_name: playbook.name.clone(),
        seed_kind: seed.kind().as_str().to_string(),
        seed_value: seed.value(),
        automation,
        actions,
        warnings,
    })
}
