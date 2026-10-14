//! Playbook runs and watch-set automations. Local tables; not part of the archive.

use color_eyre::eyre::Result;
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use super::activity::{insert_agent_run_record, insert_proposal_record};
use super::apply::apply_batch_on;
use super::evidence::{TimelineInsert, insert_timeline_event};
use super::rows::{RowExt, id, opt_ts, ts};
use super::{ApplyContext, ApplySummary, SeqTable, Store, next_seq, touch_investigation};
use crate::model::{
    AgentAction, AgentRun, Automation, EvidenceBatch, JobStatus, Playbook, PlaybookRun, Proposal,
    ProposalStatus, Provenance, Target, TimelineKind, now,
};

pub const PLAYBOOK_AGENT_NAME: &str = "playbook";

pub struct PlaybookApplyOutcome {
    pub summary: ApplySummary,
    pub digest_changed: bool,
}

/// Inputs for [`Store::apply_playbook_outcome`] besides the batches.
pub struct PlaybookApplyMeta<'a> {
    pub playbook: &'a Playbook,
    pub seed: &'a Target,
    pub automation: bool,
    pub digest: &'a str,
    pub previous_digest: Option<&'a str>,
}

impl Store {
    /// Apply every batch in one transaction together with a playbook_run row and a timeline event.
    pub fn apply_playbook_outcome(
        &mut self,
        context: ApplyContext,
        batches: &[EvidenceBatch],
        meta: PlaybookApplyMeta<'_>,
    ) -> Result<PlaybookApplyOutcome> {
        let playbook = meta.playbook;
        let seed = meta.seed;
        let automation = meta.automation;
        self.assert_can_mutate(context.investigation_id)?;
        let identity = self.identity().clone();
        let transaction = self.connection_mut().transaction()?;
        let mut summary = ApplySummary::default();
        for batch in batches {
            let part = apply_batch_on(&transaction, context, batch)?;
            merge_summary(&mut summary, part);
        }
        let timestamp = now();
        let run = PlaybookRun {
            id: Uuid::new_v4(),
            investigation_id: context.investigation_id,
            seq: next_seq(
                &transaction,
                SeqTable::PlaybookRuns,
                context.investigation_id,
            )?,
            playbook_id: playbook.id.clone(),
            playbook_name: playbook.name.clone(),
            seed_kind: seed.kind().as_str().to_string(),
            seed_value: seed.value(),
            dry_run: false,
            automation,
            status: JobStatus::Succeeded,
            summary: summary.describe(),
            error: None,
            created_at: timestamp,
            finished_at: Some(timestamp),
        };
        insert_playbook_run(&transaction, &run)?;
        let kind = TimelineKind::PlaybookRun;
        let _ = insert_timeline_event(
            &transaction,
            TimelineInsert {
                investigation_id: context.investigation_id,
                occurred_at: timestamp,
                kind: &kind,
                title: &format!("Playbook {}", playbook.id),
                description: &run.summary,
                provenance: Provenance::Analyst,
                source_id: None,
                entity_id: None,
                page_id: context.page_id,
            },
        )?;
        let mut details = crate::model::Fields::new();
        details.push("playbook_id", playbook.id.as_str());
        if automation {
            details.push("automation", "true");
        }
        super::audit::append_audit_on(
            &transaction,
            context.investigation_id,
            &identity,
            crate::model::AuditKind::PlaybookRun,
            &format!("Playbook {}", playbook.id),
            details,
            Vec::new(),
        )?;
        touch_investigation(&transaction, context.investigation_id)?;
        transaction.commit()?;
        let digest_changed = meta
            .previous_digest
            .map(|prior| prior != meta.digest)
            .unwrap_or(summary.changed_observations > 0 || summary.new_sources > 0);
        Ok(PlaybookApplyOutcome {
            summary,
            digest_changed,
        })
    }

