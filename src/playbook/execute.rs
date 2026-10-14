//! Run collect / tool / Search Around steps, then apply in one transaction (or dry-run).

use crate::collectors::tool::run_tool;
use crate::collectors::{CollectionOutput, CollectorContext, CollectorRegistry};
use crate::files::sha256_hex;
use crate::jobs::JobContext;
use crate::model::{
    AgentAction, EntityKind, EvidenceBatch, MAX_PLAYBOOK_PROPOSALS, Playbook, PlaybookOutput,
    PlaybookPlan, PlaybookStep, SearchAroundQuery, Target, Workspace, search_around,
    suggest_resolutions,
};
use crate::storage::{ApplyContext, Store};
use crate::tools::{Activity, ToolRegistry};

use super::PlaybookError;
use super::plan::plan_playbook;
use super::watch::change_proposals;

/// How a playbook should run.
#[derive(Clone, Debug, Default)]
pub struct ExecuteOptions {
    pub dry_run: bool,
    pub automation: bool,
    pub previous_digest: Option<String>,
    /// Watch interval stored on the automation row (`os-int watch --interval`).
    pub interval_seconds: Option<i64>,
}

/// Live collect/tool execution, then transactional apply (unless dry-run).
pub async fn execute(
    store: &mut Store,
    playbook: &Playbook,
    seed: &Target,
    apply: ApplyContext,
    collectors: &CollectorRegistry,
    collector_context: &CollectorContext,
    options: ExecuteOptions,
) -> Result<PlaybookOutput, PlaybookError> {
    let gathered = gather(
        playbook,
        seed,
        collectors,
        collector_context,
        options.clone(),
    )
    .await?;
    if gathered.dry_run {
        return Ok(gathered);
    }
    commit(store, playbook, seed, apply, gathered, options)
}

/// Collect and run tools. Does not write SQLite (safe on a job thread).
pub async fn gather(
    playbook: &Playbook,
    seed: &Target,
    collectors: &CollectorRegistry,
    collector_context: &CollectorContext,
    options: ExecuteOptions,
) -> Result<PlaybookOutput, PlaybookError> {
    let tools = collector_context.tools.as_ref();
    let plan = plan_playbook(
        playbook,
        seed,
        tools,
        options.automation,
        &collector_context.job,
    )?;
    if options.dry_run {
        return Ok(dry_run_output(playbook, seed, plan, options.automation));
    }
    let job = &collector_context.job;
    let mut batches = Vec::new();
    let mut warnings = plan.warnings.clone();
    let steps: Vec<&PlaybookStep> = if options.automation {
        playbook
            .steps
            .iter()
            .filter(|step| matches!(step, PlaybookStep::Collect { .. }))
            .collect()
    } else {
        playbook.steps.iter().collect()
    };
    for step in steps {
        if job.is_cancelled() {
            return Err(PlaybookError::new("cancelled"));
        }
        match step {
            PlaybookStep::Collect { collector } => {
                let Some(entry) = collectors.find(collector) else {
                    return Err(PlaybookError::new(format!("unknown collector {collector}")));
                };
                if !entry.accepts(seed.kind()) {
                    warnings.push(format!(
                        "collector {collector} does not accept {}",
                        seed.kind()
                    ));
                    continue;
                }
                job.progress(format!("collect {collector}"));
                match entry.collect(seed, collector_context).await {
                    Ok(output) => take_collection(&mut batches, &mut warnings, output),
                    Err(error) => {
                        return Err(PlaybookError::new(format!(
                            "collector {collector} failed: {error}"
                        )));
                    }
                }
            }
            PlaybookStep::RunTool { tool } => {
                run_one_tool(tool, seed, tools, job, &mut batches, &mut warnings).await?;
            }
            PlaybookStep::SearchAround { .. }
            | PlaybookStep::AssertFindingTemplate { .. }
            | PlaybookStep::ProposeResolve { .. } => {}
        }
    }
    if job.is_cancelled() {
        return Err(PlaybookError::new("cancelled"));
    }
    Ok(PlaybookOutput {
        playbook: playbook.clone(),
        playbook_id: playbook.id.clone(),
        playbook_name: playbook.name.clone(),
        seed_kind: seed.kind().as_str().to_string(),
        seed_value: seed.value(),
        dry_run: false,
        automation: options.automation,
        batches,
        proposals: template_proposals(playbook),
        plan,
        search_summary: None,
        warnings,
        digest: String::new(),
    })
}

