//! Merge and unresolve. Constituent rows stay; the bag records reversible writes.

use std::collections::HashSet;

use color_eyre::eyre::{Result, bail, eyre};
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use super::evidence::{TimelineInsert, insert_timeline_event};
use super::graph::link_entity_source;
use super::rows::{RowExt, id, json, opt_ts, ts};
use super::{SeqTable, Store, next_seq, touch_investigation};
use crate::model::{
    Entity, EntityKind, LABEL_WRITE_KEY, NOTES_WRITE_KEY, PropertyWrite, Provenance, Resolution,
    ResolutionStatus, TimelineKind, new_resolution, now, plan_property_writes, plan_source_writes,
};

pub(crate) fn list_resolutions(
    connection: &Connection,
    investigation_id: Uuid,
) -> rusqlite::Result<Vec<Resolution>> {
    let mut statement = connection.prepare(
        "SELECT id, investigation_id, seq, canonical_id, constituent_ids, status, reason,
                created_by, created_at, unresolved_by, unresolved_at, property_writes, source_writes
         FROM resolutions WHERE investigation_id = ?1 ORDER BY seq",
    )?;
    let rows = statement.query_map(params![id(&investigation_id)], resolution_from_row)?;
    rows.collect()
}

pub(crate) fn insert_resolution_record(
    connection: &Connection,
    resolution: &Resolution,
) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO resolutions (
            id, investigation_id, seq, canonical_id, constituent_ids, status, reason,
            created_by, created_at, unresolved_by, unresolved_at, property_writes, source_writes
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            id(&resolution.id),
            id(&resolution.investigation_id),
            resolution.seq,
            id(&resolution.canonical_id),
            json(&resolution.constituent_ids),
            resolution.status.as_str(),
            resolution.reason,
            resolution.created_by,
            ts(&resolution.created_at),
            resolution.unresolved_by,
            opt_ts(&resolution.unresolved_at),
            json(&resolution.property_writes),
            json(&resolution.source_writes),
        ],
    )?;
    Ok(())
}

fn resolution_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Resolution> {
    Ok(Resolution {
        id: row.uuid(0)?,
        investigation_id: row.uuid(1)?,
        seq: row.get(2)?,
        canonical_id: row.uuid(3)?,
        constituent_ids: row.json(4)?,
        status: ResolutionStatus::parse(&row.get::<_, String>(5)?),
        reason: row.get(6)?,
        created_by: row.get(7)?,
        created_at: row.timestamp(8)?,
        unresolved_by: row.get(9)?,
        unresolved_at: row.opt_timestamp(10)?,
        property_writes: row.json(11)?,
        source_writes: row.json(12)?,
    })
}

fn load_entity(connection: &Connection, entity_id: Uuid) -> rusqlite::Result<Option<Entity>> {
    let mut statement = connection.prepare(
        "SELECT id, investigation_id, seq, kind, value, label, notes, provenance, fields, created_at, updated_at
         FROM entities WHERE id = ?1",
    )?;
    let entity = statement
        .query_row(params![id(&entity_id)], |row| {
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
        })
        .optional()?;
    let Some(mut entity) = entity else {
        return Ok(None);
    };
    let mut sources = connection
        .prepare("SELECT source_id FROM entity_sources WHERE entity_id = ?1 ORDER BY source_id")?;
    entity.source_ids = sources
        .query_map(params![id(&entity_id)], |row| row.uuid(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(entity))
}

fn apply_property_writes(
    connection: &Connection,
    entity_id: Uuid,
    writes: &[PropertyWrite],
) -> rusqlite::Result<()> {
    if writes.is_empty() {
        return Ok(());
    }
    let mut entity =
        load_entity(connection, entity_id)?.ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)?;
    for write in writes {
        match write.key.as_str() {
            LABEL_WRITE_KEY => entity.label = write.new_value.clone(),
            NOTES_WRITE_KEY => entity.notes = write.new_value.clone(),
            key => entity.fields.set(key, write.new_value.clone()),
        }
    }
    connection.execute(
        "UPDATE entities SET label = ?2, notes = ?3, fields = ?4, updated_at = ?5 WHERE id = ?1",
        params![
            id(&entity_id),
            entity.label,
            entity.notes,
            json(&entity.fields),
            ts(&now()),
        ],
    )?;
    Ok(())
}

fn revert_property_writes(
    connection: &Connection,
    entity_id: Uuid,
    writes: &[PropertyWrite],
    remaining: &[Entity],
) -> rusqlite::Result<()> {
    if writes.is_empty() {
        return Ok(());
    }
    let mut entity =
        load_entity(connection, entity_id)?.ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)?;
    for write in writes.iter().rev() {
        if !cell_matches_write(&entity, write) {
            continue;
        }
        if remaining_holds_property(remaining, write) {
            continue;
        }
        match write.key.as_str() {
            LABEL_WRITE_KEY => {
                entity.label = write.previous.clone().unwrap_or_default();
            }
            NOTES_WRITE_KEY => {
                entity.notes = write.previous.clone().unwrap_or_default();
            }
            key => match &write.previous {
                Some(previous) => entity.fields.set(key, previous.clone()),
                None => {
                    entity.fields.remove(key);
                }
            },
        }
    }
    connection.execute(
        "UPDATE entities SET label = ?2, notes = ?3, fields = ?4, updated_at = ?5 WHERE id = ?1",
        params![
            id(&entity_id),
            entity.label,
            entity.notes,
            json(&entity.fields),
            ts(&now()),
        ],
    )?;
    Ok(())
}

