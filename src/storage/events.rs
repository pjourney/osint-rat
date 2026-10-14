//! First-class Event objects and their entity links.

use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use super::Store;
use super::rows::{RowExt, id, json, opt_id, opt_ts, ts};
use super::{SeqTable, next_seq, touch_investigation};
use crate::model::{Event, Fields, NewEvent, Provenance, TimelineKind, Timestamp, now};

pub(crate) fn insert_event_record(
    connection: &Connection,
    event: &Event,
) -> rusqlite::Result<usize> {
    let inserted = connection.execute(
        "INSERT OR IGNORE INTO events (
            id, investigation_id, seq, kind, title, description, started_at, ended_at,
            provenance, source_id, page_id, fields, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            id(&event.id),
            id(&event.investigation_id),
            event.seq,
            event.kind.as_str(),
            event.title,
            event.description,
            ts(&event.started_at),
            opt_ts(&event.ended_at),
            event.provenance.as_str(),
            opt_id(&event.source_id),
            opt_id(&event.page_id),
            json(&event.fields),
            ts(&event.created_at),
        ],
    )?;
    if inserted > 0 {
        for entity_id in &event.entity_ids {
            link_event_entity(connection, event.id, *entity_id)?;
        }
    }
    Ok(inserted)
}

pub(crate) struct EventInsert<'a> {
    pub investigation_id: Uuid,
    pub started_at: Timestamp,
    pub ended_at: Option<Timestamp>,
    pub kind: &'a TimelineKind,
    pub title: &'a str,
    pub description: &'a str,
    pub provenance: Provenance,
    pub source_id: Option<Uuid>,
    pub page_id: Option<Uuid>,
    pub fields: Fields,
    pub entity_ids: Vec<Uuid>,
}

pub(crate) fn insert_event(
    connection: &Connection,
    input: EventInsert<'_>,
) -> rusqlite::Result<bool> {
    let event = Event {
        id: Uuid::new_v4(),
        investigation_id: input.investigation_id,
        seq: next_seq(connection, SeqTable::Events, input.investigation_id)?,
        kind: input.kind.clone(),
        title: input.title.to_string(),
        description: input.description.to_string(),
        started_at: input.started_at,
        ended_at: input.ended_at,
        provenance: input.provenance,
        source_id: input.source_id,
        page_id: input.page_id,
        fields: input.fields,
        entity_ids: input.entity_ids,
        created_at: now(),
    };
    Ok(insert_event_record(connection, &event)? > 0)
}

pub(crate) fn link_event_entity(
    connection: &Connection,
    event_id: Uuid,
    entity_id: Uuid,
) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT OR IGNORE INTO event_entities (event_id, entity_id) VALUES (?1, ?2)",
        params![id(&event_id), id(&entity_id)],
    )?;
    Ok(())
}

pub(crate) fn list_events(
    connection: &Connection,
    investigation_id: Uuid,
) -> rusqlite::Result<Vec<Event>> {
    let mut statement = connection.prepare(
        "SELECT id, investigation_id, seq, kind, title, description, started_at, ended_at,
                provenance, source_id, page_id, fields, created_at
         FROM events WHERE investigation_id = ?1 ORDER BY started_at, seq",
    )?;
    let mut rows = statement.query(params![id(&investigation_id)])?;
    let mut events = Vec::new();
    while let Some(row) = rows.next()? {
        let event_id = row.uuid(0)?;
        events.push(Event {
            id: event_id,
            investigation_id: row.uuid(1)?,
            seq: row.get(2)?,
            kind: TimelineKind::parse(&row.get::<_, String>(3)?),
            title: row.get(4)?,
            description: row.get(5)?,
            started_at: row.timestamp(6)?,
            ended_at: row.opt_timestamp(7)?,
            provenance: Provenance::parse(&row.get::<_, String>(8)?)
                .unwrap_or(Provenance::Collected),
            source_id: row.opt_uuid(9)?,
            page_id: row.opt_uuid(10)?,
            fields: row.json(11)?,
            entity_ids: list_event_entities(connection, event_id)?,
            created_at: row.timestamp(12)?,
        });
    }
    Ok(events)
}

fn list_event_entities(connection: &Connection, event_id: Uuid) -> rusqlite::Result<Vec<Uuid>> {
    let mut statement = connection
        .prepare("SELECT entity_id FROM event_entities WHERE event_id = ?1 ORDER BY entity_id")?;
    let rows = statement.query_map(params![id(&event_id)], |row| row.uuid(0))?;
    rows.collect()
}

pub(crate) fn entities_for_source(
    connection: &Connection,
    source_id: Uuid,
) -> rusqlite::Result<Vec<Uuid>> {
    let mut statement = connection
        .prepare("SELECT entity_id FROM entity_sources WHERE source_id = ?1 ORDER BY entity_id")?;
    let rows = statement.query_map(params![id(&source_id)], |row| row.uuid(0))?;
    rows.collect()
}

impl Store {
    pub fn add_event(
        &self,
        investigation_id: Uuid,
        input: NewEvent,
        provenance: Provenance,
        entity_ids: Vec<Uuid>,
    ) -> color_eyre::eyre::Result<bool> {
        if input.title.trim().is_empty() {
            color_eyre::eyre::bail!("an event needs a title");
        }
        let inserted = insert_event(
            self.connection(),
            EventInsert {
                investigation_id,
                started_at: input.started_at,
                ended_at: input.ended_at,
                kind: &input.kind,
                title: &input.title,
                description: &input.description,
                provenance,
                source_id: None,
                page_id: None,
                fields: input.fields,
                entity_ids,
            },
        )?;
        if inserted {
            touch_investigation(self.connection(), investigation_id)?;
        }
        Ok(inserted)
    }

    pub fn find_event(&self, event_id: Uuid) -> rusqlite::Result<Option<Event>> {
        self.connection()
            .query_row(
                "SELECT investigation_id FROM events WHERE id = ?1",
                params![id(&event_id)],
                |row| row.uuid(0),
            )
            .optional()?
            .map(|investigation_id| {
                list_events(self.connection(), investigation_id)
                    .map(|events| events.into_iter().find(|event| event.id == event_id))
            })
            .transpose()
            .map(|found| found.flatten())
    }
}
