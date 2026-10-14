use color_eyre::eyre::{Result, bail};
use rusqlite::{Connection, params};
use uuid::Uuid;

use super::graph::{link_page_entity, upsert_entity};
use super::rows::{RowExt, id, opt_id, ts};
use super::{SeqTable, Store, next_seq, touch_investigation};
use crate::model::{EntityKey, Note, Page, PageKind, Provenance, Target, now};

/// Input for creating a page.
#[derive(Clone, Debug)]
pub struct NewPage {
    pub investigation_id: Uuid,
    pub title: String,
    pub kind: PageKind,
    pub target: Option<String>,
    pub description: String,
}

impl NewPage {
    pub fn new(investigation_id: Uuid, title: &str, kind: PageKind, target: Option<&str>) -> Self {
        Self {
            investigation_id,
            title: title.to_string(),
            kind,
            target: target.map(str::to_string),
            description: String::new(),
        }
    }
}

const PAGE_COLUMNS: &str = "id, investigation_id, seq, title, kind, target, description, notes, entity_id, created_at, updated_at";

impl Store {
    /// Creates a page. When the page type has a concrete subject (a domain, an IP, ...), the
    /// target is validated and normalized, and the page's primary entity is created.
    pub fn create_page(&mut self, new_page: NewPage) -> Result<Page> {
        let target = normalize_page_target(&new_page.kind, new_page.target.as_deref())?;
        let title = match new_page.title.trim() {
            "" => target.clone().unwrap_or_default(),
            title => title.to_string(),
        };
        if title.is_empty() {
            bail!("a page needs a title or a target");
        }
        self.assert_can_mutate(new_page.investigation_id)?;
        let transaction = self.connection_mut().transaction()?;
        let timestamp = now();
        let mut page = Page {
            id: Uuid::new_v4(),
            investigation_id: new_page.investigation_id,
            seq: next_seq(&transaction, SeqTable::Pages, new_page.investigation_id)?,
            title,
            kind: new_page.kind,
            target,
            description: new_page.description.trim().to_string(),
            notes: String::new(),
            entity_id: None,
            created_at: timestamp,
            updated_at: timestamp,
        };
        insert_page(&transaction, &page)?;
        if let Some(entity_kind) = page.kind.entity_kind() {
            let value = page.target.clone().unwrap_or_else(|| page.title.clone());
            let key = EntityKey::new(entity_kind, &value);
            let (entity_id, _) = upsert_entity(
                &transaction,
                page.investigation_id,
                &key,
                Provenance::Analyst,
                None,
            )?;
            transaction.execute(
                "UPDATE pages SET entity_id = ?2 WHERE id = ?1",
                params![id(&page.id), id(&entity_id)],
            )?;
            link_page_entity(&transaction, page.id, entity_id)?;
            page.entity_id = Some(entity_id);
        }
        touch_investigation(&transaction, page.investigation_id)?;
        transaction.commit()?;
        Ok(page)
    }

    pub fn update_page(&self, page: &Page) -> Result<()> {
        if page.title.trim().is_empty() {
            bail!("a page needs a title");
        }
        let target = normalize_page_target(&page.kind, page.target.as_deref())?;
        self.connection().execute(
            "UPDATE pages SET title = ?2, kind = ?3, target = ?4, description = ?5, notes = ?6, updated_at = ?7
             WHERE id = ?1",
            params![
                id(&page.id),
                page.title.trim(),
                page.kind.as_str(),
                target,
                page.description,
                page.notes,
                ts(&now()),
            ],
        )?;
        touch_investigation(self.connection(), page.investigation_id)?;
        Ok(())
    }

    pub fn update_page_notes(&self, page_id: Uuid, notes: &str) -> Result<()> {
        self.connection().execute(
            "UPDATE pages SET notes = ?2, updated_at = ?3 WHERE id = ?1",
            params![id(&page_id), notes, ts(&now())],
        )?;
        Ok(())
    }

    pub fn delete_page(&self, page_id: Uuid) -> Result<()> {
        self.connection()
            .execute("DELETE FROM pages WHERE id = ?1", params![id(&page_id)])?;
        Ok(())
    }