    /// Store pending proposals under a synthetic playbook agent run.
    pub fn insert_playbook_proposals(
        &mut self,
        investigation_id: Uuid,
        playbook: &Playbook,
        seed: &Target,
        actions: &[AgentAction],
    ) -> Result<Vec<Proposal>> {
        if actions.is_empty() {
            return Ok(Vec::new());
        }
        self.assert_can_mutate(investigation_id)?;
        let transaction = self.connection_mut().transaction()?;
        let timestamp = now();
        let run = AgentRun {
            id: Uuid::new_v4(),
            investigation_id,
            seq: next_seq(&transaction, SeqTable::AgentRuns, investigation_id)?,
            thread_id: Uuid::new_v4(),
            page_id: None,
            agent_name: PLAYBOOK_AGENT_NAME.into(),
            program: "os-int".into(),
            args: vec![playbook.id.clone(), seed.value()],
            question: format!("playbook run {}", playbook.id),
            response: format!("{} planned proposal(s)", actions.len()),
            status: JobStatus::Succeeded,
            error: None,
            focus: None,
            context_artifact_id: None,
            job_id: None,
            model: None,
            cost_usd: None,
            created_at: timestamp,
            finished_at: Some(timestamp),
        };
        insert_agent_run_record(&transaction, &run)?;
        let mut proposals = Vec::with_capacity(actions.len());
        for action in actions {
            let proposal = Proposal {
                id: Uuid::new_v4(),
                investigation_id,
                agent_run_id: run.id,
                seq: next_seq(&transaction, SeqTable::Proposals, investigation_id)?,
                action: action.clone(),
                status: ProposalStatus::Pending,
                error: None,
                created_at: timestamp,
                decided_at: None,
                decided_by: None,
            };
            insert_proposal_record(&transaction, &proposal)?;
            proposals.push(proposal);
        }
        transaction.commit()?;
        Ok(proposals)
    }

    pub fn upsert_automation(
        &mut self,
        investigation_id: Uuid,
        playbook_id: &str,
        seed: &Target,
        interval_seconds: Option<i64>,
        digest: Option<&str>,
    ) -> Result<Automation> {
        self.assert_can_mutate(investigation_id)?;
        let existing = self.find_automation(investigation_id, playbook_id, &seed.value())?;
        let timestamp = now();
        if let Some(mut row) = existing {
            row.interval_seconds = interval_seconds;
            row.enabled = true;
            row.last_run_at = Some(timestamp);
            if let Some(digest) = digest {
                row.last_digest = Some(digest.to_string());
            }
            self.connection().execute(
                "UPDATE automations SET interval_seconds = ?2, enabled = 1, last_run_at = ?3, last_digest = ?4
                 WHERE id = ?1",
                params![
                    id(&row.id),
                    row.interval_seconds,
                    opt_ts(&row.last_run_at),
                    row.last_digest,
                ],
            )?;
            return Ok(row);
        }
        let automation = Automation {
            id: Uuid::new_v4(),
            investigation_id,
            seq: next_seq(self.connection(), SeqTable::Automations, investigation_id)?,
            playbook_id: playbook_id.to_string(),
            seed_kind: seed.kind().as_str().to_string(),
            seed_value: seed.value(),
            interval_seconds,
            enabled: true,
            last_run_at: Some(timestamp),
            last_digest: digest.map(str::to_string),
            created_at: timestamp,
        };
        self.connection().execute(
            "INSERT INTO automations (id, investigation_id, seq, playbook_id, seed_kind, seed_value, interval_seconds, enabled, last_run_at, last_digest, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9, ?10)",
            params![
                id(&automation.id),
                id(&investigation_id),
                automation.seq,
                automation.playbook_id,
                automation.seed_kind,
                automation.seed_value,
                automation.interval_seconds,
                opt_ts(&automation.last_run_at),
                automation.last_digest,
                ts(&automation.created_at),
            ],
        )?;
        Ok(automation)
    }

