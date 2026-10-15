//! Entities, relationships, and their links to pages and sources.

use std::collections::HashMap;

use color_eyre::eyre::{Result, WrapErr, bail};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use uuid::Uuid;

use super::rows::{RowExt, id, json, ts};
use super::{SeqTable, Store, next_seq, touch_investigation};
use crate::model::{
    AuditKind, CataloguedLink, Confidence, Entity, EntityKey, EntityKind, Fields, Marking,
    OBJECT_MARKING_FIELD, PageEntityLink, Provenance, Relationship, SearchAroundQuery,
    SearchAroundResult, normalize_relationship_kind, now, search_around,
};

/// Inserts an entity or returns the existing one with the same kind and canonical value.
/// Returns `(id, created)`.
pub(crate) fn upsert_entity(
    connection: &Connection,
    investigation_id: Uuid,
    key: &EntityKey,
    provenance: Provenance,
    label: Option<&str>,
) -> rusqlite::Result<(Uuid, bool)> {
    let existing: Option<String> = connection
        .query_row(
            "SELECT id FROM entities WHERE investigation_id = ?1 AND kind = ?2 AND value = ?3",
            params![id(&investigation_id), key.kind.as_str(), key.value],
            |row| row.get(0),
        )
        .optional()?;
    let timestamp = ts(&now());
    if let Some(existing) = existing {
        connection.execute(
            "UPDATE entities SET updated_at = ?2,
                label = CASE WHEN label = '' AND ?3 IS NOT NULL THEN ?3 ELSE label END
             WHERE id = ?1",
            params![existing, timestamp, label],
        )?;
        let parsed = Uuid::parse_str(&existing).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        return Ok((parsed, false));
    }
    let entity_id = Uuid::new_v4();
    let seq = next_seq(connection, SeqTable::Entities, investigation_id)?;
    connection.execute(
        "INSERT INTO entities (id, investigation_id, seq, kind, value, label, notes, provenance, fields, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, '', ?7, '[]', ?8, ?8)",
        params![
            id(&entity_id),
            id(&investigation_id),
            seq,
            key.kind.as_str(),
            key.value,
            label.unwrap_or_default(),
            provenance.as_str(),
            timestamp,
        ],
    )?;
    Ok((entity_id, true))
}

/// Merges `fields` into an entity's metadata; new values replace existing values for the same key.
pub(crate) fn merge_entity_fields(
    connection: &Connection,
    entity_id: Uuid,
    fields: &Fields,
) -> rusqlite::Result<()> {
    if fields.is_empty() {
        return Ok(());
    }
    let current: String = connection.query_row(
        "SELECT fields FROM entities WHERE id = ?1",
        params![id(&entity_id)],
        |row| row.get(0),
    )?;
    let mut merged: Fields = serde_json::from_str(&current).unwrap_or_default();
    for (key, value) in fields.iter() {
        if let Some(slot) = merged.0.iter_mut().find(|(existing, _)| existing == key) {
            slot.1 = value.to_string();
        } else {
            merged.push(key, value);
        }
    }
    connection.execute(
        "UPDATE entities SET fields = ?2 WHERE id = ?1",
        params![id(&entity_id), json(&merged)],
    )?;
    Ok(())
}

pub(crate) fn link_entity_source(
    connection: &Connection,
    entity_id: Uuid,
    source_id: Uuid,
) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT OR IGNORE INTO entity_sources (entity_id, source_id) VALUES (?1, ?2)",
        params![id(&entity_id), id(&source_id)],
    )?;
    Ok(())
}

pub(crate) fn link_page_entity(
    connection: &Connection,
    page_id: Uuid,
    entity_id: Uuid,
) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT OR IGNORE INTO page_entities (page_id, entity_id) VALUES (?1, ?2)",
        params![id(&page_id), id(&entity_id)],
    )?;
    Ok(())
}

