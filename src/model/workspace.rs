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
use super::audit::AuditEntry;
use super::collaboration::{Member, same_user};
use super::entity::{Entity, EntityKind};
use super::event::Event;
use super::finding::Finding;
use super::ids::{RefKind, ShortRef};
use super::investigation::Investigation;
use super::job::JobRecord;
use super::note::Note;
use super::page::Page;
use super::relationship::Relationship;
use super::resolution::Resolution;
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
    /// Resolution bags. Empty on version-1 archives (`#[serde(default)]`).
    #[serde(default)]
    pub resolutions: Vec<Resolution>,
    /// First-class Event objects (Phase 6). Empty on version-1 and version-2 archives.
    #[serde(default)]
    pub events: Vec<Event>,
    /// Append-only governance log (Phase 7). Empty on version-1–3 archives.
    #[serde(default)]
    pub audit: Vec<AuditEntry>,
    /// Case members and roles (Phase 7). Empty means single-user owner.
    #[serde(default)]
    pub members: Vec<Member>,
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
            resolutions: Vec::new(),
            events: Vec::new(),
            audit: Vec::new(),
            members: Vec::new(),
        }
    }
}

#[derive(Debug, Default)]
struct ShortRefIndex {
    by_ref: HashMap<ShortRef, Uuid>,
    by_id: HashMap<Uuid, ShortRef>,
}

impl ShortRefIndex {
    fn register(&mut self, id: Uuid, kind: RefKind, seq: i64) {
        let reference = ShortRef::new(kind, seq);
        self.by_ref.insert(reference, id);
        self.by_id.insert(id, reference);
    }

    fn unregister(&mut self, id: Uuid) {
        if let Some(reference) = self.by_id.remove(&id) {
            self.by_ref.remove(&reference);
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
    short_refs: ShortRefIndex,
}

/// Records produced by one mutation, merged into a loaded [`Workspace`] without a full reload.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkspaceDelta {
    pub investigation: Option<Investigation>,
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
    /// Page-entity links to add. Removals are not represented; those paths reload.
    pub page_entities: Vec<PageEntityLink>,
    pub resolutions: Vec<Resolution>,
    pub events: Vec<Event>,
    pub audit: Vec<AuditEntry>,
    /// Members upserted by user_id. Removals are not represented; those paths reload.
    pub members: Vec<Member>,
    /// Finding IDs deleted by this mutation (for example `resolved_findings`).
    pub removed_finding_ids: Vec<Uuid>,
}

impl WorkspaceDelta {
    pub fn is_empty(&self) -> bool {
        self.investigation.is_none()
            && self.pages.is_empty()
            && self.sources.is_empty()
            && self.findings.is_empty()
            && self.entities.is_empty()
            && self.relationships.is_empty()
            && self.notes.is_empty()
            && self.artifacts.is_empty()
            && self.timeline.is_empty()
            && self.jobs.is_empty()
            && self.agent_runs.is_empty()
            && self.proposals.is_empty()
            && self.page_entities.is_empty()
            && self.resolutions.is_empty()
            && self.events.is_empty()
            && self.audit.is_empty()
            && self.members.is_empty()
            && self.removed_finding_ids.is_empty()
    }
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

