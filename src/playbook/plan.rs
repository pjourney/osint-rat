//! Dry-run planning: list Actions without applying them.

use crate::jobs::JobContext;
use crate::model::{
    AgentAction, InvestigationData, MAX_PLAYBOOK_PROPOSALS, PlannedAction, Playbook, PlaybookPlan,
    PlaybookStep, Target, suggest_resolutions,
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
        warnings
            .push("watch proposes a finding only when collect steps see a new content hash".into());
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

/// Fill dry-run `propose_resolve` rows with live entity ids when the graph already has them.
pub fn bind_resolve_plan(plan: &mut PlaybookPlan, data: &InvestigationData, playbook: &Playbook) {
    let reason = playbook
        .steps
        .iter()
        .find_map(|step| match step {
            PlaybookStep::ProposeResolve { reason } => Some(reason.as_str()),
            _ => None,
        })
        .unwrap_or("");
    let suggestions = suggest_resolutions(data);
    for planned in &mut plan.actions {
        let PlannedAction::Propose {
            action:
                AgentAction::ProposeResolve {
                    entities,
                    reason: planned_reason,
                },
        } = planned
        else {
            continue;
        };
        if !entities.is_empty() {
            continue;
        }
        if let Some(suggestion) = suggestions.first() {
            let refs: Vec<String> = suggestion
                .entity_ids
                .iter()
                .filter_map(|id| {
                    data.entities
                        .iter()
                        .find(|entity| entity.id == *id)
                        .map(|entity| entity.short_ref().to_string())
                })
                .collect();
            if refs.len() >= 2 {
                *entities = refs;
                if planned_reason.is_empty() && !reason.is_empty() {
                    *planned_reason = reason.to_string();
                }
            }
        }
    }
}