fn cell_matches_write(entity: &Entity, write: &PropertyWrite) -> bool {
    match write.key.as_str() {
        LABEL_WRITE_KEY => entity.label == write.new_value,
        NOTES_WRITE_KEY => entity.notes == write.new_value,
        key => entity.fields.get(key) == Some(write.new_value.as_str()),
    }
}

fn remaining_holds_property(remaining: &[Entity], write: &PropertyWrite) -> bool {
    remaining.iter().any(|entity| match write.key.as_str() {
        LABEL_WRITE_KEY => entity.label == write.new_value,
        NOTES_WRITE_KEY => entity.notes == write.new_value,
        key => entity.fields.get(key) == Some(write.new_value.as_str()),
    })
}

fn remaining_holds_source(remaining: &[Entity], source_id: Uuid) -> bool {
    remaining
        .iter()
        .any(|entity| entity.source_ids.contains(&source_id))
}

fn remaining_constituents(
    connection: &Connection,
    resolution: &Resolution,
    all: &[Resolution],
) -> rusqlite::Result<Vec<Entity>> {
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    for item in all {
        if !item.is_active()
            || item.id == resolution.id
            || item.canonical_id != resolution.canonical_id
        {
            continue;
        }
        for entity_id in item.others() {
            if seen.insert(entity_id) {
                ids.push(entity_id);
            }
        }
    }
    let mut loaded = Vec::new();
    for entity_id in ids {
        if let Some(entity) = load_entity(connection, entity_id)? {
            loaded.push(entity);
        }
    }
    Ok(loaded)
}

fn inherited_sibling_writes(
    resolution: &Resolution,
    all: &[Resolution],
) -> (Vec<PropertyWrite>, Vec<Uuid>) {
    let mut properties = resolution.property_writes.clone();
    let mut sources = resolution.source_writes.clone();
    for item in all {
        if item.id == resolution.id
            || item.is_active()
            || item.canonical_id != resolution.canonical_id
        {
            continue;
        }
        properties.extend(item.property_writes.iter().cloned());
        for source_id in &item.source_writes {
            if !sources.contains(source_id) {
                sources.push(*source_id);
            }
        }
    }
    (properties, sources)
}

