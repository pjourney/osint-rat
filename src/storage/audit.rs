//! Append-only audit log. Rows are never updated.

use color_eyre::eyre::Result;
use rusqlite::{Connection, params};
use uuid::Uuid;

use super::rows::{RowExt, id, json, ts};
use super::{SeqTable, Store, next_seq, touch_investigation};
use crate::model::{AuditEntry, AuditKind, Fields, Identity};

const COLUMNS: &str =
    "id, investigation_id, seq, kind, actor, machine, occurred_at, summary, details, target_ids";

pub(crate) fn insert_audit_record(
    connection: &Connection,
    entry: &AuditEntry,
) -> rusqlite::Result<()> {
    connection.execute(
        &format!(
            "INSERT INTO audit_log ({COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
        ),
        params![
            id(&entry.id),
            id(&entry.investigation_id),
            entry.seq,
            entry.kind.as_str(),
            entry.actor,
            entry.machine,
            ts(&entry.occurred_at),
            entry.summary,
            json(&entry.details),
            json(&entry.target_ids),
        ],
    )?;
    Ok(())
}

pub(crate) fn append_audit_on(
    connection: &Connection,
    investigation_id: Uuid,
    identity: &Identity,
    kind: AuditKind,
    summary: &str,
    details: Fields,
    target_ids: Vec<Uuid>,
) -> rusqlite::Result<AuditEntry> {
    let seq = next_seq(connection, SeqTable::Audit, investigation_id)?;
    let entry = AuditEntry::new(
        investigation_id,
        seq,
        identity,
        kind,
        summary,
        details,
        target_ids,
    );
    insert_audit_record(connection, &entry)?;
    touch_investigation(connection, investigation_id)?;
    Ok(entry)
}

pub(crate) fn list_audit(
    connection: &Connection,
    investigation_id: Uuid,
) -> rusqlite::Result<Vec<AuditEntry>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {COLUMNS} FROM audit_log WHERE investigation_id = ?1 ORDER BY seq"
    ))?;
    let rows = statement.query_map(params![id(&investigation_id)], audit_from_row)?;
    rows.collect()
}

fn audit_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditEntry> {
    let kind_key: String = row.get(3)?;
    let kind = AuditKind::parse(&kind_key).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(error)),
        )
    })?;
    Ok(AuditEntry {
        id: row.uuid(0)?,
        investigation_id: row.uuid(1)?,
        seq: row.get(2)?,
        kind,
        actor: row.get(4)?,
        machine: row.get(5)?,
        occurred_at: row.timestamp(6)?,
        summary: row.get(7)?,
        details: row.json(8)?,
        target_ids: row.json(9)?,
    })
}

impl Store {
    /// Appends one audit row. There is no update API.
    pub fn append_audit(
        &mut self,
        investigation_id: Uuid,
        kind: AuditKind,
        summary: &str,
        details: Fields,
        target_ids: Vec<Uuid>,
    ) -> Result<AuditEntry> {
        self.assert_can_mutate(investigation_id)?;
        let identity = self.identity.clone();
        let transaction = self.connection_mut().transaction()?;
        let entry = append_audit_on(
            &transaction,
            investigation_id,
            &identity,
            kind,
            summary,
            details,
            target_ids,
        )?;
        transaction.commit()?;
        Ok(entry)
    }

    pub fn list_audit(&self, investigation_id: Uuid) -> Result<Vec<AuditEntry>> {
        Ok(list_audit(self.connection(), investigation_id)?)
    }

    /// Records an export. Readers may export without writing an audit row.
    pub fn record_export(&mut self, investigation_id: Uuid, destination: &str) -> Result<()> {
        if !self.role_for(investigation_id)?.can_write() {
            return Ok(());
        }
        let identity = self.identity.clone();
        let mut details = Fields::new();
        details.push("destination", destination);
        let transaction = self.connection_mut().transaction()?;
        append_audit_on(
            &transaction,
            investigation_id,
            &identity,
            AuditKind::Export,
            &format!("Exported investigation to {destination}"),
            details,
            Vec::new(),
        )?;
        transaction.commit()?;
        Ok(())
    }
}
