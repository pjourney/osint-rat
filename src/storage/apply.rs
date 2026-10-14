//! Applying an `EvidenceBatch` (collector output) in a single transaction.

use std::collections::HashMap;

use color_eyre::eyre::Result;
use rusqlite::Connection;
use uuid::Uuid;

use super::Store;
use super::evidence::{
    FindingSync, NewSourceRecord, PriorObservation, TimelineInsert, clear_collected_finding,
    find_duplicate_source, find_prior_observation, insert_artifact, insert_ingested_finding,
    insert_source, insert_timeline_event, mark_source_seen, sync_collected_finding,
};
use super::graph::{
    link_entity_source, link_page_entity, link_relationship_source, merge_entity_fields,
    upsert_entity, upsert_relationship,
};
use super::touch_investigation;
use crate::model::{EntityKey, EvidenceBatch, NewSource, Provenance, TimelineKind, Timestamp};

/// Where a batch is being applied.
#[derive(Clone, Copy, Debug)]
pub struct ApplyContext {
    pub investigation_id: Uuid,
    pub page_id: Option<Uuid>,
    pub job_id: Option<Uuid>,
}

/// What changed when a batch was applied.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApplySummary {
    /// Source IDs in the same order as `EvidenceBatch::sources`.
    pub source_ids: Vec<Uuid>,
    pub new_sources: usize,
    pub unchanged_sources: usize,
    pub new_entities: usize,
    pub total_entities: usize,
    pub new_relationships: usize,
    pub new_events: usize,
    pub new_artifacts: usize,
    /// Sources whose content differed from the latest stored observation of the same record.
    pub changed_observations: usize,
    pub new_findings: usize,
    pub updated_findings: usize,
    pub resolved_findings: usize,
}

impl ApplySummary {
    pub fn describe(&self) -> String {
        let mut parts = vec![format!("{} new source(s)", self.new_sources)];
        if self.unchanged_sources > 0 {
            parts.push(format!("{} unchanged", self.unchanged_sources));
        }
        parts.push(format!(
            "{} new entities ({} seen)",
            self.new_entities, self.total_entities
        ));
        parts.push(format!("{} new relationships", self.new_relationships));
        if self.new_events > 0 {
            parts.push(format!("{} timeline events", self.new_events));
        }
        if self.new_artifacts > 0 {
            parts.push(format!("{} artifacts", self.new_artifacts));
        }
        if self.changed_observations > 0 {
            parts.push(format!(
                "{} changed observation(s)",
                self.changed_observations
            ));
        }
        if self.new_findings > 0 {
            parts.push(format!("{} diagnostic finding(s)", self.new_findings));
        }
        if self.resolved_findings > 0 {
            parts.push(format!("{} resolved finding(s)", self.resolved_findings));
        }
        parts.join(", ")
    }
}

impl Store {
    /// Inserts sources (re-using identical earlier observations), upserts entities and
    /// relationships, links everything to its supporting sources, and records timeline events.
    pub fn apply_batch(
        &mut self,
        context: ApplyContext,
        batch: &EvidenceBatch,
    ) -> Result<ApplySummary> {
        let transaction = self.connection_mut().transaction()?;
        let summary = apply_batch_on(&transaction, context, batch)?;
        touch_investigation(&transaction, context.investigation_id)?;
        transaction.commit()?;
        Ok(summary)
    }
}

pub(crate) fn apply_batch_on(
    connection: &Connection,
    context: ApplyContext,
    batch: &EvidenceBatch,
) -> rusqlite::Result<ApplySummary> {
    let mut summary = ApplySummary::default();
    let changes = apply_sources(connection, context, batch, &mut summary)?;
    let entity_ids = apply_entities(connection, context, batch, &mut summary)?;
    apply_relationships(connection, context, batch, &entity_ids, &mut summary)?;
    apply_timeline(connection, context, batch, &entity_ids, &mut summary)?;
    apply_observation_changes(connection, context, &changes, &mut summary)?;
    apply_artifacts(connection, context, batch, &mut summary)?;
    apply_diagnostic_findings(connection, context, batch, &mut summary)?;
    Ok(summary)
}

/// A public record whose latest stored copy differs from the copy just collected.
struct ObservationChange {
    source_index: usize,
    occurred_at: Timestamp,
    title: String,
    description: String,
}