    /// Merges records from a mutation into this snapshot and updates lookup indexes in place.
    pub fn apply_delta(&mut self, delta: WorkspaceDelta) {
        if let Some(investigation) = delta.investigation {
            self.data.investigation = investigation;
        }
        for page in delta.pages {
            upsert_indexed(
                &mut self.data.pages,
                &mut self.index.pages,
                &mut self.index.short_refs,
                page.id,
                RefKind::Page,
                page.seq,
                page,
            );
        }
        for source in delta.sources {
            upsert_indexed(
                &mut self.data.sources,
                &mut self.index.sources,
                &mut self.index.short_refs,
                source.id,
                RefKind::Source,
                source.seq,
                source,
            );
        }
        for finding in delta.findings {
            upsert_indexed(
                &mut self.data.findings,
                &mut self.index.findings,
                &mut self.index.short_refs,
                finding.id,
                RefKind::Finding,
                finding.seq,
                finding,
            );
        }
        for entity in delta.entities {
            upsert_indexed(
                &mut self.data.entities,
                &mut self.index.entities,
                &mut self.index.short_refs,
                entity.id,
                RefKind::Entity,
                entity.seq,
                entity,
            );
        }
        for relationship in delta.relationships {
            upsert_unindexed(
                &mut self.data.relationships,
                &mut self.index.short_refs,
                relationship.id,
                RefKind::Relationship,
                relationship.seq,
                |item| item.id,
                relationship,
            );
        }
        for note in delta.notes {
            upsert_indexed(
                &mut self.data.notes,
                &mut self.index.notes,
                &mut self.index.short_refs,
                note.id,
                RefKind::Note,
                note.seq,
                note,
            );
        }
        for artifact in delta.artifacts {
            upsert_indexed(
                &mut self.data.artifacts,
                &mut self.index.artifacts,
                &mut self.index.short_refs,
                artifact.id,
                RefKind::Artifact,
                artifact.seq,
                artifact,
            );
        }
        for event in delta.timeline {
            upsert_unindexed(
                &mut self.data.timeline,
                &mut self.index.short_refs,
                event.id,
                RefKind::Timeline,
                event.seq,
                |item| item.id,
                event,
            );
        }
        for job in delta.jobs {
            upsert_unindexed(
                &mut self.data.jobs,
                &mut self.index.short_refs,
                job.id,
                RefKind::Job,
                job.seq,
                |item| item.id,
                job,
            );
        }
        for run in delta.agent_runs {
            upsert_indexed(
                &mut self.data.agent_runs,
                &mut self.index.agent_runs,
                &mut self.index.short_refs,
                run.id,
                RefKind::AgentRun,
                run.seq,
                run,
            );
        }
        for proposal in delta.proposals {
            upsert_unindexed(
                &mut self.data.proposals,
                &mut self.index.short_refs,
                proposal.id,
                RefKind::Proposal,
                proposal.seq,
                |item| item.id,
                proposal,
            );
        }
        for resolution in delta.resolutions {
            upsert_unindexed(
                &mut self.data.resolutions,
                &mut self.index.short_refs,
                resolution.id,
                RefKind::Resolution,
                resolution.seq,
                |item| item.id,
                resolution,
            );
        }
        for event in delta.events {
            upsert_unindexed(
                &mut self.data.events,
                &mut self.index.short_refs,
                event.id,
                RefKind::Event,
                event.seq,
                |item| item.id,
                event,
            );
        }
        for entry in delta.audit {
            upsert_unindexed(
                &mut self.data.audit,
                &mut self.index.short_refs,
                entry.id,
                RefKind::Audit,
                entry.seq,
                |item| item.id,
                entry,
            );
        }
        for member in delta.members {
            if let Some(existing) = self
                .data
                .members
                .iter_mut()
                .find(|candidate| same_user(&candidate.user_id, &member.user_id))
            {
                *existing = member;
            } else {
                self.data.members.push(member);
            }
        }
        for link in delta.page_entities {
            if !self.data.page_entities.iter().any(|existing| {
                existing.page_id == link.page_id && existing.entity_id == link.entity_id
            }) {
                self.data.page_entities.push(link);
            }
        }
        for finding_id in delta.removed_finding_ids {
            remove_indexed(
                &mut self.data.findings,
                &mut self.index.findings,
                &mut self.index.short_refs,
                finding_id,
                |finding| finding.id,
            );
        }
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
        self.index.short_refs.by_ref.get(&reference).copied()
    }