/// Inserts a relationship or returns the existing `(from, kind, to)` edge. Returns `(id, created)`.
pub(crate) fn upsert_relationship(
    connection: &Connection,
    investigation_id: Uuid,
    from_id: Uuid,
    kind: &str,
    to_id: Uuid,
    provenance: Provenance,
    confidence: Confidence,
) -> rusqlite::Result<(Uuid, bool)> {
    let kind = normalize_relationship_kind(kind);
    let existing: Option<String> = connection
        .query_row(
            "SELECT id FROM relationships WHERE from_id = ?1 AND kind = ?2 AND to_id = ?3",
            params![id(&from_id), kind, id(&to_id)],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(existing) = existing {
        let parsed = Uuid::parse_str(&existing).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        return Ok((parsed, false));
    }
    let relationship_id = Uuid::new_v4();
    let seq = next_seq(connection, SeqTable::Relationships, investigation_id)?;
    connection.execute(
        "INSERT INTO relationships (id, investigation_id, seq, from_id, to_id, kind, provenance, confidence, notes, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, '', ?9)",
        params![
            id(&relationship_id),
            id(&investigation_id),
            seq,
            id(&from_id),
            id(&to_id),
            kind,
            provenance.as_str(),
            confidence.as_str(),
            ts(&now()),
        ],
    )?;
    catalogue_link_type(connection, investigation_id, &kind)?;
    Ok((relationship_id, true))
}

/// Records a verb the first time it is used in an investigation. Built-in catalog
/// metadata is copied; unknown snake_case verbs are stored with an empty description.
pub(crate) fn catalogue_link_type(
    connection: &Connection,
    investigation_id: Uuid,
    kind: &str,
) -> rusqlite::Result<()> {
    catalogue_link_type_at(connection, investigation_id, kind, &ts(&now()))
}

fn catalogue_link_type_at(
    connection: &Connection,
    investigation_id: Uuid,
    kind: &str,
    first_seen_at: &str,
) -> rusqlite::Result<()> {
    let recorded = CataloguedLink::from_first_use(kind);
    connection.execute(
        "INSERT OR IGNORE INTO link_types (investigation_id, kind, description, inverse, first_seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            id(&investigation_id),
            recorded.kind,
            recorded.description,
            recorded.inverse,
            first_seen_at,
        ],
    )?;
    Ok(())
}

fn link_catalog_needs_backfill(connection: &Connection) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS (
            SELECT 1 FROM relationships r
            WHERE NOT EXISTS (
                SELECT 1 FROM link_types t
                WHERE t.investigation_id = r.investigation_id AND t.kind = r.kind
            )
         )",
        [],
        |row| row.get::<_, i64>(0).map(|value| value != 0),
    )
}

