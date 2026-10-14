//! Incremental record loads so a large workspace can patch instead of reloading.

use color_eyre::eyre::Result;
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use super::Store;
use super::apply::ApplySummary;
use super::evidence::SOURCE_COLUMNS;
use super::rows::{RowExt, id};
use crate::model::{
    Confidence, Entity, EntityKind, Finding, Page, PageEntityLink, PageKind, Provenance, RefKind,
    Relationship, ShortRef, Source, SourceKind, WorkspaceDelta,
};

impl Store {
    /// Loads only the records touched by an apply, for an incremental workspace patch.
    pub fn load_apply_delta(
        &self,
        investigation_id: Uuid,
        summary: &ApplySummary,
    ) -> Result<WorkspaceDelta> {
        let connection = self.connection();
        let mut delta = WorkspaceDelta {
            investigation: self.get_investigation(investigation_id)?,
            sources: sources_by_ids(connection, &summary.source_ids)?,
            entities: entities_by_ids(connection, &summary.entity_ids)?,
            relationships: relationships_by_ids(connection, &summary.relationship_ids)?,
            findings: findings_by_ids(connection, &summary.finding_ids)?,
            ..WorkspaceDelta::default()
        };
        if let Some(page_id) = summary.page_id
            && let Some(page) = page_by_id(connection, page_id)?
        {
            delta.page_entities = page_entities_for_page(connection, page_id)?;
            if let Some(entity_id) = page.entity_id
                && !delta.entities.iter().any(|entity| entity.id == entity_id)
                && let Some(entity) = entity_by_id(connection, entity_id)?
            {
                delta.entities.push(entity);
            }
            delta.pages.push(page);
        }
        Ok(delta)
    }

    /// Resolves a short reference (`E-4`) or a UUID to a record id in this investigation.
    pub fn resolve_record_ref(
        &self,
        investigation_id: Uuid,
        raw: &str,
    ) -> Result<Option<(RefKind, Uuid)>> {
        let trimmed = raw.trim();
        if let Ok(uuid) = Uuid::parse_str(trimmed) {
            if let Some(kind) = self.record_kind(investigation_id, uuid)? {
                return Ok(Some((kind, uuid)));
            }
            return Ok(None);
        }
        let Ok(reference) = trimmed.parse::<ShortRef>() else {
            return Ok(None);
        };
        let table = table_for_ref(reference.kind);
        let found: Option<String> = self
            .connection()
            .query_row(
                &format!("SELECT id FROM {table} WHERE investigation_id = ?1 AND seq = ?2"),
                params![id(&investigation_id), reference.seq],
                |row| row.get(0),
            )
            .optional()?;
        match found {
            Some(value) => Ok(Some((reference.kind, Uuid::parse_str(&value)?))),
            None => Ok(None),
        }
    }

    pub fn load_page(&self, page_id: Uuid) -> Result<Option<Page>> {
        Ok(page_by_id(self.connection(), page_id)?)
    }

    pub fn load_entity(&self, entity_id: Uuid) -> Result<Option<Entity>> {
        Ok(entity_by_id(self.connection(), entity_id)?)
    }

    fn record_kind(&self, investigation_id: Uuid, record_id: Uuid) -> Result<Option<RefKind>> {
        for kind in RefKind::ALL {
            let table = table_for_ref(kind);
            let exists: Option<i64> = self
                .connection()
                .query_row(
                    &format!(
                        "SELECT 1 FROM {table} WHERE id = ?1 AND investigation_id = ?2 LIMIT 1"
                    ),
                    params![id(&record_id), id(&investigation_id)],
                    |row| row.get(0),
                )
                .optional()?;
            if exists.is_some() {
                return Ok(Some(kind));
            }
        }
        Ok(None)
    }
}