    pub fn create_note(
        &self,
        investigation_id: Uuid,
        page_id: Option<Uuid>,
        body: &str,
        provenance: Provenance,
    ) -> Result<Note> {
        if body.trim().is_empty() {
            bail!("a note cannot be empty");
        }
        let timestamp = now();
        let note = Note {
            id: Uuid::new_v4(),
            investigation_id,
            seq: next_seq(self.connection(), SeqTable::Notes, investigation_id)?,
            page_id,
            body: body.trim_end().to_string(),
            provenance,
            created_at: timestamp,
            updated_at: timestamp,
        };
        insert_note(self.connection(), &note)?;
        touch_investigation(self.connection(), investigation_id)?;
        Ok(note)
    }

    pub fn update_note(&self, note_id: Uuid, body: &str) -> Result<()> {
        if body.trim().is_empty() {
            bail!("a note cannot be empty");
        }
        self.connection().execute(
            "UPDATE notes SET body = ?2, updated_at = ?3 WHERE id = ?1",
            params![id(&note_id), body.trim_end(), ts(&now())],
        )?;
        Ok(())
    }

    pub fn delete_note(&self, note_id: Uuid) -> Result<()> {
        self.connection()
            .execute("DELETE FROM notes WHERE id = ?1", params![id(&note_id)])?;
        Ok(())
    }
}

/// Validates and canonicalizes a page target for page kinds that have collectable subjects.
pub fn normalize_page_target(kind: &PageKind, raw: Option<&str>) -> Result<Option<String>> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    match kind.target_kind() {
        Some(target_kind) => match Target::parse(target_kind, raw) {
            Ok(target) => Ok(Some(target.value())),
            Err(error) => bail!("invalid {} target: {error}", kind.label()),
        },
        None => Ok(Some(raw.to_string())),
    }
}

pub(crate) fn insert_page(connection: &Connection, page: &Page) -> rusqlite::Result<()> {
    connection.execute(
        &format!("INSERT INTO pages ({PAGE_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"),
        params![
            id(&page.id),
            id(&page.investigation_id),
            page.seq,
            page.title,
            page.kind.as_str(),
            page.target,
            page.description,
            page.notes,
            opt_id(&page.entity_id),
            ts(&page.created_at),
            ts(&page.updated_at),
        ],
    )?;
    Ok(())
}

pub(crate) fn insert_note(connection: &Connection, note: &Note) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO notes (id, investigation_id, seq, page_id, body, provenance, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id(&note.id),
            id(&note.investigation_id),
            note.seq,
            opt_id(&note.page_id),
            note.body,
            note.provenance.as_str(),
            ts(&note.created_at),
            ts(&note.updated_at),
        ],
    )?;
    Ok(())
}

pub(crate) fn list_pages(
    connection: &Connection,
    investigation_id: Uuid,
) -> rusqlite::Result<Vec<Page>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {PAGE_COLUMNS} FROM pages WHERE investigation_id = ?1 ORDER BY seq"
    ))?;
    let rows = statement.query_map(params![id(&investigation_id)], |row| {
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
    })?;
    rows.collect()
}

pub(crate) fn list_notes(
    connection: &Connection,
    investigation_id: Uuid,
) -> rusqlite::Result<Vec<Note>> {
    let mut statement = connection.prepare(
        "SELECT id, investigation_id, seq, page_id, body, provenance, created_at, updated_at
         FROM notes WHERE investigation_id = ?1 ORDER BY seq",
    )?;
    let rows = statement.query_map(params![id(&investigation_id)], |row| {
        Ok(Note {
            id: row.uuid(0)?,
            investigation_id: row.uuid(1)?,
            seq: row.get(2)?,
            page_id: row.opt_uuid(3)?,
            body: row.get(4)?,
            provenance: Provenance::parse(&row.get::<_, String>(5)?).unwrap_or(Provenance::Analyst),
            created_at: row.timestamp(6)?,
            updated_at: row.timestamp(7)?,
        })
    })?;
    rows.collect()
}
