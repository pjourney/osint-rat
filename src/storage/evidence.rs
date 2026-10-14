//! Sources, findings, artifacts, and timeline events.

use color_eyre::eyre::{Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use super::graph::load_links;
use super::rows::{RowExt, id, json, opt_id, opt_ts, ts};
use super::{SeqTable, Store, next_seq, touch_investigation};
use crate::model::{
    Artifact, ArtifactKind, Confidence, Finding, NewArtifact, NewCollectedFinding, NewSource,
    Provenance, Source, SourceKind, TimelineEvent, TimelineKind, Timestamp, now,
};
use crate::paths::is_safe_relative_path;

/// A source to be inserted, with its investigation context.
#[derive(Clone, Debug)]
pub struct NewSourceRecord {
    pub investigation_id: Uuid,
    pub page_id: Option<Uuid>,
    pub job_id: Option<Uuid>,
    pub source: NewSource,
}

/// Input for creating a finding.
#[derive(Clone, Debug)]
pub struct NewFinding {
    pub investigation_id: Uuid,
    pub page_id: Option<Uuid>,
    pub title: String,
    pub details: String,
    pub confidence: Confidence,
    pub provenance: Provenance,
    pub source_ids: Vec<Uuid>,
}

pub(crate) const SOURCE_COLUMNS: &str = "id, investigation_id, seq, kind, title, url, retrieved_at, last_seen_at, published_at, \
     author, collector, content_hash, snapshot_path, content, fields, notes, provenance, page_id, job_id";

pub(crate) fn insert_source_record(
    connection: &Connection,
    source: &Source,
) -> rusqlite::Result<()> {
    connection.execute(
        &format!(
            "INSERT INTO sources ({SOURCE_COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)"
        ),
        params![
            id(&source.id),
            id(&source.investigation_id),
            source.seq,
            source.kind.as_str(),
            source.title,
            source.url,
            ts(&source.retrieved_at),
            opt_ts(&source.last_seen_at),
            opt_ts(&source.published_at),
            source.author,
            source.collector,
            source.content_hash,
            source.snapshot_path,
            source.content,
            json(&source.fields),
            source.notes,
            source.provenance.as_str(),
            opt_id(&source.page_id),
            opt_id(&source.job_id),
        ],
    )?;
    Ok(())
}

pub(crate) fn insert_source(
    connection: &Connection,
    record: &NewSourceRecord,
) -> rusqlite::Result<Source> {
    let input = &record.source;
    let source = Source {
        id: Uuid::new_v4(),
        investigation_id: record.investigation_id,
        seq: next_seq(connection, SeqTable::Sources, record.investigation_id)?,
        kind: input.kind.clone(),
        title: input.title.clone(),
        url: input.url.clone(),
        retrieved_at: input.retrieved_at,
        last_seen_at: None,
        published_at: input.published_at,
        author: input.author.clone(),
        collector: input.collector.clone(),
        content_hash: input.content_hash.clone(),
        snapshot_path: input
            .snapshot_path
            .as_deref()
            .filter(|path| is_safe_relative_path(path))
            .map(ToString::to_string),
        content: input.content.clone(),
        fields: input.fields.clone(),
        notes: String::new(),
        provenance: input.provenance,
        page_id: record.page_id,
        job_id: record.job_id,
    };
    insert_source_record(connection, &source)?;
    Ok(source)
}

/// Finds an existing source with identical collector, title, URL, and content hash.
pub(crate) fn find_duplicate_source(
    connection: &Connection,
    investigation_id: Uuid,
    source: &NewSource,
) -> rusqlite::Result<Option<Uuid>> {
    let Some(hash) = source.content_hash.as_deref() else {
        return Ok(None);
    };
    let found: Option<String> = connection
        .query_row(
            "SELECT id FROM sources WHERE investigation_id = ?1 AND collector IS ?2 AND content_hash = ?3
               AND title = ?4 AND url IS ?5
             ORDER BY seq DESC LIMIT 1",
            params![id(&investigation_id), source.collector, hash, source.title, source.url],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.and_then(|value| Uuid::parse_str(&value).ok()))
}

/// The newest stored copy of the same observation, regardless of content hash.
pub(crate) struct PriorObservation {
    pub content_hash: Option<String>,
}

/// Finds the latest source with the same collector, title, and URL.
pub(crate) fn find_prior_observation(
    connection: &Connection,
    investigation_id: Uuid,
    source: &NewSource,
) -> rusqlite::Result<Option<PriorObservation>> {
    connection
        .query_row(
            "SELECT content_hash FROM sources
             WHERE investigation_id = ?1 AND collector IS ?2 AND title = ?3 AND url IS ?4
             ORDER BY seq DESC LIMIT 1",
            params![
                id(&investigation_id),
                source.collector,
                source.title,
                source.url
            ],
            |row| {
                Ok(PriorObservation {
                    content_hash: row.get(0)?,
                })
            },
        )
        .optional()
}

pub(crate) fn mark_source_seen(
    connection: &Connection,
    source_id: Uuid,
    seen_at: Timestamp,
) -> rusqlite::Result<()> {
    connection.execute(
        "UPDATE sources SET last_seen_at = ?2 WHERE id = ?1",
        params![id(&source_id), ts(&seen_at)],
    )?;
    Ok(())
}

pub(crate) fn list_sources(
    connection: &Connection,
    investigation_id: Uuid,
) -> rusqlite::Result<Vec<Source>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {SOURCE_COLUMNS} FROM sources WHERE investigation_id = ?1 ORDER BY seq"
    ))?;
    let rows = statement.query_map(params![id(&investigation_id)], |row| {
        Ok(Source {
            id: row.uuid(0)?,
            investigation_id: row.uuid(1)?,
            seq: row.get(2)?,
            kind: SourceKind::parse(&row.get::<_, String>(3)?),
            title: row.get(4)?,
            url: row.get(5)?,
            retrieved_at: row.timestamp(6)?,
            last_seen_at: row.opt_timestamp(7)?,
            published_at: row.opt_timestamp(8)?,
            author: row.get(9)?,
            collector: row.get(10)?,
            content_hash: row.get(11)?,
            snapshot_path: row.get(12)?,
            content: row.get(13)?,
            fields: row.json(14)?,
            notes: row.get(15)?,
            provenance: Provenance::parse(&row.get::<_, String>(16)?)
                .unwrap_or(Provenance::Collected),
            page_id: row.opt_uuid(17)?,
            job_id: row.opt_uuid(18)?,
        })
    })?;
    rows.collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FindingSync {
    Created(Uuid),
    Updated(Uuid),
    LeftAlone,
}