    pub fn find_automation(
        &self,
        investigation_id: Uuid,
        playbook_id: &str,
        seed_value: &str,
    ) -> Result<Option<Automation>> {
        let mut statement = self.connection().prepare(
            "SELECT id, investigation_id, seq, playbook_id, seed_kind, seed_value, interval_seconds, enabled, last_run_at, last_digest, created_at
             FROM automations WHERE investigation_id = ?1 AND playbook_id = ?2 AND seed_value = ?3",
        )?;
        let row = statement
            .query_row(
                params![id(&investigation_id), playbook_id, seed_value],
                read_automation,
            )
            .optional()?;
        Ok(row)
    }

    pub fn list_playbook_runs(&self, investigation_id: Uuid) -> Result<Vec<PlaybookRun>> {
        let mut statement = self.connection().prepare(
            "SELECT id, investigation_id, seq, playbook_id, playbook_name, seed_kind, seed_value, dry_run, automation, status, summary, error, created_at, finished_at
             FROM playbook_runs WHERE investigation_id = ?1 ORDER BY seq",
        )?;
        let rows = statement.query_map(params![id(&investigation_id)], |row| {
            Ok(PlaybookRun {
                id: row.uuid(0)?,
                investigation_id: row.uuid(1)?,
                seq: row.get(2)?,
                playbook_id: row.get(3)?,
                playbook_name: row.get(4)?,
                seed_kind: row.get(5)?,
                seed_value: row.get(6)?,
                dry_run: row.get::<_, i64>(7)? != 0,
                automation: row.get::<_, i64>(8)? != 0,
                status: JobStatus::parse(&row.get::<_, String>(9)?),
                summary: row.get(10)?,
                error: row.get(11)?,
                created_at: row.timestamp(12)?,
                finished_at: row.opt_timestamp(13)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }
}

fn insert_playbook_run(connection: &Connection, run: &PlaybookRun) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO playbook_runs (id, investigation_id, seq, playbook_id, playbook_name, seed_kind, seed_value, dry_run, automation, status, summary, error, created_at, finished_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            id(&run.id),
            id(&run.investigation_id),
            run.seq,
            run.playbook_id,
            run.playbook_name,
            run.seed_kind,
            run.seed_value,
            if run.dry_run { 1 } else { 0 },
            if run.automation { 1 } else { 0 },
            run.status.as_str(),
            run.summary,
            run.error,
            ts(&run.created_at),
            opt_ts(&run.finished_at),
        ],
    )?;
    Ok(())
}

fn read_automation(row: &rusqlite::Row<'_>) -> rusqlite::Result<Automation> {
    Ok(Automation {
        id: row.uuid(0)?,
        investigation_id: row.uuid(1)?,
        seq: row.get(2)?,
        playbook_id: row.get(3)?,
        seed_kind: row.get(4)?,
        seed_value: row.get(5)?,
        interval_seconds: row.get(6)?,
        enabled: row.get::<_, i64>(7)? != 0,
        last_run_at: row.opt_timestamp(8)?,
        last_digest: row.get(9)?,
        created_at: row.timestamp(10)?,
    })
}

fn merge_summary(into: &mut ApplySummary, part: ApplySummary) {
    into.source_ids.extend(part.source_ids);
    into.entity_ids.extend(part.entity_ids);
    into.relationship_ids.extend(part.relationship_ids);
    into.finding_ids.extend(part.finding_ids);
    into.deleted_finding_ids.extend(part.deleted_finding_ids);
    into.timeline_ids.extend(part.timeline_ids);
    into.event_ids.extend(part.event_ids);
    into.artifact_ids.extend(part.artifact_ids);
    into.new_sources += part.new_sources;
    into.unchanged_sources += part.unchanged_sources;
    into.new_entities += part.new_entities;
    into.total_entities += part.total_entities;
    into.new_relationships += part.new_relationships;
    into.new_events += part.new_events;
    into.new_artifacts += part.new_artifacts;
    into.changed_observations += part.changed_observations;
    into.new_findings += part.new_findings;
    into.updated_findings += part.updated_findings;
    into.resolved_findings += part.resolved_findings;
}