    /// The short reference for a record UUID.
    pub fn ref_of(&self, id: Uuid) -> Option<ShortRef> {
        self.index.short_refs.by_id.get(&id).copied()
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

    pub fn visible_entities(&self) -> Vec<&Entity> {
        self.data
            .entities
            .iter()
            .filter(|entity| !self.data.is_absorbed(entity.id))
            .collect()
    }

    pub fn constituents_of(&self, canonical_id: Uuid) -> Vec<&Entity> {
        self.data
            .constituent_ids_of(canonical_id)
            .into_iter()
            .filter_map(|id| self.entity(id))
            .collect()
    }

    pub fn canonical_id(&self, entity_id: Uuid) -> Uuid {
        self.data.canonical_id(entity_id)
    }

    pub fn latest_active_resolution(&self, entity_id: Uuid) -> Option<&Resolution> {
        self.data
            .active_resolutions()
            .filter(|resolution| {
                resolution.canonical_id == entity_id || resolution.contains(entity_id)
            })
            .max_by_key(|resolution| resolution.seq)
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
        index.short_refs.register(id, kind, seq);
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
    for resolution in &data.resolutions {
        register(resolution.id, RefKind::Resolution, resolution.seq);
    }
    for event in &data.events {
        register(event.id, RefKind::Event, event.seq);
    }
    for entry in &data.audit {
        register(entry.id, RefKind::Audit, entry.seq);
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

fn upsert_indexed<T>(
    items: &mut Vec<T>,
    positions: &mut HashMap<Uuid, usize>,
    short_refs: &mut ShortRefIndex,
    id: Uuid,
    kind: RefKind,
    seq: i64,
    record: T,
) {
    if let Some(&position) = positions.get(&id) {
        items[position] = record;
        return;
    }
    let position = items.len();
    items.push(record);
    positions.insert(id, position);
    short_refs.register(id, kind, seq);
}

fn upsert_unindexed<T>(
    items: &mut Vec<T>,
    short_refs: &mut ShortRefIndex,
    id: Uuid,
    kind: RefKind,
    seq: i64,
    get_id: fn(&T) -> Uuid,
    record: T,
) {
    if let Some(existing) = items.iter_mut().find(|item| get_id(item) == id) {
        *existing = record;
        return;
    }
    items.push(record);
    short_refs.register(id, kind, seq);
}

fn remove_indexed<T>(
    items: &mut Vec<T>,
    positions: &mut HashMap<Uuid, usize>,
    short_refs: &mut ShortRefIndex,
    id: Uuid,
    get_id: fn(&T) -> Uuid,
) {
    let Some(_) = positions.remove(&id) else {
        return;
    };
    items.retain(|item| get_id(item) != id);
    *positions = position_map(items.iter().map(get_id));
    short_refs.unregister(id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EntityKind, Investigation, Marking, Provenance, now};

    fn sample_entity(investigation_id: Uuid, seq: i64, value: &str) -> Entity {
        let timestamp = now();
        Entity {
            id: Uuid::new_v4(),
            investigation_id,
            seq,
            kind: EntityKind::Domain,
            value: value.into(),
            label: String::new(),
            notes: String::new(),
            provenance: Provenance::Analyst,
            fields: crate::model::Fields::new(),
            source_ids: Vec::new(),
            created_at: timestamp,
            updated_at: timestamp,
        }
    }

    #[test]
    fn apply_delta_upserts_without_dropping_existing_records() {
        let timestamp = now();
        let investigation = Investigation {
            id: Uuid::new_v4(),
            name: "Case".into(),
            description: String::new(),
            tags: Vec::new(),
            marking: Marking::Unmarked,
            created_at: timestamp,
            updated_at: timestamp,
        };
        let first = sample_entity(investigation.id, 1, "one.example");
        let second = sample_entity(investigation.id, 2, "two.example");
        let mut data = InvestigationData::empty(investigation.clone());
        data.entities.push(first.clone());
        let mut workspace = Workspace::new(data);
        let mut updated_first = first.clone();
        updated_first.label = "One".into();
        workspace.apply_delta(WorkspaceDelta {
            entities: vec![updated_first.clone(), second.clone()],
            ..WorkspaceDelta::default()
        });
        assert_eq!(workspace.data.entities.len(), 2);
        assert_eq!(
            workspace.entity(first.id).map(|e| e.label.as_str()),
            Some("One")
        );
        assert_eq!(
            workspace.entity(second.id).map(|e| e.value.as_str()),
            Some("two.example")
        );
        assert_eq!(
            workspace.ref_of(second.id).map(|r| r.to_string()),
            Some("E-2".into())
        );
        assert_eq!(
            workspace.resolve_ref("E-1".parse().expect("ref")),
            Some(first.id)
        );
    }
}