/// Inserts or updates a collected diagnostic finding with this exact title.
/// Analyst and agent findings with the same title are left untouched.
pub(crate) fn sync_collected_finding(
    connection: &Connection,
    investigation_id: Uuid,
    page_id: Option<Uuid>,
    title: &str,
    details: &str,
    confidence: Confidence,
    source_ids: &[Uuid],
) -> rusqlite::Result<FindingSync> {
    if let Some((finding_id, provenance)) = finding_by_title(connection, investigation_id, title)? {
        if provenance != Provenance::Collected {
            return Ok(FindingSync::LeftAlone);
        }
        if let Some(page_id) = page_id {
            connection.execute(
                "UPDATE findings SET details = ?2, confidence = ?3, page_id = ?4, updated_at = ?5 WHERE id = ?1",
                params![
                    id(&finding_id),
                    details.trim_end(),
                    confidence.as_str(),
                    id(&page_id),
                    ts(&now())
                ],
            )?;
        } else {
            connection.execute(
                "UPDATE findings SET details = ?2, confidence = ?3, updated_at = ?4 WHERE id = ?1",
                params![
                    id(&finding_id),
                    details.trim_end(),
                    confidence.as_str(),
                    ts(&now())
                ],
            )?;
        }
        replace_finding_sources(connection, finding_id, source_ids)?;
        return Ok(FindingSync::Updated(finding_id));
    }
    let timestamp = now();
    let finding = Finding {
        id: Uuid::new_v4(),
        investigation_id,
        seq: next_seq(connection, SeqTable::Findings, investigation_id)?,
        page_id,
        title: title.trim().to_string(),
        details: details.trim_end().to_string(),
        confidence,
        provenance: Provenance::Collected,
        source_ids: source_ids.to_vec(),
        created_at: timestamp,
        updated_at: timestamp,
    };
    insert_finding_record(connection, &finding)?;
    Ok(FindingSync::Created(finding.id))
}

