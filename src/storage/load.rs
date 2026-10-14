//! Loading a whole investigation and inserting a whole investigation (import).

use color_eyre::eyre::{Result, eyre};
use rusqlite::params;
use uuid::Uuid;

use super::Store;
use super::activity::{
    insert_agent_run_record, insert_job_record, insert_proposal_record, list_agent_runs, list_jobs,
    list_proposals,
};
use super::evidence::{
    insert_artifact_record, insert_finding_record, insert_source_record, insert_timeline_record,
    list_artifacts, list_findings, list_sources, list_timeline,
};
use super::graph::{catalogue_link_type, list_entities, list_page_entities, list_relationships};
use super::pages::{insert_note, insert_page, list_notes, list_pages};
use super::resolution::{insert_resolution_record, list_resolutions};
use super::rows::{id, json, ts};
use crate::model::InvestigationData;

impl Store {
    /// Loads every record of an investigation.
    pub fn load_investigation(&self, investigation_id: Uuid) -> Result<InvestigationData> {
        let investigation = self
            .get_investigation(investigation_id)?
            .ok_or_else(|| eyre!("investigation {investigation_id} does not exist"))?;
        let connection = self.connection();
        Ok(InvestigationData {
            investigation,
            pages: list_pages(connection, investigation_id)?,
            sources: list_sources(connection, investigation_id)?,
            findings: list_findings(connection, investigation_id)?,
            entities: list_entities(connection, investigation_id)?,
            relationships: list_relationships(connection, investigation_id)?,
            notes: list_notes(connection, investigation_id)?,
            artifacts: list_artifacts(connection, investigation_id)?,
            timeline: list_timeline(connection, investigation_id)?,
            jobs: list_jobs(connection, investigation_id)?,
            agent_runs: list_agent_runs(connection, investigation_id)?,
            proposals: list_proposals(connection, investigation_id)?,
            page_entities: list_page_entities(connection, investigation_id)?,
            resolutions: list_resolutions(connection, investigation_id)?,
        })
    }

    /// Inserts a complete investigation with its existing IDs (used by import).
    pub fn insert_investigation_data(&mut self, data: &InvestigationData) -> Result<()> {
        let transaction = self.connection_mut().transaction()?;
        let investigation = &data.investigation;
        transaction.execute(
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
        for entity in &data.entities {
            transaction.execute(
                "INSERT INTO entities (id, investigation_id, seq, kind, value, label, notes, provenance, fields, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    id(&entity.id),
                    id(&entity.investigation_id),
                    entity.seq,
                    entity.kind.as_str(),
                    entity.value,
                    entity.label,
                    entity.notes,
                    entity.provenance.as_str(),
                    json(&entity.fields),
                    ts(&entity.created_at),
                    ts(&entity.updated_at),
                ],
            )?;
        }
        for page in &data.pages {
            insert_page(&transaction, page)?;
        }
        for source in &data.sources {
            insert_source_record(&transaction, source)?;
        }
        for entity in &data.entities {
            for source_id in &entity.source_ids {
                transaction.execute(
                    "INSERT OR IGNORE INTO entity_sources (entity_id, source_id) VALUES (?1, ?2)",
                    params![id(&entity.id), id(source_id)],
                )?;
            }
        }
        for link in &data.page_entities {
            transaction.execute(
                "INSERT OR IGNORE INTO page_entities (page_id, entity_id) VALUES (?1, ?2)",
                params![id(&link.page_id), id(&link.entity_id)],
            )?;
        }
        for relationship in &data.relationships {
            transaction.execute(
                "INSERT INTO relationships (id, investigation_id, seq, from_id, to_id, kind, provenance, confidence, notes, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id(&relationship.id),
                    id(&relationship.investigation_id),
                    relationship.seq,
                    id(&relationship.from_id),
                    id(&relationship.to_id),
                    relationship.kind,
                    relationship.provenance.as_str(),
                    relationship.confidence.as_str(),
                    relationship.notes,
                    ts(&relationship.created_at),
                ],
            )?;
            for source_id in &relationship.source_ids {
                transaction.execute(
                    "INSERT OR IGNORE INTO relationship_sources (relationship_id, source_id) VALUES (?1, ?2)",
                    params![id(&relationship.id), id(source_id)],
                )?;
            }
            catalogue_link_type(
                &transaction,
                relationship.investigation_id,
                &relationship.kind,
            )?;
        }
        for finding in &data.findings {
            insert_finding_record(&transaction, finding)?;
        }
        for note in &data.notes {
            insert_note(&transaction, note)?;
        }
        for artifact in &data.artifacts {
            insert_artifact_record(&transaction, artifact)?;
        }
        for event in &data.timeline {
            insert_timeline_record(&transaction, event)?;
        }
        for job in &data.jobs {
            insert_job_record(&transaction, job)?;
        }
        for run in &data.agent_runs {
            insert_agent_run_record(&transaction, run)?;
        }
        for proposal in &data.proposals {
            insert_proposal_record(&transaction, proposal)?;
        }
        for resolution in &data.resolutions {
            insert_resolution_record(&transaction, resolution)?;
        }
        transaction.commit()?;
        Ok(())
    }
}