/// Apply gathered batches on the UI / CLI thread (one transaction).
pub fn commit(
    store: &mut Store,
    playbook: &Playbook,
    seed: &Target,
    apply: ApplyContext,
    gathered: PlaybookOutput,
    options: ExecuteOptions,
) -> Result<PlaybookOutput, PlaybookError> {
    if gathered.dry_run {
        return Ok(gathered);
    }
    finish(store, playbook, seed, apply, gathered, options)
}

/// Apply already-produced batches (deterministic tests; no network).
pub fn execute_prepared(
    store: &mut Store,
    playbook: &Playbook,
    seed: &Target,
    apply: ApplyContext,
    batches: Vec<EvidenceBatch>,
    tools: &ToolRegistry,
    options: ExecuteOptions,
) -> Result<PlaybookOutput, PlaybookError> {
    let job = JobContext::detached(false);
    let plan = plan_playbook(playbook, seed, tools, options.automation, &job)?;
    if options.dry_run {
        return Ok(dry_run_output(playbook, seed, plan, options.automation));
    }
    let warnings = plan.warnings.clone();
    let gathered = PlaybookOutput {
        playbook: playbook.clone(),
        playbook_id: playbook.id.clone(),
        playbook_name: playbook.name.clone(),
        seed_kind: seed.kind().as_str().to_string(),
        seed_value: seed.value(),
        dry_run: false,
        automation: options.automation,
        batches,
        proposals: Vec::new(),
        plan,
        search_summary: None,
        warnings,
        digest: String::new(),
    };
    finish(store, playbook, seed, apply, gathered, options)
}

fn dry_run_output(
    playbook: &Playbook,
    seed: &Target,
    plan: PlaybookPlan,
    automation: bool,
) -> PlaybookOutput {
    PlaybookOutput {
        playbook: playbook.clone(),
        playbook_id: playbook.id.clone(),
        playbook_name: playbook.name.clone(),
        seed_kind: seed.kind().as_str().to_string(),
        seed_value: seed.value(),
        dry_run: true,
        automation,
        batches: Vec::new(),
        proposals: plan
            .actions
            .iter()
            .filter_map(|action| match action {
                crate::model::PlannedAction::Propose { action } => Some(action.clone()),
                _ => None,
            })
            .collect(),
        plan,
        search_summary: None,
        warnings: Vec::new(),
        digest: String::new(),
    }
}

fn take_collection(
    batches: &mut Vec<EvidenceBatch>,
    warnings: &mut Vec<String>,
    output: CollectionOutput,
) {
    warnings.extend(output.warnings);
    batches.push(output.batch);
}

async fn run_one_tool(
    tool: &str,
    seed: &Target,
    tools: &ToolRegistry,
    job: &JobContext,
    batches: &mut Vec<EvidenceBatch>,
    warnings: &mut Vec<String>,
) -> Result<(), PlaybookError> {
    let Some(entry) = tools.find(tool) else {
        warnings.push(format!("tool {tool} is not registered; skipped"));
        return Ok(());
    };
    if entry.definition.activity == Activity::Active {
        return Err(PlaybookError::new(format!(
            "{} is marked active; playbooks never execute active tools",
            entry.definition.name
        )));
    }
    if !entry.is_available() {
        warnings.push(format!("tool {tool} is not available; skipped"));
        return Ok(());
    }
    let Some(program) = entry.path.clone() else {
        warnings.push(format!("tool {tool} is not on PATH; skipped"));
        return Ok(());
    };
    job.progress(format!("run_tool {tool}"));
    match run_tool(&entry.definition, program, seed, job).await {
        Ok(output) => take_collection(batches, warnings, output),
        Err(error) if error.contains("active") => {
            return Err(PlaybookError::new(error));
        }
        Err(error) if error == "cancelled" => return Err(PlaybookError::new("cancelled")),
        Err(error) => warnings.push(format!("tool {tool}: {error}")),
    }
    Ok(())
}