fn revert_source_writes(
    connection: &Connection,
    resolution: &Resolution,
    source_writes: &[Uuid],
    remaining: &[Entity],
) -> rusqlite::Result<()> {
    for source_id in source_writes {
        if remaining_holds_source(remaining, *source_id) {
            continue;
        }
        let attached = connection
            .query_row(
                "SELECT 1 FROM entity_sources WHERE entity_id = ?1 AND source_id = ?2",
                params![id(&resolution.canonical_id), id(source_id)],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if !attached {
            continue;
        }
        connection.execute(
            "DELETE FROM entity_sources WHERE entity_id = ?1 AND source_id = ?2",
            params![id(&resolution.canonical_id), id(source_id)],
        )?;
    }
    Ok(())
}

impl Store {
    /// Merges `entity_ids` into `canonical_id`. Constituents stay; writes are audited.
    pub fn resolve_entities(
        &mut self,
        investigation_id: Uuid,
        entity_ids: &[Uuid],
        canonical_id: Uuid,
        reason: &str,
        actor: &str,
    ) -> Result<Resolution> {
        let mut unique = Vec::new();
        for id in entity_ids {
            if !unique.contains(id) {
                unique.push(*id);
            }
        }
        if !unique.contains(&canonical_id) {
            unique.insert(0, canonical_id);
        }
        if unique.len() < 2 {
            bail!("merge needs at least two distinct entities");
        }
        let transaction = self.connection_mut().transaction()?;
        let mut loaded = Vec::new();
        for entity_id in &unique {
            let entity = load_entity(&transaction, *entity_id)?
                .ok_or_else(|| eyre!("unknown entity {entity_id}"))?;
            if entity.investigation_id != investigation_id {
                bail!("entity {} is not in this investigation", entity.short_ref());
            }
            loaded.push(entity);
        }
        let active = list_resolutions(&transaction, investigation_id)?;
        for entity in &loaded {
            if entity.id == canonical_id {
                continue;
            }
            let Some(existing) = active
                .iter()
                .find(|resolution| resolution.is_active() && resolution.contains(entity.id))
            else {
                continue;
            };
            if existing.canonical_id == entity.id || existing.canonical_id == canonical_id {
                continue;
            }
            bail!(
                "{} is already absorbed into {}; unresolve that bag first",
                entity.short_ref(),
                existing.short_ref()
            );
        }
        let canonical = loaded
            .iter()
            .find(|entity| entity.id == canonical_id)
            .cloned()
            .ok_or_else(|| eyre!("canonical entity is missing"))?;
        let others: Vec<&Entity> = loaded
            .iter()
            .filter(|entity| entity.id != canonical_id)
            .collect();
        let property_writes = plan_property_writes(&canonical, &others);
        let source_writes = plan_source_writes(&canonical, &others);
        apply_property_writes(&transaction, canonical_id, &property_writes)?;
        for source_id in &source_writes {
            link_entity_source(&transaction, canonical_id, *source_id)?;
        }
        let seq = next_seq(&transaction, SeqTable::Resolutions, investigation_id)?;
        let resolution = new_resolution(
            investigation_id,
            seq,
            canonical_id,
            unique,
            reason,
            actor,
            property_writes,
            source_writes,
        );
        insert_resolution_record(&transaction, &resolution)?;
        let title = format!(
            "Resolved {} into {}",
            others
                .iter()
                .map(|entity| entity.short_ref().to_string())
                .collect::<Vec<_>>()
                .join(", "),
            canonical.short_ref()
        );
        let description = format!(
            "actor={} constituents={} reason={}",
            actor,
            resolution
                .constituent_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(","),
            reason.trim()
        );
        insert_timeline_event(
            &transaction,
            TimelineInsert {
                investigation_id,
                occurred_at: now(),
                kind: &TimelineKind::EntitiesResolved,
                title: &format!("{} ({})", title, resolution.short_ref()),
                description: &description,
                provenance: Provenance::Analyst,
                source_id: None,
                entity_id: Some(canonical_id),
                page_id: None,
            },
        )?;
        touch_investigation(&transaction, investigation_id)?;
        transaction.commit()?;
        Ok(resolution)
    }

    /// Restores constituent visibility and undoes property/source writes for one bag.
    pub fn unresolve_entities(&mut self, resolution_id: Uuid, actor: &str) -> Result<Resolution> {
        let transaction = self.connection_mut().transaction()?;
        let mut resolution = transaction
            .query_row(
                "SELECT id, investigation_id, seq, canonical_id, constituent_ids, status, reason,
                        created_by, created_at, unresolved_by, unresolved_at, property_writes, source_writes
                 FROM resolutions WHERE id = ?1",
                params![id(&resolution_id)],
                resolution_from_row,
            )
            .optional()?
            .ok_or_else(|| eyre!("resolution {resolution_id} does not exist"))?;
        if !resolution.is_active() {
            bail!(
                "resolution {} is already unresolved",
                resolution.short_ref()
            );
        }
        let all = list_resolutions(&transaction, resolution.investigation_id)?;
        let remaining = remaining_constituents(&transaction, &resolution, &all)?;
        let (property_writes, source_writes) = inherited_sibling_writes(&resolution, &all);
        revert_property_writes(
            &transaction,
            resolution.canonical_id,
            &property_writes,
            &remaining,
        )?;
        revert_source_writes(&transaction, &resolution, &source_writes, &remaining)?;
        let timestamp = now();
        transaction.execute(
            "UPDATE resolutions SET status = ?2, unresolved_by = ?3, unresolved_at = ?4 WHERE id = ?1",
            params![
                id(&resolution_id),
                ResolutionStatus::Unresolved.as_str(),
                actor,
                ts(&timestamp),
            ],
        )?;
        resolution.status = ResolutionStatus::Unresolved;
        resolution.unresolved_by = Some(actor.to_string());
        resolution.unresolved_at = Some(timestamp);
        let title = format!(
            "Unresolved {} from {}",
            resolution.short_ref(),
            resolution
                .others()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let description = format!(
            "actor={} constituents={}",
            actor,
            resolution
                .constituent_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        );
        insert_timeline_event(
            &transaction,
            TimelineInsert {
                investigation_id: resolution.investigation_id,
                occurred_at: timestamp,
                kind: &TimelineKind::EntitiesUnresolved,
                title: &format!("{title} ({})", resolution.id.simple()),
                description: &description,
                provenance: Provenance::Analyst,
                source_id: None,
                entity_id: Some(resolution.canonical_id),
                page_id: None,
            },
        )?;
        touch_investigation(&transaction, resolution.investigation_id)?;
        transaction.commit()?;
        Ok(resolution)
    }

    pub fn list_resolutions(&self, investigation_id: Uuid) -> Result<Vec<Resolution>> {
        Ok(list_resolutions(self.connection(), investigation_id)?)
    }

    /// SQLite foreign-key violations. Empty means no orphaned FKs.
    pub fn foreign_key_violations(&self) -> Result<Vec<String>> {
        let mut statement = self.connection().prepare("PRAGMA foreign_key_check")?;
        let rows = statement.query_map([], |row| {
            let table: String = row.get(0)?;
            let rowid: i64 = row.get(1)?;
            let parent: String = row.get(2)?;
            Ok(format!("{table} row {rowid} -> {parent}"))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