/// Inserts an ingested (non-collected) finding. Never updates or deletes collected diagnostics.
pub(crate) fn insert_ingested_finding(
    connection: &Connection,
    investigation_id: Uuid,
    page_id: Option<Uuid>,
    finding: &NewCollectedFinding,
    source_ids: &[Uuid],
) -> rusqlite::Result<FindingSync> {
    if finding_by_title_and_provenance(
        connection,
        investigation_id,
        &finding.title,
        finding.provenance,
    )?
    .is_some()
    {
        return Ok(FindingSync::LeftAlone);
    }
    let timestamp = now();
    let record = Finding {
        id: Uuid::new_v4(),
        investigation_id,
        seq: next_seq(connection, SeqTable::Findings, investigation_id)?,
        page_id,
        title: finding.title.trim().to_string(),
        details: finding.details.trim_end().to_string(),
        confidence: finding.confidence,
        provenance: finding.provenance,
        source_ids: source_ids.to_vec(),
        created_at: timestamp,
        updated_at: timestamp,
    };
    insert_finding_record(connection, &record)?;
    Ok(FindingSync::Created(record.id))
}

fn finding_by_title_and_provenance(
    connection: &Connection,
    investigation_id: Uuid,
    title: &str,
    provenance: Provenance,
) -> rusqlite::Result<Option<Uuid>> {
    connection
        .query_row(
            "SELECT id FROM findings WHERE investigation_id = ?1 AND title = ?2 AND provenance = ?3
             ORDER BY seq DESC LIMIT 1",
            params![id(&investigation_id), title, provenance.as_str()],
            |row| row.uuid(0),
        )
        .optional()
}

/// Deletes a collected finding with this title. Other provenances are kept.
pub(crate) fn clear_collected_finding(
    connection: &Connection,
    investigation_id: Uuid,
    title: &str,
) -> rusqlite::Result<bool> {
    let Some((finding_id, provenance)) = finding_by_title(connection, investigation_id, title)?
    else {
        return Ok(false);
    };
    if provenance != Provenance::Collected {
        return Ok(false);
    }
    let deleted = connection.execute(
        "DELETE FROM findings WHERE id = ?1",
        params![id(&finding_id)],
    )?;
    Ok(deleted > 0)
}

fn finding_by_title(
    connection: &Connection,
    investigation_id: Uuid,
    title: &str,
) -> rusqlite::Result<Option<(Uuid, Provenance)>> {
    connection
        .query_row(
            "SELECT id, provenance FROM findings WHERE investigation_id = ?1 AND title = ?2 ORDER BY seq DESC LIMIT 1",
            params![id(&investigation_id), title],
            |row| {
                let provenance = Provenance::parse(&row.get::<_, String>(1)?)
                    .unwrap_or(Provenance::Collected);
                Ok((row.uuid(0)?, provenance))
            },
        )
        .optional()
}

pub(crate) fn insert_finding_record(
    connection: &Connection,
    finding: &Finding,
) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO findings (id, investigation_id, seq, page_id, title, details, confidence, provenance, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id(&finding.id),
            id(&finding.investigation_id),
            finding.seq,
            opt_id(&finding.page_id),
            finding.title,
            finding.details,
            finding.confidence.as_str(),
            finding.provenance.as_str(),
            ts(&finding.created_at),
            ts(&finding.updated_at),
        ],
    )?;
    replace_finding_sources(connection, finding.id, &finding.source_ids)
}

fn replace_finding_sources(
    connection: &Connection,
    finding_id: Uuid,
    source_ids: &[Uuid],
) -> rusqlite::Result<()> {
    connection.execute(
        "DELETE FROM finding_sources WHERE finding_id = ?1",
        params![id(&finding_id)],
    )?;
    for source_id in source_ids {
        connection.execute(
            "INSERT OR IGNORE INTO finding_sources (finding_id, source_id) VALUES (?1, ?2)",
            params![id(&finding_id), id(source_id)],
        )?;
    }
    Ok(())
}

