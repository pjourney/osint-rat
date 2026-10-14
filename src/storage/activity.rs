//! Job history, agent runs, and agent proposals.

use color_eyre::eyre::Result;
use rusqlite::{Connection, params};
use uuid::Uuid;

use super::rows::{RowExt, id, json, opt_id, opt_ts, ts};
use super::{SeqTable, Store, next_seq};
use crate::model::{AgentAction, AgentRun, JobRecord, JobStatus, Proposal, ProposalStatus, now};

const JOB_COLUMNS: &str = "id, investigation_id, seq, kind, label, description, status, program, args, created_at, \
     started_at, finished_at, progress, log, stderr, result, error, page_id";
const RUN_COLUMNS: &str = "id, investigation_id, seq, thread_id, page_id, agent_name, program, args, question, response, \
     status, error, focus, context_artifact_id, job_id, model, cost_usd, created_at, finished_at";
const INTERRUPTED_MESSAGE: &str = "interrupted: os-int exited while this job was running";

fn next_job_seq(connection: &Connection, investigation_id: Option<Uuid>) -> rusqlite::Result<i64> {
    connection.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM jobs WHERE investigation_id IS ?1",
        params![opt_id(&investigation_id)],
        |row| row.get(0),
    )
}

pub(crate) fn insert_job_record(connection: &Connection, job: &JobRecord) -> rusqlite::Result<()> {
    connection.execute(
        &format!(
            "INSERT INTO jobs ({JOB_COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)"
        ),
        params![
            id(&job.id),
            opt_id(&job.investigation_id),
            job.seq,
            job.kind,
            job.label,
            job.description,
            job.status.as_str(),
            job.program,
            json(&job.args),
            ts(&job.created_at),
            opt_ts(&job.started_at),
            opt_ts(&job.finished_at),
            job.progress,
            job.log,
            job.stderr,
            job.result,
            job.error,
            opt_id(&job.page_id),
        ],
    )?;
    Ok(())
}

pub(crate) fn list_jobs(
    connection: &Connection,
    investigation_id: Uuid,
) -> rusqlite::Result<Vec<JobRecord>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {JOB_COLUMNS} FROM jobs WHERE investigation_id = ?1 ORDER BY seq"
    ))?;
    let rows = statement.query_map(params![id(&investigation_id)], |row| {
        Ok(JobRecord {
            id: row.uuid(0)?,
            investigation_id: row.opt_uuid(1)?,
            seq: row.get(2)?,
            kind: row.get(3)?,
            label: row.get(4)?,
            description: row.get(5)?,
            status: JobStatus::parse(&row.get::<_, String>(6)?),
            program: row.get(7)?,
            args: row.json(8)?,
            created_at: row.timestamp(9)?,
            started_at: row.opt_timestamp(10)?,
            finished_at: row.opt_timestamp(11)?,
            progress: row.get(12)?,
            log: row.get(13)?,
            stderr: row.get(14)?,
            result: row.get(15)?,
            error: row.get(16)?,
            page_id: row.opt_uuid(17)?,
        })
    })?;
    rows.collect()
}

pub(crate) fn insert_agent_run_record(
    connection: &Connection,
    run: &AgentRun,
) -> rusqlite::Result<()> {
    connection.execute(
        &format!(
            "INSERT INTO agent_runs ({RUN_COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)"
        ),
        params![
            id(&run.id),
            id(&run.investigation_id),
            run.seq,
            id(&run.thread_id),
            opt_id(&run.page_id),
            run.agent_name,
            run.program,
            json(&run.args),
            run.question,
            run.response,
            run.status.as_str(),
            run.error,
            run.focus,
            opt_id(&run.context_artifact_id),
            opt_id(&run.job_id),
            run.model,
            run.cost_usd,
            ts(&run.created_at),
            opt_ts(&run.finished_at),
        ],
    )?;
    Ok(())
}

pub(crate) fn list_agent_runs(
    connection: &Connection,
    investigation_id: Uuid,
) -> rusqlite::Result<Vec<AgentRun>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {RUN_COLUMNS} FROM agent_runs WHERE investigation_id = ?1 ORDER BY seq"
    ))?;
    let rows = statement.query_map(params![id(&investigation_id)], |row| {
        Ok(AgentRun {
            id: row.uuid(0)?,
            investigation_id: row.uuid(1)?,
            seq: row.get(2)?,
            thread_id: row.uuid(3)?,
            page_id: row.opt_uuid(4)?,
            agent_name: row.get(5)?,
            program: row.get(6)?,
            args: row.json(7)?,
            question: row.get(8)?,
            response: row.get(9)?,
            status: JobStatus::parse(&row.get::<_, String>(10)?),
            error: row.get(11)?,
            focus: row.get(12)?,
            context_artifact_id: row.opt_uuid(13)?,
            job_id: row.opt_uuid(14)?,
            model: row.get(15)?,
            cost_usd: row.get(16)?,
            created_at: row.timestamp(17)?,
            finished_at: row.opt_timestamp(18)?,
        })
    })?;
    rows.collect()
}