fn table_for_ref(kind: RefKind) -> &'static str {
    match kind {
        RefKind::Page => "pages",
        RefKind::Source => "sources",
        RefKind::Finding => "findings",
        RefKind::Entity => "entities",
        RefKind::Relationship => "relationships",
        RefKind::Note => "notes",
        RefKind::Artifact => "artifacts",
        RefKind::Timeline => "timeline_events",
        RefKind::Job => "jobs",
        RefKind::AgentRun => "agent_runs",
        RefKind::Proposal => "proposals",
        RefKind::Resolution => "resolutions",
        RefKind::Event => "events",
        RefKind::Audit => "audit_log",
    }
}

fn entities_by_ids(connection: &Connection, ids: &[Uuid]) -> rusqlite::Result<Vec<Entity>> {
    let mut entities = Vec::new();
    for entity_id in ids {
        if let Some(entity) = entity_by_id(connection, *entity_id)? {
            entities.push(entity);
        }
    }
    Ok(entities)
}

fn relationships_by_ids(
    connection: &Connection,
    ids: &[Uuid],
) -> rusqlite::Result<Vec<Relationship>> {
    let mut relationships = Vec::new();
    for relationship_id in ids {
        if let Some(relationship) = relationship_by_id(connection, *relationship_id)? {
            relationships.push(relationship);
        }
    }
    Ok(relationships)
}

fn findings_by_ids(connection: &Connection, ids: &[Uuid]) -> rusqlite::Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for finding_id in ids {
        if let Some(finding) = finding_by_id(connection, *finding_id)? {
            findings.push(finding);
        }
    }
    Ok(findings)
}

fn sources_by_ids(connection: &Connection, ids: &[Uuid]) -> rusqlite::Result<Vec<Source>> {
    let mut sources = Vec::new();
    for source_id in ids {
        if let Some(source) = source_by_id(connection, *source_id)? {
            sources.push(source);
        }
    }
    Ok(sources)
}

fn entity_by_id(connection: &Connection, entity_id: Uuid) -> rusqlite::Result<Option<Entity>> {
    let mut entity = connection
        .query_row(
            "SELECT id, investigation_id, seq, kind, value, label, notes, provenance, fields, created_at, updated_at
             FROM entities WHERE id = ?1",
            params![id(&entity_id)],
            |row| {
                Ok(Entity {
                    id: row.uuid(0)?,
                    investigation_id: row.uuid(1)?,
                    seq: row.get(2)?,
                    kind: EntityKind::parse(&row.get::<_, String>(3)?),
                    value: row.get(4)?,
                    label: row.get(5)?,
                    notes: row.get(6)?,
                    provenance: Provenance::parse(&row.get::<_, String>(7)?)
                        .unwrap_or(Provenance::Collected),
                    fields: row.json(8)?,
                    source_ids: Vec::new(),
                    created_at: row.timestamp(9)?,
                    updated_at: row.timestamp(10)?,
                })
            },
        )
        .optional()?;
    if let Some(entity) = entity.as_mut() {
        entity.source_ids = linked_ids(
            connection,
            "SELECT es.source_id FROM entity_sources es
             JOIN sources s ON s.id = es.source_id
             WHERE es.entity_id = ?1 ORDER BY s.seq",
            entity_id,
        )?;
    }
    Ok(entity)
}

fn relationship_by_id(
    connection: &Connection,
    relationship_id: Uuid,
) -> rusqlite::Result<Option<Relationship>> {
    let mut relationship = connection
        .query_row(
            "SELECT id, investigation_id, seq, from_id, to_id, kind, provenance, confidence, notes, created_at
             FROM relationships WHERE id = ?1",
            params![id(&relationship_id)],
            |row| {
                Ok(Relationship {
                    id: row.uuid(0)?,
                    investigation_id: row.uuid(1)?,
                    seq: row.get(2)?,
                    from_id: row.uuid(3)?,
                    to_id: row.uuid(4)?,
                    kind: row.get(5)?,
                    provenance: Provenance::parse(&row.get::<_, String>(6)?)
                        .unwrap_or(Provenance::Collected),
                    confidence: Confidence::parse(&row.get::<_, String>(7)?).unwrap_or_default(),
                    notes: row.get(8)?,
                    source_ids: Vec::new(),
                    created_at: row.timestamp(9)?,
                })
            },
        )
        .optional()?;
    if let Some(relationship) = relationship.as_mut() {
        relationship.source_ids = linked_ids(
            connection,
            "SELECT rs.source_id FROM relationship_sources rs
             JOIN sources s ON s.id = rs.source_id
             WHERE rs.relationship_id = ?1 ORDER BY s.seq",
            relationship_id,
        )?;
    }
    Ok(relationship)
}