pub(crate) fn list_findings(
    connection: &Connection,
    investigation_id: Uuid,
) -> rusqlite::Result<Vec<Finding>> {
    let mut links = load_links(
        connection,
        "SELECT fs.finding_id, fs.source_id FROM finding_sources fs
         JOIN findings f ON f.id = fs.finding_id
         JOIN sources s ON s.id = fs.source_id
         WHERE f.investigation_id = ?1 ORDER BY s.seq",
        investigation_id,
    )?;
    let mut statement = connection.prepare(
        "SELECT id, investigation_id, seq, page_id, title, details, confidence, provenance, created_at, updated_at
         FROM findings WHERE investigation_id = ?1 ORDER BY seq",
    )?;
    let rows = statement.query_map(params![id(&investigation_id)], |row| {
        Ok(Finding {
            id: row.uuid(0)?,
            investigation_id: row.uuid(1)?,
            seq: row.get(2)?,
            page_id: row.opt_uuid(3)?,
            title: row.get(4)?,
            details: row.get(5)?,
            confidence: Confidence::parse(&row.get::<_, String>(6)?).unwrap_or_default(),
            provenance: Provenance::parse(&row.get::<_, String>(7)?).unwrap_or(Provenance::Analyst),
            source_ids: Vec::new(),
            created_at: row.timestamp(8)?,
            updated_at: row.timestamp(9)?,
        })
    })?;
    let mut findings = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    for finding in &mut findings {
        finding.source_ids = links.remove(&finding.id).unwrap_or_default();
    }
    Ok(findings)
}

pub(crate) fn insert_artifact_record(
    connection: &Connection,
    artifact: &Artifact,
) -> rusqlite::Result<usize> {
    connection.execute(
        "INSERT OR IGNORE INTO artifacts (id, investigation_id, seq, page_id, kind, name, path, mime, size_bytes, sha256, fields, source_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            id(&artifact.id),
            id(&artifact.investigation_id),
            artifact.seq,
            opt_id(&artifact.page_id),
            artifact.kind.as_str(),
            artifact.name,
            artifact.path,
            artifact.mime,
            artifact.size_bytes,
            artifact.sha256,
            json(&artifact.fields),
            opt_id(&artifact.source_id),
            ts(&artifact.created_at),
        ],
    )
}

/// Inserts an artifact; when one with the same path already exists, returns that one instead.
pub(crate) fn insert_artifact(
    connection: &Connection,
    investigation_id: Uuid,
    page_id: Option<Uuid>,
    input: &NewArtifact,
    source_id: Option<Uuid>,
) -> rusqlite::Result<(Uuid, bool)> {
    let existing: Option<String> = connection
        .query_row(
            "SELECT id FROM artifacts WHERE investigation_id = ?1 AND path = ?2",
            params![id(&investigation_id), input.path],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(existing_id) = existing.and_then(|value| Uuid::parse_str(&value).ok()) {
        return Ok((existing_id, false));
    }
    if !is_safe_relative_path(&input.path) {
        return Ok((Uuid::nil(), false));
    }
    let artifact = Artifact {
        id: Uuid::new_v4(),
        investigation_id,
        seq: next_seq(connection, SeqTable::Artifacts, investigation_id)?,
        page_id,
        kind: input.kind.clone(),
        name: input.name.clone(),
        path: input.path.clone(),
        mime: input.mime.clone(),
        size_bytes: input.size_bytes,
        sha256: input.sha256.clone(),
        fields: input.fields.clone(),
        source_id,
        created_at: now(),
    };
    insert_artifact_record(connection, &artifact)?;
    Ok((artifact.id, true))
}

pub(crate) fn list_artifacts(
    connection: &Connection,
    investigation_id: Uuid,
) -> rusqlite::Result<Vec<Artifact>> {
    let mut statement = connection.prepare(
        "SELECT id, investigation_id, seq, page_id, kind, name, path, mime, size_bytes, sha256, fields, source_id, created_at
         FROM artifacts WHERE investigation_id = ?1 ORDER BY seq",
    )?;
    let rows = statement.query_map(params![id(&investigation_id)], |row| {
        Ok(Artifact {
            id: row.uuid(0)?,
            investigation_id: row.uuid(1)?,
            seq: row.get(2)?,
            page_id: row.opt_uuid(3)?,
            kind: ArtifactKind::parse(&row.get::<_, String>(4)?),
            name: row.get(5)?,
            path: row.get(6)?,
            mime: row.get(7)?,
            size_bytes: row.get(8)?,
            sha256: row.get(9)?,
            fields: row.json(10)?,
            source_id: row.opt_uuid(11)?,
            created_at: row.timestamp(12)?,
        })
    })?;
    rows.collect()
}

pub(crate) fn insert_timeline_record(
    connection: &Connection,
    event: &TimelineEvent,
) -> rusqlite::Result<usize> {
    connection.execute(
        "INSERT OR IGNORE INTO timeline_events (id, investigation_id, seq, occurred_at, kind, title, description, provenance, source_id, entity_id, page_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            id(&event.id),
            id(&event.investigation_id),
            event.seq,
            ts(&event.occurred_at),
            event.kind.as_str(),
            event.title,
            event.description,
            event.provenance.as_str(),
            opt_id(&event.source_id),
            opt_id(&event.entity_id),
            opt_id(&event.page_id),
            ts(&event.created_at),
        ],
    )
}