pub(crate) fn insert_proposal_record(
    connection: &Connection,
    proposal: &Proposal,
) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO proposals (id, investigation_id, agent_run_id, seq, action, status, error, created_at, decided_at, decided_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id(&proposal.id),
            id(&proposal.investigation_id),
            id(&proposal.agent_run_id),
            proposal.seq,
            json(&proposal.action),
            proposal.status.as_str(),
            proposal.error,
            ts(&proposal.created_at),
            opt_ts(&proposal.decided_at),
            proposal.decided_by,
        ],
    )?;
    Ok(())
}

pub(crate) fn list_proposals(
    connection: &Connection,
    investigation_id: Uuid,
) -> rusqlite::Result<Vec<Proposal>> {
    let mut statement = connection.prepare(
        "SELECT id, investigation_id, agent_run_id, seq, action, status, error, created_at, decided_at, decided_by
         FROM proposals WHERE investigation_id = ?1 ORDER BY seq",
    )?;
    let rows = statement.query_map(params![id(&investigation_id)], |row| {
        Ok(Proposal {
            id: row.uuid(0)?,
            investigation_id: row.uuid(1)?,
            agent_run_id: row.uuid(2)?,
            seq: row.get(3)?,
            action: row.json(4)?,
            status: ProposalStatus::parse(&row.get::<_, String>(5)?),
            error: row.get(6)?,
            created_at: row.timestamp(7)?,
            decided_at: row.opt_timestamp(8)?,
            decided_by: row.get(9)?,
        })
    })?;
    rows.collect()
}

impl Store {
    /// Persists a new job, assigning its sequence number.
    pub fn insert_job(&self, job: &mut JobRecord) -> Result<()> {
        if let Some(investigation_id) = job.investigation_id {
            self.assert_can_mutate(investigation_id)?;
        }
        job.seq = next_job_seq(self.connection(), job.investigation_id)?;
        insert_job_record(self.connection(), job)?;
        Ok(())
    }