fn finding_by_id(connection: &Connection, finding_id: Uuid) -> rusqlite::Result<Option<Finding>> {
    let mut finding = connection
        .query_row(
            "SELECT id, investigation_id, seq, page_id, title, details, confidence, provenance, created_at, updated_at
             FROM findings WHERE id = ?1",
            params![id(&finding_id)],
            |row| {
                Ok(Finding {
                    id: row.uuid(0)?,
                    investigation_id: row.uuid(1)?,
                    seq: row.get(2)?,
                    page_id: row.opt_uuid(3)?,
                    title: row.get(4)?,
                    details: row.get(5)?,
                    confidence: Confidence::parse(&row.get::<_, String>(6)?).unwrap_or_default(),
                    provenance: Provenance::parse(&row.get::<_, String>(7)?)
                        .unwrap_or(Provenance::Analyst),
                    source_ids: Vec::new(),
                    created_at: row.timestamp(8)?,
                    updated_at: row.timestamp(9)?,
                })
            },
        )
        .optional()?;
    if let Some(finding) = finding.as_mut() {
        finding.source_ids = linked_ids(
            connection,
            "SELECT fs.source_id FROM finding_sources fs
             JOIN sources s ON s.id = fs.source_id
             WHERE fs.finding_id = ?1 ORDER BY s.seq",
            finding_id,
        )?;
    }
    Ok(finding)
}

fn source_by_id(connection: &Connection, source_id: Uuid) -> rusqlite::Result<Option<Source>> {
    connection
        .query_row(
            &format!("SELECT {SOURCE_COLUMNS} FROM sources WHERE id = ?1"),
            params![id(&source_id)],
            |row| {
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
            },
        )
        .optional()
}

fn page_by_id(connection: &Connection, page_id: Uuid) -> rusqlite::Result<Option<Page>> {
    connection
        .query_row(
            "SELECT id, investigation_id, seq, title, kind, target, description, notes, entity_id, created_at, updated_at
             FROM pages WHERE id = ?1",
            params![id(&page_id)],
            |row| {
                Ok(Page {
                    id: row.uuid(0)?,
                    investigation_id: row.uuid(1)?,
                    seq: row.get(2)?,
                    title: row.get(3)?,
                    kind: PageKind::parse(&row.get::<_, String>(4)?),
                    target: row.get(5)?,
                    description: row.get(6)?,
                    notes: row.get(7)?,
                    entity_id: row.opt_uuid(8)?,
                    created_at: row.timestamp(9)?,
                    updated_at: row.timestamp(10)?,
                })
            },
        )
        .optional()
}

fn page_entities_for_page(
    connection: &Connection,
    page_id: Uuid,
) -> rusqlite::Result<Vec<PageEntityLink>> {
    let mut statement =
        connection.prepare("SELECT page_id, entity_id FROM page_entities WHERE page_id = ?1")?;
    let rows = statement.query_map(params![id(&page_id)], |row| {
        Ok(PageEntityLink {
            page_id: row.uuid(0)?,
            entity_id: row.uuid(1)?,
        })
    })?;
    rows.collect()
}

fn linked_ids(connection: &Connection, sql: &str, owner_id: Uuid) -> rusqlite::Result<Vec<Uuid>> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map(params![id(&owner_id)], |row| row.uuid(0))?;
    rows.collect()
}