/// Timeline event fields with resolved record IDs.
pub(crate) struct TimelineInsert<'a> {
    pub investigation_id: Uuid,
    pub occurred_at: Timestamp,
    pub kind: &'a TimelineKind,
    pub title: &'a str,
    pub description: &'a str,
    pub provenance: Provenance,
    pub source_id: Option<Uuid>,
    pub entity_id: Option<Uuid>,
    pub page_id: Option<Uuid>,
}

/// Inserts a timeline event unless an identical one (kind, time, title) already exists.
/// Returns `true` when a new event was stored.
pub(crate) fn insert_timeline_event(
    connection: &Connection,
    input: TimelineInsert<'_>,
) -> rusqlite::Result<bool> {
    let event = TimelineEvent {
        id: Uuid::new_v4(),
        investigation_id: input.investigation_id,
        seq: next_seq(connection, SeqTable::Timeline, input.investigation_id)?,
        occurred_at: input.occurred_at,
        kind: input.kind.clone(),
        title: input.title.to_string(),
        description: input.description.to_string(),
        provenance: input.provenance,
        source_id: input.source_id,
        entity_id: input.entity_id,
        page_id: input.page_id,
        created_at: now(),
    };
    Ok(insert_timeline_record(connection, &event)? > 0)
}

pub(crate) fn list_timeline(
    connection: &Connection,
    investigation_id: Uuid,
) -> rusqlite::Result<Vec<TimelineEvent>> {
    let mut statement = connection.prepare(
        "SELECT id, investigation_id, seq, occurred_at, kind, title, description, provenance, source_id, entity_id, page_id, created_at
         FROM timeline_events WHERE investigation_id = ?1 ORDER BY occurred_at, seq",
    )?;
    let rows = statement.query_map(params![id(&investigation_id)], |row| {
        Ok(TimelineEvent {
            id: row.uuid(0)?,
            investigation_id: row.uuid(1)?,
            seq: row.get(2)?,
            occurred_at: row.timestamp(3)?,
            kind: TimelineKind::parse(&row.get::<_, String>(4)?),
            title: row.get(5)?,
            description: row.get(6)?,
            provenance: Provenance::parse(&row.get::<_, String>(7)?)
                .unwrap_or(Provenance::Collected),
            source_id: row.opt_uuid(8)?,
            entity_id: row.opt_uuid(9)?,
            page_id: row.opt_uuid(10)?,
            created_at: row.timestamp(11)?,
        })
    })?;
    rows.collect()
}

impl Store {
    /// Adds a source directly (for example original material supplied by the analyst).
    pub fn add_source(&self, record: NewSourceRecord) -> Result<Source> {
        if record.source.title.trim().is_empty() {
            bail!("a source needs a title");
        }
        self.assert_can_mutate(record.investigation_id)?;
        let source = insert_source(self.connection(), &record)?;
        touch_investigation(self.connection(), record.investigation_id)?;
        Ok(source)
    }

    pub fn update_source_notes(&self, source_id: Uuid, notes: &str) -> Result<()> {
        self.assert_can_mutate_record("sources", source_id)?;
        self.connection().execute(
            "UPDATE sources SET notes = ?2 WHERE id = ?1",
            params![id(&source_id), notes],
        )?;
        Ok(())
    }

    pub fn delete_source(&self, source_id: Uuid) -> Result<()> {
        self.assert_can_mutate_record("sources", source_id)?;
        self.connection()
            .execute("DELETE FROM sources WHERE id = ?1", params![id(&source_id)])?;
        Ok(())
    }