    /// Count of playbook jobs still queued or running (CLI concurrent cap).
    pub fn active_playbook_job_count(&self) -> Result<usize> {
        let count: i64 = self.connection().query_row(
            "SELECT COUNT(*) FROM jobs WHERE kind LIKE 'playbook:%' AND status IN ('queued', 'running')",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn update_job(&self, job: &JobRecord) -> Result<()> {
        if let Some(investigation_id) = job.investigation_id {
            self.assert_can_mutate(investigation_id)?;
        }
        self.connection().execute(
            "UPDATE jobs SET status = ?2, program = ?3, args = ?4, started_at = ?5, finished_at = ?6,
                progress = ?7, log = ?8, stderr = ?9, result = ?10, error = ?11
             WHERE id = ?1",
            params![
                id(&job.id),
                job.status.as_str(),
                job.program,
                json(&job.args),
                opt_ts(&job.started_at),
                opt_ts(&job.finished_at),
                job.progress,
                job.log,
                job.stderr,
                job.result,
                job.error,
            ],
        )?;
        Ok(())
    }

    /// Marks jobs left running by a previous process as failed. Returns how many were updated.
    ///
    /// Jobs with no investigation (process-local) are always recovered. Jobs on a case are
    /// recovered only when the current identity can mutate that case, so a reader opening a
    /// shared data directory cannot rewrite another cell's rows.
    pub fn mark_interrupted_jobs(&self) -> Result<usize> {
        let timestamp = ts(&now());
        let mut updated_jobs = self.connection().execute(
            "UPDATE jobs SET status = 'failed', error = ?1, finished_at = ?2
             WHERE status IN ('queued', 'running') AND investigation_id IS NULL",
            params![INTERRUPTED_MESSAGE, timestamp],
        )?;
        for summary in self.list_investigations()? {
            if !self.role_for(summary.investigation.id)?.can_write() {
                continue;
            }
            updated_jobs += self.connection().execute(
                "UPDATE jobs SET status = 'failed', error = ?1, finished_at = ?2
                 WHERE status IN ('queued', 'running') AND investigation_id = ?3",
                params![
                    INTERRUPTED_MESSAGE,
                    timestamp,
                    id(&summary.investigation.id)
                ],
            )?;
            self.connection().execute(
                "UPDATE agent_runs SET status = 'failed', error = ?1, finished_at = ?2
                 WHERE status IN ('queued', 'running') AND investigation_id = ?3",
                params![
                    INTERRUPTED_MESSAGE,
                    timestamp,
                    id(&summary.investigation.id)
                ],
            )?;
        }
        Ok(updated_jobs)
    }

    pub fn insert_agent_run(&self, run: &mut AgentRun) -> Result<()> {
        self.assert_can_mutate(run.investigation_id)?;
        run.seq = next_seq(self.connection(), SeqTable::AgentRuns, run.investigation_id)?;
        insert_agent_run_record(self.connection(), run)?;
        Ok(())
    }

    pub fn update_agent_run(&self, run: &AgentRun) -> Result<()> {
        self.assert_can_mutate(run.investigation_id)?;
        self.connection().execute(
            "UPDATE agent_runs SET response = ?2, status = ?3, error = ?4, context_artifact_id = ?5, job_id = ?6,
                model = ?7, cost_usd = ?8, finished_at = ?9, args = ?10
             WHERE id = ?1",
            params![
                id(&run.id),
                run.response,
                run.status.as_str(),
                run.error,
                opt_id(&run.context_artifact_id),
                opt_id(&run.job_id),
                run.model,
                run.cost_usd,
                opt_ts(&run.finished_at),
                json(&run.args),
            ],
        )?;
        Ok(())
    }

    pub fn delete_agent_run(&self, run_id: Uuid) -> Result<()> {
        self.assert_can_mutate_record("agent_runs", run_id)?;
        self.connection()
            .execute("DELETE FROM agent_runs WHERE id = ?1", params![id(&run_id)])?;
        Ok(())
    }

    /// Stores agent-proposed actions as pending (or invalid) proposals.
    pub fn insert_proposals(
        &mut self,
        investigation_id: Uuid,
        agent_run_id: Uuid,
        actions: Vec<(AgentAction, ProposalStatus, Option<String>)>,
    ) -> Result<Vec<Proposal>> {
        self.assert_can_mutate(investigation_id)?;
        let transaction = self.connection_mut().transaction()?;
        let mut proposals = Vec::with_capacity(actions.len());
        for (action, status, error) in actions {
            let proposal = Proposal {
                id: Uuid::new_v4(),
                investigation_id,
                agent_run_id,
                seq: next_seq(&transaction, SeqTable::Proposals, investigation_id)?,
                action,
                status,
                error,
                created_at: now(),
                decided_at: None,
                decided_by: None,
            };
            insert_proposal_record(&transaction, &proposal)?;
            proposals.push(proposal);
        }
        transaction.commit()?;
        Ok(proposals)
    }

    pub fn update_proposal(
        &mut self,
        proposal_id: Uuid,
        status: ProposalStatus,
        error: Option<&str>,
    ) -> Result<()> {
        let (investigation_id, action_name): (Uuid, String) = self.connection().query_row(
            "SELECT investigation_id, action FROM proposals WHERE id = ?1",
            params![id(&proposal_id)],
            |row| Ok((row.uuid(0)?, row.get(1)?)),
        )?;
        self.assert_can_mutate(investigation_id)?;
        let decided = status != ProposalStatus::Pending;
        let decided_at = decided.then(now);
        let decided_by = decided.then(|| self.identity().user_id.clone());
        let identity = self.identity().clone();
        let transaction = self.connection_mut().transaction()?;
        transaction.execute(
            "UPDATE proposals SET status = ?2, error = ?3, decided_at = ?4, decided_by = ?5 WHERE id = ?1",
            params![
                id(&proposal_id),
                status.as_str(),
                error,
                opt_ts(&decided_at),
                decided_by,
            ],
        )?;
        if status == ProposalStatus::Accepted {
            let action_label = serde_json::from_str::<AgentAction>(&action_name)
                .map(|action| action.name().to_string())
                .unwrap_or_else(|_| "ACTION".into());
            let mut details = crate::model::Fields::new();
            details.push("action", action_label.as_str());
            super::audit::append_audit_on(
                &transaction,
                investigation_id,
                &identity,
                crate::model::AuditKind::ActionAccepted,
                &format!("Accepted {action_label}"),
                details,
                vec![proposal_id],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Stores Actions from the HTTP API or scriptable CLI as pending proposals.
    /// Creates a synthetic agent run for provenance. Does not apply the Actions.
    pub fn insert_pending_actions(
        &mut self,
        investigation_id: Uuid,
        origin: &str,
        actions: Vec<AgentAction>,
    ) -> Result<Vec<Proposal>> {
        self.assert_can_mutate(investigation_id)?;
        if actions.is_empty() {
            return Ok(Vec::new());
        }
        let timestamp = now();
        let mut run = AgentRun {
            id: Uuid::new_v4(),
            investigation_id,
            seq: 0,
            thread_id: Uuid::new_v4(),
            page_id: None,
            agent_name: origin.to_string(),
            program: "os-int".into(),
            args: Vec::new(),
            question: format!("pending Actions from {origin}"),
            response: String::new(),
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
        self.insert_agent_run(&mut run)?;
        let pending = actions
            .into_iter()
            .map(|action| (action, ProposalStatus::Pending, None))
            .collect();
        self.insert_proposals(investigation_id, run.id, pending)
    }

    pub fn update_proposal_action(&self, proposal_id: Uuid, action: &AgentAction) -> Result<()> {
        self.assert_can_mutate_record("proposals", proposal_id)?;
        self.connection().execute(
            "UPDATE proposals SET action = ?2 WHERE id = ?1",
            params![id(&proposal_id), json(action)],
        )?;
        Ok(())
    }
}
