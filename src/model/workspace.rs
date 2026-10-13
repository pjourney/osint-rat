//! An investigation loaded into memory, with lookup indexes.
//!
//! [`InvestigationData`] is also the `data` object inside an export manifest. [`Workspace`]
//! wraps that snapshot with maps from UUID and short reference back to each record, so the
//! workbench and the report renderer do not scan the vectors for every lookup.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::agent::{AgentRun, Proposal, ProposalStatus};
use super::artifact::Artifact;
use super::entity::{Entity, EntityKind};
use super::finding::Finding;
use super::ids::{RefKind, ShortRef};
use super::investigation::Investigation;
use super::job::JobRecord;
use super::note::Note;
use super::page::Page;
use super::relationship::Relationship;
use super::source::Source;
use super::timeline::TimelineEvent;

/// Links an entity to a page it appears on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PageEntityLink {
    pub page_id: Uuid,
    pub entity_id: Uuid,
}

/// Every record of one investigation. This is also the portable export format body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InvestigationData {
    pub investigation: Investigation,
    pub pages: Vec<Page>,
    pub sources: Vec<Source>,
    pub findings: Vec<Finding>,
    pub entities: Vec<Entity>,
    pub relationships: Vec<Relationship>,
    pub notes: Vec<Note>,
    pub artifacts: Vec<Artifact>,
    pub timeline: Vec<TimelineEvent>,
    pub jobs: Vec<JobRecord>,
    pub agent_runs: Vec<AgentRun>,
    pub proposals: Vec<Proposal>,
    pub page_entities: Vec<PageEntityLink>,
}

impl InvestigationData {
    pub fn empty(investigation: Investigation) -> Self {
        Self {
            investigation,
            pages: Vec::new(),
            sources: Vec::new(),
            findings: Vec::new(),
            entities: Vec::new(),
            relationships: Vec::new(),
            notes: Vec::new(),
            artifacts: Vec::new(),
            timeline: Vec::new(),
            jobs: Vec::new(),
            agent_runs: Vec::new(),
            proposals: Vec::new(),
            page_entities: Vec::new(),
        }
    }
}

#[derive(Debug, Default)]
struct WorkspaceIndex {
    pages: HashMap<Uuid, usize>,
    sources: HashMap<Uuid, usize>,
    findings: HashMap<Uuid, usize>,
    entities: HashMap<Uuid, usize>,
    notes: HashMap<Uuid, usize>,
    artifacts: HashMap<Uuid, usize>,
    agent_runs: HashMap<Uuid, usize>,
    by_ref: HashMap<ShortRef, Uuid>,
    refs: HashMap<Uuid, ShortRef>,
}

/// An investigation loaded for interactive use.
#[derive(Debug)]
pub struct Workspace {
    pub data: InvestigationData,
    index: WorkspaceIndex,
}

impl Workspace {
    pub fn new(data: InvestigationData) -> Self {
        let index = build_index(&data);
        Self { data, index }
    }

    pub fn investigation(&self) -> &Investigation {
        &self.data.investigation
    }

    pub fn id(&self) -> Uuid {
        self.data.investigation.id
    }

    pub fn page(&self, id: Uuid) -> Option<&Page> {
        self.index.pages.get(&id).map(|&i| &self.data.pages[i])
    }

    pub fn source(&self, id: Uuid) -> Option<&Source> {
        self.index.sources.get(&id).map(|&i| &self.data.sources[i])
    }

    pub fn finding(&self, id: Uuid) -> Option<&Finding> {
        self.index
            .findings
            .get(&id)
            .map(|&i| &self.data.findings[i])
    }

    pub fn entity(&self, id: Uuid) -> Option<&Entity> {
        self.index
            .entities
            .get(&id)
            .map(|&i| &self.data.entities[i])
    }

    pub fn note(&self, id: Uuid) -> Option<&Note> {
        self.index.notes.get(&id).map(|&i| &self.data.notes[i])
    }

    pub fn artifact(&self, id: Uuid) -> Option<&Artifact> {
        self.index
            .artifacts
            .get(&id)
            .map(|&i| &self.data.artifacts[i])
    }

    pub fn agent_run(&self, id: Uuid) -> Option<&AgentRun> {
        self.index
            .agent_runs
            .get(&id)
            .map(|&i| &self.data.agent_runs[i])
    }

    pub fn job(&self, id: Uuid) -> Option<&JobRecord> {
        self.data.jobs.iter().find(|job| job.id == id)
    }

    /// Resolves a short reference (`S-12`) to the record's UUID.
    pub fn resolve_ref(&self, reference: ShortRef) -> Option<Uuid> {
        self.index.by_ref.get(&reference).copied()
    }

    /// The short reference for a record UUID.
    pub fn ref_of(&self, id: Uuid) -> Option<ShortRef> {
        self.index.refs.get(&id).copied()
    }