fn apply_sources(
    connection: &Connection,
    context: ApplyContext,
    batch: &EvidenceBatch,
    summary: &mut ApplySummary,
) -> rusqlite::Result<Vec<ObservationChange>> {
    let mut changes = Vec::new();
    for (source_index, source) in batch.sources.iter().enumerate() {
        if let Some(existing) = find_duplicate_source(connection, context.investigation_id, source)?
        {
            mark_source_seen(connection, existing, source.retrieved_at)?;
            summary.source_ids.push(existing);
            summary.unchanged_sources += 1;
            continue;
        }
        let prior = find_prior_observation(connection, context.investigation_id, source)?;
        let record = NewSourceRecord {
            investigation_id: context.investigation_id,
            page_id: context.page_id,
            job_id: context.job_id,
            source: source.clone(),
        };
        let inserted = insert_source(connection, &record)?;
        if let Some(change) = observation_change(source_index, source, prior.as_ref()) {
            changes.push(change);
        }
        summary.source_ids.push(inserted.id);
        summary.new_sources += 1;
    }
    Ok(changes)
}

fn observation_change(
    source_index: usize,
    source: &NewSource,
    prior: Option<&PriorObservation>,
) -> Option<ObservationChange> {
    let prior = prior?;
    let new_hash = source.content_hash.as_deref()?;
    if prior.content_hash.as_deref() == Some(new_hash) {
        return None;
    }
    let previous = prior
        .content_hash
        .as_deref()
        .map(hash_prefix)
        .unwrap_or_else(|| "none".into());
    Some(ObservationChange {
        source_index,
        occurred_at: source.retrieved_at,
        title: format!("{} changed ({})", source.title, hash_prefix(new_hash)),
        description: format!(
            "The latest stored observation hash {previous} differs from the newly collected hash {}.",
            hash_prefix(new_hash)
        ),
    })
}

fn hash_prefix(hash: &str) -> String {
    hash.chars().take(12).collect()
}

fn apply_observation_changes(
    connection: &Connection,
    context: ApplyContext,
    changes: &[ObservationChange],
    summary: &mut ApplySummary,
) -> rusqlite::Result<()> {
    for change in changes {
        let source_id = summary.source_ids.get(change.source_index).copied();
        let inserted = insert_timeline_event(
            connection,
            TimelineInsert {
                investigation_id: context.investigation_id,
                occurred_at: change.occurred_at,
                kind: &TimelineKind::Changed,
                title: &change.title,
                description: &change.description,
                provenance: Provenance::Collected,
                source_id,
                entity_id: None,
                page_id: context.page_id,
            },
        )?;
        if inserted {
            summary.new_events += 1;
            summary.changed_observations += 1;
        }
    }
    Ok(())
}

fn apply_diagnostic_findings(
    connection: &Connection,
    context: ApplyContext,
    batch: &EvidenceBatch,
    summary: &mut ApplySummary,
) -> rusqlite::Result<()> {
    if honors_resolved_findings(batch) {
        for title in &batch.resolved_findings {
            if clear_collected_finding(connection, context.investigation_id, title)? {
                summary.resolved_findings += 1;
            }
        }
    }
    for finding in &batch.findings {
        let source_ids: Vec<Uuid> = finding
            .source
            .and_then(|key| summary.source_ids.get(key).copied())
            .into_iter()
            .collect();
        let sync = if finding.provenance == Provenance::Collected {
            sync_collected_finding(
                connection,
                context.investigation_id,
                context.page_id,
                &finding.title,
                &finding.details,
                finding.confidence,
                &source_ids,
            )?
        } else {
            insert_ingested_finding(
                connection,
                context.investigation_id,
                context.page_id,
                finding,
                &source_ids,
            )?
        };
        match sync {
            FindingSync::Created => summary.new_findings += 1,
            FindingSync::Updated => summary.updated_findings += 1,
            FindingSync::LeftAlone => {}
        }
    }
    Ok(())
}

/// Built-in collectors emit `Collected` sources. File/sidecar ingest forces `External`,
/// so an untrusted `resolved_findings` list cannot delete collected diagnostics.
fn honors_resolved_findings(batch: &EvidenceBatch) -> bool {
    !batch.sources.is_empty()
        && batch
            .sources
            .iter()
            .all(|source| source.provenance == Provenance::Collected)
}