    pub fn create_finding(&mut self, input: NewFinding) -> Result<Finding> {
        if input.title.trim().is_empty() {
            bail!("a finding needs a statement");
        }
        self.assert_can_mutate(input.investigation_id)?;
        let transaction = self.connection_mut().transaction()?;
        let timestamp = now();
        let finding = Finding {
            id: Uuid::new_v4(),
            investigation_id: input.investigation_id,
            seq: next_seq(&transaction, SeqTable::Findings, input.investigation_id)?,
            page_id: input.page_id,
            title: input.title.trim().to_string(),
            details: input.details.trim_end().to_string(),
            confidence: input.confidence,
            provenance: input.provenance,
            source_ids: input.source_ids,
            created_at: timestamp,
            updated_at: timestamp,
        };
        insert_finding_record(&transaction, &finding)?;
        touch_investigation(&transaction, finding.investigation_id)?;
        transaction.commit()?;
        Ok(finding)
    }

    /// Updates a finding's text, confidence, and cited sources. Provenance is never changed.
    pub fn update_finding(&mut self, finding: &Finding) -> Result<()> {
        if finding.title.trim().is_empty() {
            bail!("a finding needs a statement");
        }
        self.assert_can_mutate(finding.investigation_id)?;
        let transaction = self.connection_mut().transaction()?;
        transaction.execute(
            "UPDATE findings SET title = ?2, details = ?3, confidence = ?4, page_id = ?5, updated_at = ?6 WHERE id = ?1",
            params![
                id(&finding.id),
                finding.title.trim(),
                finding.details,
                finding.confidence.as_str(),
                opt_id(&finding.page_id),
                ts(&now()),
            ],
        )?;
        replace_finding_sources(&transaction, finding.id, &finding.source_ids)?;
        touch_investigation(&transaction, finding.investigation_id)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn delete_finding(&self, finding_id: Uuid) -> Result<()> {
        self.assert_can_mutate_record("findings", finding_id)?;
        self.connection().execute(
            "DELETE FROM findings WHERE id = ?1",
            params![id(&finding_id)],
        )?;
        Ok(())
    }

    /// Registers a file stored in the investigation directory. Returns the artifact ID.
    pub fn add_artifact(
        &self,
        investigation_id: Uuid,
        page_id: Option<Uuid>,
        input: &NewArtifact,
    ) -> Result<Uuid> {
        self.assert_can_mutate(investigation_id)?;
        let (artifact_id, _) =
            insert_artifact(self.connection(), investigation_id, page_id, input, None)?;
        touch_investigation(self.connection(), investigation_id)?;
        Ok(artifact_id)
    }

    pub fn delete_artifact(&self, artifact_id: Uuid) -> Result<()> {
        self.assert_can_mutate_record("artifacts", artifact_id)?;
        self.connection().execute(
            "DELETE FROM artifacts WHERE id = ?1",
            params![id(&artifact_id)],
        )?;
        Ok(())
    }

    /// Adds an analyst-entered timeline event.
    pub fn add_timeline_event(
        &self,
        investigation_id: Uuid,
        page_id: Option<Uuid>,
        occurred_at: Timestamp,
        title: &str,
        description: &str,
    ) -> Result<()> {
        if title.trim().is_empty() {
            bail!("a timeline event needs a title");
        }
        self.assert_can_mutate(investigation_id)?;
        let inserted = insert_timeline_event(
            self.connection(),
            TimelineInsert {
                investigation_id,
                occurred_at,
                kind: &TimelineKind::UserEvent,
                title: title.trim(),
                description,
                provenance: Provenance::Analyst,
                source_id: None,
                entity_id: None,
                page_id,
            },
        )?;
        if !inserted {
            bail!("an identical timeline event already exists");
        }
        touch_investigation(self.connection(), investigation_id)?;
        Ok(())
    }

    pub fn delete_timeline_event(&self, event_id: Uuid) -> Result<()> {
        self.assert_can_mutate_record("timeline_events", event_id)?;
        self.connection().execute(
            "DELETE FROM timeline_events WHERE id = ?1",
            params![id(&event_id)],
        )?;
        Ok(())
    }
}