fn finish(
    store: &mut Store,
    playbook: &Playbook,
    seed: &Target,
    apply: ApplyContext,
    gathered: PlaybookOutput,
    options: ExecuteOptions,
) -> Result<PlaybookOutput, PlaybookError> {
    let PlaybookOutput {
        batches,
        mut warnings,
        plan,
        ..
    } = gathered;
    let digest = batch_digest(&batches);
    let apply_batches = if options.automation {
        batches
            .iter()
            .map(|batch| {
                let mut clone = batch.clone();
                clone.findings.clear();
                clone.resolved_findings.clear();
                clone
            })
            .collect()
    } else {
        batches.clone()
    };
    let mut proposals = if options.automation {
        Vec::new()
    } else {
        template_proposals(playbook)
    };
    let outcome = store
        .apply_playbook_outcome(
            apply,
            &apply_batches,
            crate::storage::PlaybookApplyMeta {
                playbook,
                seed,
                automation: options.automation,
                digest: &digest,
                previous_digest: options.previous_digest.as_deref(),
            },
        )
        .map_err(|error| PlaybookError::new(error.to_string()))?;
    if options.automation {
        let digest_changed = outcome.digest_changed;
        proposals = change_proposals(playbook, &outcome.summary, &batches, digest_changed);
    } else {
        run_search_and_resolve(
            store,
            apply.investigation_id,
            playbook,
            seed,
            &mut warnings,
            &mut proposals,
        );
    }
    proposals.truncate(MAX_PLAYBOOK_PROPOSALS);
    store
        .insert_playbook_proposals(apply.investigation_id, playbook, seed, &proposals)
        .map_err(|error| PlaybookError::new(error.to_string()))?;
    if options.automation {
        store
            .upsert_automation(
                apply.investigation_id,
                &playbook.id,
                seed,
                options.interval_seconds,
                Some(&digest),
            )
            .map_err(|error| PlaybookError::new(error.to_string()))?;
    }
    let output = PlaybookOutput {
        playbook: playbook.clone(),
        playbook_id: playbook.id.clone(),
        playbook_name: playbook.name.clone(),
        seed_kind: seed.kind().as_str().to_string(),
        seed_value: seed.value(),
        dry_run: false,
        automation: options.automation,
        batches,
        proposals,
        plan,
        search_summary: warnings
            .iter()
            .find(|line| line.starts_with("search_around"))
            .cloned(),
        warnings,
        digest,
    };
    Ok(output)
}

fn template_proposals(playbook: &Playbook) -> Vec<AgentAction> {
    playbook
        .steps
        .iter()
        .filter_map(|step| match step {
            PlaybookStep::AssertFindingTemplate { title, details } => {
                Some(AgentAction::AssertFindingTemplate {
                    title: title.clone(),
                    details: details.clone(),
                })
            }
            _ => None,
        })
        .take(MAX_PLAYBOOK_PROPOSALS)
        .collect()
}

fn run_search_and_resolve(
    store: &Store,
    investigation_id: uuid::Uuid,
    playbook: &Playbook,
    seed: &Target,
    warnings: &mut Vec<String>,
    proposals: &mut Vec<AgentAction>,
) {
    if let Some(search) = search_step(playbook) {
        match run_search(store, investigation_id, seed, search) {
            Ok(summary) => warnings.push(summary),
            Err(error) => warnings.push(error),
        }
    }
    if !playbook
        .steps
        .iter()
        .any(|step| matches!(step, PlaybookStep::ProposeResolve { .. }))
    {
        return;
    }
    match resolve_proposals(store, investigation_id, playbook) {
        Ok(found) => {
            for action in found {
                if proposals.len() >= MAX_PLAYBOOK_PROPOSALS {
                    break;
                }
                proposals.push(action);
            }
        }
        Err(error) => warnings.push(error),
    }
}

fn search_step(playbook: &Playbook) -> Option<&PlaybookStep> {
    playbook
        .steps
        .iter()
        .find(|step| matches!(step, PlaybookStep::SearchAround { .. }))
}