fn apply_entities(
    connection: &Connection,
    context: ApplyContext,
    batch: &EvidenceBatch,
    summary: &mut ApplySummary,
) -> rusqlite::Result<HashMap<EntityKey, Uuid>> {
    let mut entity_ids = HashMap::new();
    for entity in &batch.entities {
        let provenance = provenance_for(batch, &entity.sources);
        let (entity_id, created) = upsert_entity(
            connection,
            context.investigation_id,
            &entity.key,
            provenance,
            entity.label.as_deref(),
        )?;
        merge_entity_fields(connection, entity_id, &entity.fields)?;
        for key in &entity.sources {
            if let Some(source_id) = summary.source_ids.get(*key) {
                link_entity_source(connection, entity_id, *source_id)?;
            }
        }
        if entity.link_to_page
            && let Some(page_id) = context.page_id
        {
            link_page_entity(connection, page_id, entity_id)?;
        }
        summary.total_entities += 1;
        if created {
            summary.new_entities += 1;
        }
        entity_ids.insert(entity.key.clone(), entity_id);
    }
    Ok(entity_ids)
}

fn apply_relationships(
    connection: &Connection,
    context: ApplyContext,
    batch: &EvidenceBatch,
    entity_ids: &HashMap<EntityKey, Uuid>,
    summary: &mut ApplySummary,
) -> rusqlite::Result<()> {
    for relationship in &batch.relationships {
        let from = resolve_entity(connection, context, entity_ids, &relationship.from)?;
        let to = resolve_entity(connection, context, entity_ids, &relationship.to)?;
        let provenance = provenance_for(batch, &relationship.sources);
        let (relationship_id, created) = upsert_relationship(
            connection,
            context.investigation_id,
            from,
            &relationship.kind,
            to,
            provenance,
            relationship.confidence,
        )?;
        for key in &relationship.sources {
            if let Some(source_id) = summary.source_ids.get(*key) {
                link_relationship_source(connection, relationship_id, *source_id)?;
            }
        }
        if created {
            summary.new_relationships += 1;
        }
    }
    Ok(())
}

fn apply_timeline(
    connection: &Connection,
    context: ApplyContext,
    batch: &EvidenceBatch,
    entity_ids: &HashMap<EntityKey, Uuid>,
    summary: &mut ApplySummary,
) -> rusqlite::Result<()> {
    for event in &batch.timeline {
        let source_id = event
            .source
            .and_then(|key| summary.source_ids.get(key).copied());
        let entity_id = event
            .entity
            .as_ref()
            .and_then(|key| entity_ids.get(key).copied());
        let provenance = event
            .source
            .and_then(|key| batch.sources.get(key))
            .map(|source| source.provenance)
            .unwrap_or(Provenance::Collected);
        let inserted = insert_timeline_event(
            connection,
            TimelineInsert {
                investigation_id: context.investigation_id,
                occurred_at: event.occurred_at,
                kind: &event.kind,
                title: &event.title,
                description: &event.description,
                provenance,
                source_id,
                entity_id,
                page_id: context.page_id,
            },
        )?;
        if inserted {
            summary.new_events += 1;
        }
    }
    Ok(())
}

fn apply_artifacts(
    connection: &Connection,
    context: ApplyContext,
    batch: &EvidenceBatch,
    summary: &mut ApplySummary,
) -> rusqlite::Result<()> {
    for artifact in &batch.artifacts {
        let source_id = artifact
            .source
            .and_then(|key| summary.source_ids.get(key).copied());
        let (_, created) = insert_artifact(
            connection,
            context.investigation_id,
            context.page_id,
            artifact,
            source_id,
        )?;
        if created {
            summary.new_artifacts += 1;
        }
    }
    Ok(())
}

fn resolve_entity(
    connection: &Connection,
    context: ApplyContext,
    entity_ids: &HashMap<EntityKey, Uuid>,
    key: &EntityKey,
) -> rusqlite::Result<Uuid> {
    if let Some(entity_id) = entity_ids.get(key) {
        return Ok(*entity_id);
    }
    let (entity_id, _) = upsert_entity(
        connection,
        context.investigation_id,
        key,
        Provenance::Collected,
        None,
    )?;
    Ok(entity_id)
}

/// Entities and relationships inherit provenance from their first supporting source.
fn provenance_for(batch: &EvidenceBatch, sources: &[usize]) -> Provenance {
    sources
        .first()
        .and_then(|key| batch.sources.get(*key))
        .map(|source| source.provenance)
        .unwrap_or(Provenance::Collected)
}