pub(crate) fn backfill_link_catalog(connection: &mut Connection) -> rusqlite::Result<()> {
    if !link_catalog_needs_backfill(connection)? {
        return Ok(());
    }
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let rows = {
        let mut statement = tx.prepare(
            "SELECT investigation_id, kind, MIN(created_at) FROM relationships GROUP BY investigation_id, kind",
        )?;
        statement
            .query_map([], |row| {
                Ok((
                    row.uuid(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (investigation_id, kind, first_seen_at) in rows {
        catalogue_link_type_at(&tx, investigation_id, &kind, &first_seen_at)?;
    }
    tx.commit()?;
    Ok(())
}

pub(crate) fn list_link_types(
    connection: &Connection,
    investigation_id: Uuid,
) -> rusqlite::Result<Vec<CataloguedLink>> {
    let mut statement = connection.prepare(
        "SELECT kind, description, inverse FROM link_types WHERE investigation_id = ?1 ORDER BY kind",
    )?;
    let rows = statement.query_map(params![id(&investigation_id)], |row| {
        Ok(CataloguedLink {
            kind: row.get(0)?,
            description: row.get(1)?,
            inverse: row.get(2)?,
        })
    })?;
    rows.collect()
}

pub(crate) fn link_relationship_source(
    connection: &Connection,
    relationship_id: Uuid,
    source_id: Uuid,
) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT OR IGNORE INTO relationship_sources (relationship_id, source_id) VALUES (?1, ?2)",
        params![id(&relationship_id), id(&source_id)],
    )?;
    Ok(())
}

/// Loads `(owner, source)` link rows into a map keyed by owner.
pub(crate) fn load_links(
    connection: &Connection,
    sql: &str,
    investigation_id: Uuid,
) -> rusqlite::Result<HashMap<Uuid, Vec<Uuid>>> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map(params![id(&investigation_id)], |row| {
        Ok((row.uuid(0)?, row.uuid(1)?))
    })?;
    let mut links: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for row in rows {
        let (owner, source) = row?;
        links.entry(owner).or_default().push(source);
    }
    Ok(links)
}

pub(crate) fn list_entities(
    connection: &Connection,
    investigation_id: Uuid,
) -> rusqlite::Result<Vec<Entity>> {
    let mut links = load_links(
        connection,
        "SELECT es.entity_id, es.source_id FROM entity_sources es
         JOIN entities e ON e.id = es.entity_id
         JOIN sources s ON s.id = es.source_id
         WHERE e.investigation_id = ?1 ORDER BY s.seq",
        investigation_id,
    )?;
    let mut statement = connection.prepare(
        "SELECT id, investigation_id, seq, kind, value, label, notes, provenance, fields, created_at, updated_at
         FROM entities WHERE investigation_id = ?1 ORDER BY seq",
    )?;
    let rows = statement.query_map(params![id(&investigation_id)], |row| {
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
    })?;
    let mut entities = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    for entity in &mut entities {
        entity.source_ids = links.remove(&entity.id).unwrap_or_default();
    }
    Ok(entities)
}

pub(crate) fn list_relationships(
    connection: &Connection,
    investigation_id: Uuid,
) -> rusqlite::Result<Vec<Relationship>> {
    let mut links = load_links(
        connection,
        "SELECT rs.relationship_id, rs.source_id FROM relationship_sources rs
         JOIN relationships r ON r.id = rs.relationship_id
         JOIN sources s ON s.id = rs.source_id
         WHERE r.investigation_id = ?1 ORDER BY s.seq",
        investigation_id,
    )?;
    let mut statement = connection.prepare(
        "SELECT id, investigation_id, seq, from_id, to_id, kind, provenance, confidence, notes, created_at
         FROM relationships WHERE investigation_id = ?1 ORDER BY seq",
    )?;
    let rows = statement.query_map(params![id(&investigation_id)], |row| {
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
    })?;
    let mut relationships = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    for relationship in &mut relationships {
        relationship.source_ids = links.remove(&relationship.id).unwrap_or_default();
    }
    Ok(relationships)
}

pub(crate) fn list_page_entities(
    connection: &Connection,
    investigation_id: Uuid,
) -> rusqlite::Result<Vec<PageEntityLink>> {
    let mut statement = connection.prepare(
        "SELECT pe.page_id, pe.entity_id FROM page_entities pe
         JOIN pages p ON p.id = pe.page_id
         JOIN entities e ON e.id = pe.entity_id
         WHERE p.investigation_id = ?1 ORDER BY p.seq, e.seq",
    )?;
    let rows = statement.query_map(params![id(&investigation_id)], |row| {
        Ok(PageEntityLink {
            page_id: row.uuid(0)?,
            entity_id: row.uuid(1)?,
        })
    })?;
    rows.collect()
}

impl Store {
    /// Adds an entity manually (or returns the existing one), optionally linking it to a page.
    pub fn add_entity(
        &mut self,
        investigation_id: Uuid,
        kind: EntityKind,
        value: &str,
        provenance: Provenance,
        page_id: Option<Uuid>,
    ) -> Result<Uuid> {
        if value.trim().is_empty() {
            bail!("an entity needs a value");
        }
        self.assert_can_mutate(investigation_id)?;
        let key = EntityKey::new(kind, value);
        let transaction = self.connection_mut().transaction()?;
        let (entity_id, _) = upsert_entity(&transaction, investigation_id, &key, provenance, None)?;
        if let Some(page_id) = page_id {
            link_page_entity(&transaction, page_id, entity_id)?;
        }
        touch_investigation(&transaction, investigation_id)?;
        transaction.commit()?;
        Ok(entity_id)
    }

    pub fn update_entity(&self, entity_id: Uuid, label: &str, notes: &str) -> Result<()> {
        self.assert_can_mutate_record("entities", entity_id)?;
        self.connection().execute(
            "UPDATE entities SET label = ?2, notes = ?3, updated_at = ?4 WHERE id = ?1",
            params![id(&entity_id), label.trim(), notes, ts(&now())],
        )?;
        Ok(())
    }

    pub fn delete_entity(&self, entity_id: Uuid) -> Result<()> {
        self.assert_can_mutate_record("entities", entity_id)?;
        self.connection().execute(
            "DELETE FROM entities WHERE id = ?1",
            params![id(&entity_id)],
        )?;
        Ok(())
    }

    /// Creates (or reuses) a relationship and records its supporting sources.
    #[allow(clippy::too_many_arguments)]
    pub fn link_entities(
        &mut self,
        investigation_id: Uuid,
        from_id: Uuid,
        kind: &str,
        to_id: Uuid,
        provenance: Provenance,
        confidence: Confidence,
        source_ids: &[Uuid],
    ) -> Result<Uuid> {
        if from_id == to_id {
            bail!("an entity cannot be related to itself");
        }
        self.assert_can_mutate(investigation_id)?;
        let transaction = self.connection_mut().transaction()?;
        let (relationship_id, _) = upsert_relationship(
            &transaction,
            investigation_id,
            from_id,
            kind,
            to_id,
            provenance,
            confidence,
        )?;
        for source_id in source_ids {
            link_relationship_source(&transaction, relationship_id, *source_id)?;
        }
        touch_investigation(&transaction, investigation_id)?;
        transaction.commit()?;
        Ok(relationship_id)
    }

    pub fn set_entity_marking(&mut self, entity_id: Uuid, marking: Marking) -> Result<()> {
        let (investigation_id, previous): (Uuid, String) = self.connection().query_row(
            "SELECT investigation_id, fields FROM entities WHERE id = ?1",
            params![id(&entity_id)],
            |row| Ok((row.uuid(0)?, row.get(1)?)),
        )?;
        self.assert_can_mutate(investigation_id)?;
        let mut fields: Fields =
            serde_json::from_str(&previous).wrap_err("entity fields JSON is invalid")?;
        let from = fields
            .get(OBJECT_MARKING_FIELD)
            .and_then(|value| Marking::parse(value).ok())
            .unwrap_or(Marking::Unmarked);
        if from == marking {
            return Ok(());
        }
        if marking == Marking::Unmarked {
            fields.remove(OBJECT_MARKING_FIELD);
        } else {
            fields.set(OBJECT_MARKING_FIELD, marking.as_str());
        }
        let identity = self.identity().clone();
        let transaction = self.connection_mut().transaction()?;
        transaction.execute(
            "UPDATE entities SET fields = ?2, updated_at = ?3 WHERE id = ?1",
            params![id(&entity_id), json(&fields), ts(&now())],
        )?;
        let mut details = Fields::new();
        details.push("from", from.as_str());
        details.push("to", marking.as_str());
        super::audit::append_audit_on(
            &transaction,
            investigation_id,
            &identity,
            AuditKind::MarkingChanged,
            &format!("Object marking {} → {}", from.label(), marking.label()),
            details,
            vec![entity_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn delete_relationship(&self, relationship_id: Uuid) -> Result<()> {
        self.assert_can_mutate_record("relationships", relationship_id)?;
        self.connection().execute(
            "DELETE FROM relationships WHERE id = ?1",
            params![id(&relationship_id)],
        )?;
        Ok(())
    }

    /// Search Around over the stored investigation snapshot (same algorithm as the in-memory path).
    pub fn search_around(
        &self,
        investigation_id: Uuid,
        query: &SearchAroundQuery,
    ) -> Result<SearchAroundResult> {
        let data = self.load_investigation(investigation_id)?;
        Ok(search_around(&data, query))
    }
}