fn run_search(
    store: &Store,
    investigation_id: uuid::Uuid,
    seed: &Target,
    step: &PlaybookStep,
) -> Result<String, String> {
    let PlaybookStep::SearchAround {
        hops,
        link_types,
        entity_kinds,
    } = step
    else {
        return Ok(String::new());
    };
    let data = store
        .load_investigation(investigation_id)
        .map_err(|error| error.to_string())?;
    let workspace = Workspace::new(data);
    let Some(entity) = workspace.entity_by_any_value(&seed.value()) else {
        return Ok(format!(
            "search_around: seed {} not in the graph yet",
            seed.value()
        ));
    };
    let mut query = SearchAroundQuery::new(vec![entity.id], *hops);
    query.link_types = link_types.clone();
    query.entity_kinds = entity_kinds
        .iter()
        .map(|kind| EntityKind::parse(kind))
        .collect();
    let result = search_around(&workspace.data, &query);
    Ok(format!(
        "search_around {} hop(s): {} node(s), {} edge(s)",
        result.hops_used,
        result.nodes.len(),
        result.edges.len()
    ))
}

fn resolve_proposals(
    store: &Store,
    investigation_id: uuid::Uuid,
    playbook: &Playbook,
) -> Result<Vec<AgentAction>, String> {
    let reason = playbook
        .steps
        .iter()
        .find_map(|step| match step {
            PlaybookStep::ProposeResolve { reason } => Some(reason.as_str()),
            _ => None,
        })
        .unwrap_or("");
    let data = store
        .load_investigation(investigation_id)
        .map_err(|error| error.to_string())?;
    let suggestions = suggest_resolutions(&data);
    let mut actions = Vec::new();
    for suggestion in suggestions.into_iter().take(4) {
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
        if refs.len() < 2 {
            continue;
        }
        actions.push(AgentAction::ProposeResolve {
            entities: refs,
            reason: if reason.is_empty() {
                suggestion.reason
            } else {
                reason.to_string()
            },
        });
    }
    Ok(actions)
}