    /// Formats a list of source IDs as `S-1, S-2`.
    pub fn format_source_refs(&self, ids: &[Uuid]) -> String {
        ids.iter()
            .filter_map(|id| self.ref_of(*id))
            .map(|reference| reference.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn entity_by_value(&self, kind: &EntityKind, value: &str) -> Option<&Entity> {
        let normalized = kind.normalize_value(value);
        self.data
            .entities
            .iter()
            .find(|entity| &entity.kind == kind && entity.value == normalized)
    }

    /// Finds an entity by value regardless of kind (used for agent-supplied identifiers).
    pub fn entity_by_any_value(&self, value: &str) -> Option<&Entity> {
        let trimmed = value.trim();
        self.data.entities.iter().find(|entity| {
            entity.value.eq_ignore_ascii_case(trimmed)
                || entity.kind.normalize_value(trimmed) == entity.value
        })
    }

    pub fn outgoing(&self, entity_id: Uuid) -> Vec<&Relationship> {
        self.data
            .relationships
            .iter()
            .filter(|relationship| relationship.from_id == entity_id)
            .collect()
    }

    pub fn incoming(&self, entity_id: Uuid) -> Vec<&Relationship> {
        self.data
            .relationships
            .iter()
            .filter(|relationship| relationship.to_id == entity_id)
            .collect()
    }

    pub fn page_entity_ids(&self, page_id: Uuid) -> Vec<Uuid> {
        let mut ids: Vec<Uuid> = self
            .data
            .page_entities
            .iter()
            .filter(|link| link.page_id == page_id)
            .map(|link| link.entity_id)
            .collect();
        if let Some(primary) = self.page(page_id).and_then(|page| page.entity_id)
            && !ids.contains(&primary)
        {
            ids.insert(0, primary);
        }
        ids
    }

    pub fn pages_for_entity(&self, entity_id: Uuid) -> Vec<&Page> {
        self.data
            .pages
            .iter()
            .filter(|page| {
                page.entity_id == Some(entity_id)
                    || self
                        .data
                        .page_entities
                        .iter()
                        .any(|link| link.page_id == page.id && link.entity_id == entity_id)
            })
            .collect()
    }

    pub fn findings_citing(&self, source_id: Uuid) -> Vec<&Finding> {
        self.data
            .findings
            .iter()
            .filter(|finding| finding.source_ids.contains(&source_id))
            .collect()
    }

    pub fn entities_citing(&self, source_id: Uuid) -> Vec<&Entity> {
        self.data
            .entities
            .iter()
            .filter(|entity| entity.source_ids.contains(&source_id))
            .collect()
    }

    pub fn proposals_for_run(&self, run_id: Uuid) -> Vec<&Proposal> {
        self.data
            .proposals
            .iter()
            .filter(|proposal| proposal.agent_run_id == run_id)
            .collect()
    }

    pub fn pending_proposal_count(&self) -> usize {
        self.data
            .proposals
            .iter()
            .filter(|proposal| proposal.status == ProposalStatus::Pending)
            .count()
    }

    /// Runs in a conversation thread, oldest first.
    pub fn thread_runs(&self, thread_id: Uuid) -> Vec<&AgentRun> {
        let mut runs: Vec<&AgentRun> = self
            .data
            .agent_runs
            .iter()
            .filter(|run| run.thread_id == thread_id)
            .collect();
        runs.sort_by_key(|run| run.seq);
        runs
    }
}

fn build_index(data: &InvestigationData) -> WorkspaceIndex {
    let mut index = WorkspaceIndex::default();
    let mut register = |id: Uuid, kind: RefKind, seq: i64| {
        let reference = ShortRef::new(kind, seq);
        index.by_ref.insert(reference, id);
        index.refs.insert(id, reference);
    };
    for page in &data.pages {
        register(page.id, RefKind::Page, page.seq);
    }
    for source in &data.sources {
        register(source.id, RefKind::Source, source.seq);
    }
    for finding in &data.findings {
        register(finding.id, RefKind::Finding, finding.seq);
    }
    for entity in &data.entities {
        register(entity.id, RefKind::Entity, entity.seq);
    }
    for relationship in &data.relationships {
        register(relationship.id, RefKind::Relationship, relationship.seq);
    }
    for note in &data.notes {
        register(note.id, RefKind::Note, note.seq);
    }
    for artifact in &data.artifacts {
        register(artifact.id, RefKind::Artifact, artifact.seq);
    }
    for event in &data.timeline {
        register(event.id, RefKind::Timeline, event.seq);
    }
    for job in &data.jobs {
        register(job.id, RefKind::Job, job.seq);
    }
    for run in &data.agent_runs {
        register(run.id, RefKind::AgentRun, run.seq);
    }
    for proposal in &data.proposals {
        register(proposal.id, RefKind::Proposal, proposal.seq);
    }
    index.pages = position_map(data.pages.iter().map(|page| page.id));
    index.sources = position_map(data.sources.iter().map(|source| source.id));
    index.findings = position_map(data.findings.iter().map(|finding| finding.id));
    index.entities = position_map(data.entities.iter().map(|entity| entity.id));
    index.notes = position_map(data.notes.iter().map(|note| note.id));
    index.artifacts = position_map(data.artifacts.iter().map(|artifact| artifact.id));
    index.agent_runs = position_map(data.agent_runs.iter().map(|run| run.id));
    index
}

fn position_map(ids: impl Iterator<Item = Uuid>) -> HashMap<Uuid, usize> {
    ids.enumerate()
        .map(|(position, id)| (id, position))
        .collect()
}
