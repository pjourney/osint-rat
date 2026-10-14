//! Persistent storage in SQLite.
//!
//! `Store` is deliberately synchronous: every operation is a small indexed query or a short
//! transaction. Large payloads (snapshots, files, reports) live on disk in the investigation
//! directory and are referenced by relative path.

mod activity;
mod apply;
mod evidence;
mod graph;
mod load;
pub mod migrations;
mod pages;
mod resolution;
mod rows;

use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, WrapErr, bail, eyre};
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

pub use apply::{ApplyContext, ApplySummary};
pub use evidence::{NewFinding, NewSourceRecord};
pub use pages::NewPage;
use rows::{RowExt, id, json, ts};

use crate::model::{CataloguedLink, Investigation, InvestigationSummary, now};
use graph::{backfill_link_catalog, list_link_types};

const BUSY_TIMEOUT_MS: u64 = 5_000;
const MIN_ID_PREFIX_LENGTH: usize = 4;
pub const GLOBAL_SCOPE: &str = "global";

/// Tables that carry a per-investigation sequence number.
#[derive(Clone, Copy, Debug)]
pub(crate) enum SeqTable {
    Pages,
    Sources,
    Findings,
    Entities,
    Relationships,
    Notes,
    Artifacts,
    Timeline,
    AgentRuns,
    Proposals,
    Resolutions,
}

impl SeqTable {
    fn name(self) -> &'static str {
        match self {
            SeqTable::Pages => "pages",
            SeqTable::Sources => "sources",
            SeqTable::Findings => "findings",
            SeqTable::Entities => "entities",
            SeqTable::Relationships => "relationships",
            SeqTable::Notes => "notes",
            SeqTable::Artifacts => "artifacts",
            SeqTable::Timeline => "timeline_events",
            SeqTable::AgentRuns => "agent_runs",
            SeqTable::Proposals => "proposals",
            SeqTable::Resolutions => "resolutions",
        }
    }
}

pub(crate) fn next_seq(
    connection: &Connection,
    table: SeqTable,
    investigation_id: Uuid,
) -> rusqlite::Result<i64> {
    let sql = format!(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM {} WHERE investigation_id = ?1",
        table.name()
    );
    connection.query_row(&sql, params![id(&investigation_id)], |row| row.get(0))
}

pub(crate) fn touch_investigation(
    connection: &Connection,
    investigation_id: Uuid,
) -> rusqlite::Result<()> {
    connection.execute(
        "UPDATE investigations SET updated_at = ?2 WHERE id = ?1",
        params![id(&investigation_id), ts(&now())],
    )?;
    Ok(())
}

/// A synchronous SQLite connection plus the migrations applied at open.
///
/// Callers on the UI task use this directly. Collectors never see a `Store`. They return a
/// batch, and `Store::apply_batch` writes it in one transaction.
pub struct Store {
    connection: Connection,
    path: Option<PathBuf>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Store")
            .field("path", &self.path)
            .finish()
    }
}