fn batch_digest(batches: &[EvidenceBatch]) -> String {
    let mut material = String::new();
    for batch in batches {
        for source in &batch.sources {
            material.push_str(source.content_hash.as_deref().unwrap_or(""));
            material.push('\n');
            material.push_str(&source.title);
            material.push('\n');
        }
    }
    sha256_hex(material.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::model::{
        Confidence, EntityKey, NewEntity, NewRelationship, NewSource, Provenance, SourceKind,
        relation,
    };
    use crate::playbook::{bundled, parse_playbook_yaml};
    use crate::storage::Store;
    use crate::tools::ToolRegistry;

    fn fixture_batch() -> EvidenceBatch {
        let mut batch = EvidenceBatch::new();
        batch.sources.push({
            let mut source = NewSource::collected(SourceKind::Dns, "dns", "DNS example.com");
            source.content = "A 192.0.2.1".into();
            source.content_hash = Some("abc123".into());
            source
        });
        let domain = EntityKey::new(EntityKind::Domain, "example.com");
        let host = EntityKey::new(EntityKind::Hostname, "www.example.com");
        let ip = EntityKey::new(EntityKind::Ip, "192.0.2.1");
        batch.entities.push(NewEntity {
            key: domain.clone(),
            label: None,
            fields: crate::model::Fields::new(),
            sources: vec![0],
            link_to_page: true,
        });
        batch.entities.push(NewEntity {
            key: host.clone(),
            label: None,
            fields: crate::model::Fields::new(),
            sources: vec![0],
            link_to_page: false,
        });
        batch.entities.push(NewEntity {
            key: ip.clone(),
            label: None,
            fields: crate::model::Fields::new(),
            sources: vec![0],
            link_to_page: false,
        });
        batch.relationships.push(NewRelationship {
            from: host,
            kind: relation::SUBDOMAIN_OF.into(),
            to: domain.clone(),
            sources: vec![0],
            confidence: Confidence::High,
        });
        batch.relationships.push(NewRelationship {
            from: domain,
            kind: relation::RESOLVES_TO.into(),
            to: ip,
            sources: vec![0],
            confidence: Confidence::High,
        });
        batch.summary = "fixture domain infra".into();
        batch
    }

    #[test]
    fn domain_infra_on_fixture_leaves_cited_graph_and_pending_proposals() {
        let mut store = Store::open_in_memory().expect("store");
        let investigation = store
            .create_investigation("Fixture", "")
            .expect("investigation");
        let page = store
            .create_page(crate::storage::NewPage::new(
                investigation.id,
                "example.com",
                crate::model::PageKind::Domain,
                Some("example.com"),
            ))
            .expect("page");
        let playbook = bundled("domain-infra").expect("playbook");
        let seed = Target::parse(crate::model::TargetKind::Domain, "example.com").expect("seed");
        let tools = ToolRegistry::detect(&Config::default().tools);
        let output = execute_prepared(
            &mut store,
            &playbook,
            &seed,
            ApplyContext {
                investigation_id: investigation.id,
                page_id: Some(page.id),
                job_id: None,
            },
            vec![fixture_batch()],
            &tools,
            ExecuteOptions::default(),
        )
        .expect("execute");
        assert!(!output.dry_run);
        let data = store.load_investigation(investigation.id).expect("load");
        assert!(
            data.entities
                .iter()
                .any(|entity| entity.value == "example.com")
        );
        assert!(data.relationships.iter().any(|relationship| {
            relationship.kind == relation::SUBDOMAIN_OF
                || relationship.kind == relation::RESOLVES_TO
        }));
        assert!(
            data.sources
                .iter()
                .any(|source| source.provenance == Provenance::Collected)
        );
        let pending: Vec<_> = data
            .proposals
            .iter()
            .filter(|proposal| proposal.status == crate::model::ProposalStatus::Pending)
            .collect();
        assert!(!pending.is_empty(), "templates become pending proposals");
        assert!(
            pending
                .iter()
                .any(|proposal| proposal.action.name() == "ASSERT_FINDING_TEMPLATE")
        );
        assert!(
            !data
                .findings
                .iter()
                .any(|finding| finding.title == "Review domain infrastructure"),
            "finding templates must not write until accept"
        );
        assert!(data.resolutions.is_empty(), "never auto-merge");
        assert_eq!(
            data.proposals
                .iter()
                .filter(|proposal| proposal.status == crate::model::ProposalStatus::Accepted)
                .count(),
            0
        );
    }

    #[test]
    fn invalid_yaml_does_not_touch_the_database() {
        let store = Store::open_in_memory().expect("store");
        let investigation = store.create_investigation("Clean", "").expect("inv");
        let before = store.load_investigation(investigation.id).expect("before");
        let actual = parse_playbook_yaml(b"not: [yaml: :").expect_err("parse");
        assert!(actual.message.contains("invalid playbook YAML"));
        let after = store.load_investigation(investigation.id).expect("after");
        assert_eq!(before.entities.len(), after.entities.len());
        assert_eq!(before.proposals.len(), after.proposals.len());
        assert_eq!(before.sources.len(), after.sources.len());
    }

    #[test]
    fn dry_run_applies_nothing() {
        let mut store = Store::open_in_memory().expect("store");
        let investigation = store.create_investigation("Dry", "").expect("inv");
        let playbook = bundled("domain-infra").expect("playbook");
        let seed = Target::parse(crate::model::TargetKind::Domain, "example.com").expect("seed");
        let tools = ToolRegistry::detect(&Config::default().tools);
        let output = execute_prepared(
            &mut store,
            &playbook,
            &seed,
            ApplyContext {
                investigation_id: investigation.id,
                page_id: None,
                job_id: None,
            },
            vec![fixture_batch()],
            &tools,
            ExecuteOptions {
                dry_run: true,
                automation: false,
                previous_digest: None,
                interval_seconds: None,
            },
        )
        .expect("dry");
        assert!(output.dry_run);
        assert!(!output.plan.actions.is_empty());
        let data = store.load_investigation(investigation.id).expect("load");
        assert!(data.entities.is_empty());
        assert!(data.proposals.is_empty());
        assert!(data.sources.is_empty());
    }

    #[test]
    fn automation_dry_run_lists_planned_actions_without_applying() {
        let mut store = Store::open_in_memory().expect("store");
        let investigation = store.create_investigation("Watch", "").expect("inv");
        let playbook = bundled("domain-infra").expect("playbook");
        let seed = Target::parse(crate::model::TargetKind::Domain, "example.com").expect("seed");
        let tools = ToolRegistry::detect(&Config::default().tools);
        let output = execute_prepared(
            &mut store,
            &playbook,
            &seed,
            ApplyContext {
                investigation_id: investigation.id,
                page_id: None,
                job_id: None,
            },
            vec![fixture_batch()],
            &tools,
            ExecuteOptions {
                dry_run: true,
                automation: true,
                previous_digest: None,
                interval_seconds: None,
            },
        )
        .expect("watch dry");
        let described = output.plan.describe();
        assert!(described.contains("automation dry-run"));
        assert!(described.contains("collect dns"));
        assert!(described.contains("Nothing was applied"));
        let data = store.load_investigation(investigation.id).expect("load");
        assert!(data.entities.is_empty());
        assert!(data.proposals.is_empty());
    }

    #[test]
    fn automation_does_not_silently_write_findings() {
        let mut store = Store::open_in_memory().expect("store");
        let investigation = store.create_investigation("Watch apply", "").expect("inv");
        let playbook = bundled("domain-infra").expect("playbook");
        let seed = Target::parse(crate::model::TargetKind::Domain, "example.com").expect("seed");
        let tools = ToolRegistry::detect(&Config::default().tools);
        let mut batches = vec![fixture_batch()];
        batches[0].findings.push(crate::model::NewCollectedFinding {
            title: "Silent write".into(),
            details: "must stay a proposal".into(),
            confidence: Confidence::Medium,
            source: Some(0),
            provenance: Provenance::Collected,
        });
        let output = execute_prepared(
            &mut store,
            &playbook,
            &seed,
            ApplyContext {
                investigation_id: investigation.id,
                page_id: None,
                job_id: None,
            },
            batches,
            &tools,
            ExecuteOptions {
                dry_run: false,
                automation: true,
                previous_digest: None,
                interval_seconds: Some(3600),
            },
        )
        .expect("watch apply");
        assert!(output.automation);
        let data = store.load_investigation(investigation.id).expect("load");
        assert!(
            data.entities
                .iter()
                .any(|entity| entity.value == "example.com")
        );
        assert!(
            !data
                .findings
                .iter()
                .any(|finding| finding.title == "Silent write"
                    || finding.title == "Review domain infrastructure"),
            "watch must not write findings until accept"
        );
        assert!(data.proposals.iter().any(|proposal| {
            proposal.status == crate::model::ProposalStatus::Pending
                && proposal.action.name() == "ASSERT_FINDING_TEMPLATE"
        }));
        let automation = store
            .find_automation(investigation.id, "domain-infra", "example.com")
            .expect("lookup")
            .expect("row");
        assert_eq!(automation.interval_seconds, Some(3600));
        assert!(automation.enabled);
    }

    #[test]
    fn untrusted_playbook_cannot_clear_collected_findings() {
        let input = br#"
format: os-int-playbook
format_version: 1
id: wipe-findings
seed:
  type: domain
steps:
  - collect: dns
resolved_findings:
  - Apex domain is live
"#;
        let actual = crate::playbook::parse_playbook_yaml(input).expect_err("resolved");
        assert!(actual.message.contains("resolved_findings"));
    }

    #[test]
    fn active_tool_is_refused_at_plan_time() {
        let yaml = br#"
format: os-int-playbook
format_version: 1
id: nmap-no
seed:
  type: domain
steps:
  - run_tool: nmap
"#;
        let playbook = parse_playbook_yaml(yaml).expect("parses");
        let mut config = Config::default();
        config
            .tools
            .custom
            .push(crate::config::ToolDefinitionConfig {
                name: "nmap".into(),
                program: "true".into(),
                args: vec!["{target}".into()],
                accepts: vec!["domain".into()],
                description: "probe".into(),
                timeout_seconds: 5,
                parser: "text".into(),
                activity: "active".into(),
            });
        let tools = ToolRegistry::detect(&config.tools);
        let seed = Target::parse(crate::model::TargetKind::Domain, "example.com").expect("seed");
        let job = JobContext::detached(false);
        let actual = plan_playbook(&playbook, &seed, &tools, false, &job).expect_err("active");
        assert!(actual.message.contains("active"));
    }
}