impl Store {
    /// Opens (creating if necessary) the database at `path` and applies migrations.
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .wrap_err_with(|| format!("could not create {}", parent.display()))?;
        }
        let connection = Connection::open(path)
            .wrap_err_with(|| format!("could not open database {}", path.display()))?;
        Store::initialize(connection, Some(path.to_path_buf()))
    }

    pub fn open_in_memory() -> Result<Store> {
        let connection =
            Connection::open_in_memory().wrap_err("could not open in-memory database")?;
        Store::initialize(connection, None)
    }

    fn initialize(mut connection: Connection, path: Option<PathBuf>) -> Result<Store> {
        connection
            .busy_timeout(std::time::Duration::from_millis(BUSY_TIMEOUT_MS))
            .wrap_err("could not configure database")?;
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;",
            )
            .wrap_err("could not configure database")?;
        migrations::migrate(&mut connection).wrap_err("database migration failed")?;
        backfill_link_catalog(&connection).wrap_err("could not catalogue existing link types")?;
        Ok(Store { connection, path })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }

    pub(crate) fn connection_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(migrations::current_version(&self.connection)?)
    }

    /// Link verbs seen in this investigation, including custom snake_case first-use entries.
    pub fn list_link_types(&self, investigation_id: Uuid) -> Result<Vec<CataloguedLink>> {
        Ok(list_link_types(self.connection(), investigation_id)?)
    }

    /// Runs SQLite's integrity check; returns `"ok"` when healthy.
    pub fn integrity_check(&self) -> Result<String> {
        let result: String = self
            .connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        Ok(result)
    }

    pub fn create_investigation(&self, name: &str, description: &str) -> Result<Investigation> {
        let name = name.trim();
        if name.is_empty() {
            bail!("an investigation needs a name");
        }
        if self.find_investigation_by_name(name)?.is_some() {
            bail!("an investigation named {name:?} already exists");
        }
        let timestamp = now();
        let investigation = Investigation {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: description.trim().to_string(),
            tags: Vec::new(),
            created_at: timestamp,
            updated_at: timestamp,
        };
        self.insert_investigation(&self.connection, &investigation)?;
        Ok(investigation)
    }

    pub(crate) fn insert_investigation(
        &self,
        connection: &Connection,
        investigation: &Investigation,
    ) -> Result<()> {
        connection.execute(
            "INSERT INTO investigations (id, name, description, tags, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id(&investigation.id),
                investigation.name,
                investigation.description,
                json(&investigation.tags),
                ts(&investigation.created_at),
                ts(&investigation.updated_at),
            ],
        )?;
        Ok(())
    }

    pub fn list_investigations(&self) -> Result<Vec<InvestigationSummary>> {
        let mut statement = self.connection.prepare(
            "SELECT i.id, i.name, i.description, i.tags, i.created_at, i.updated_at,
                (SELECT COUNT(*) FROM pages WHERE investigation_id = i.id),
                (SELECT COUNT(*) FROM sources WHERE investigation_id = i.id),
                (SELECT COUNT(*) FROM findings WHERE investigation_id = i.id),
                (SELECT COUNT(*) FROM entities WHERE investigation_id = i.id)
             FROM investigations i ORDER BY i.updated_at DESC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(InvestigationSummary {
                investigation: investigation_from_row(row)?,
                page_count: row.get(6)?,
                source_count: row.get(7)?,
                finding_count: row.get(8)?,
                entity_count: row.get(9)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_investigation(&self, investigation_id: Uuid) -> Result<Option<Investigation>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id, name, description, tags, created_at, updated_at FROM investigations WHERE id = ?1",
                params![id(&investigation_id)],
                investigation_from_row,
            )
            .optional()?)
    }

    fn find_investigation_by_name(&self, name: &str) -> Result<Option<Investigation>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id, name, description, tags, created_at, updated_at FROM investigations
                 WHERE name = ?1 COLLATE NOCASE",
                params![name.trim()],
                investigation_from_row,
            )
            .optional()?)
    }

    /// Finds an investigation by exact name (case-insensitive), UUID, or unique UUID prefix.
    pub fn find_investigation(&self, query: &str) -> Result<Option<Investigation>> {
        let query = query.trim();
        if let Some(found) = self.find_investigation_by_name(query)? {
            return Ok(Some(found));
        }
        if let Ok(uuid) = Uuid::parse_str(query) {
            return self.get_investigation(uuid);
        }
        if query.len() < MIN_ID_PREFIX_LENGTH
            || !query.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        {
            return Ok(None);
        }
        let mut statement = self.connection.prepare(
            "SELECT id, name, description, tags, created_at, updated_at FROM investigations WHERE id LIKE ?1",
        )?;
        let pattern = format!("{}%", query.to_ascii_lowercase());
        let matches = statement
            .query_map(params![pattern], investigation_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        match matches.len() {
            0 => Ok(None),
            1 => Ok(matches.into_iter().next()),
            _ => Err(eyre!(
                "{query:?} matches several investigations; use more characters"
            )),
        }
    }

    /// Like `find_investigation`, but returns a helpful error when nothing matches.
    pub fn require_investigation(&self, query: &str) -> Result<Investigation> {
        self.find_investigation(query)?.ok_or_else(|| {
            eyre!(
                "no investigation named {query:?}; run `os-int list` to see existing investigations"
            )
        })
    }

    pub fn update_investigation(&self, investigation: &Investigation) -> Result<()> {
        let name = investigation.name.trim();
        if name.is_empty() {
            bail!("an investigation needs a name");
        }
        if let Some(existing) = self.find_investigation_by_name(name)?
            && existing.id != investigation.id
        {
            bail!("an investigation named {name:?} already exists");
        }
        self.connection.execute(
            "UPDATE investigations SET name = ?2, description = ?3, tags = ?4, updated_at = ?5 WHERE id = ?1",
            params![
                id(&investigation.id),
                name,
                investigation.description,
                json(&investigation.tags),
                ts(&now()),
            ],
        )?;
        Ok(())
    }

    pub fn delete_investigation(&self, investigation_id: Uuid) -> Result<()> {
        self.connection.execute(
            "DELETE FROM investigations WHERE id = ?1",
            params![id(&investigation_id)],
        )?;
        self.connection.execute(
            "DELETE FROM ui_state WHERE scope = ?1",
            params![id(&investigation_id)],
        )?;
        Ok(())
    }

    pub fn get_state(&self, scope: &str, key: &str) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT value FROM ui_state WHERE scope = ?1 AND key = ?2",
                params![scope, key],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn set_state(&self, scope: &str, key: &str, value: &str) -> Result<()> {
        self.connection.execute(
            "INSERT INTO ui_state (scope, key, value) VALUES (?1, ?2, ?3)
             ON CONFLICT (scope, key) DO UPDATE SET value = excluded.value",
            params![scope, key, value],
        )?;
        Ok(())
    }
}

fn investigation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Investigation> {
    Ok(Investigation {
        id: row.uuid(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        tags: row.json(3)?,
        created_at: row.timestamp(4)?,
        updated_at: row.timestamp(5)?,
    })
}

#[cfg(test)]
mod tests;
