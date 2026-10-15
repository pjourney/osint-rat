//! Case home dashboard: targets, glance counts, and NEXT actions.

use serde::Serialize;

use super::App;
use crate::model::{Page, PageKind, Provenance, Timestamp, Workspace, now};
use crate::tui::relative_time_from;

/// One target row on the Case board.
#[derive(Clone, Debug)]
pub struct CaseTarget {
    pub page_id: uuid::Uuid,
    pub kind: PageKind,
    pub value: String,
    pub provenance: Option<Provenance>,
    pub source_count: usize,
    pub last_collect: Option<Timestamp>,
    pub running: bool,
    pub never_collected: bool,
}

/// Provenance breakdown used by AT A GLANCE.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct GlanceCounts {
    pub total: usize,
    pub collected: usize,
    pub external: usize,
    pub analyst: usize,
    pub agent: usize,
}

impl GlanceCounts {
    fn push(&mut self, provenance: Provenance) {
        self.total += 1;
        match provenance {
            Provenance::Collected => self.collected += 1,
            Provenance::External => self.external += 1,
            Provenance::Analyst => self.analyst += 1,
            Provenance::Agent => self.agent += 1,
        }
    }

    /// Provenance buckets with a non-zero count, in legend order.
    pub fn nonzero_buckets(self) -> Vec<(Provenance, usize)> {
        [
            (Provenance::Collected, self.collected),
            (Provenance::External, self.external),
            (Provenance::Analyst, self.analyst),
            (Provenance::Agent, self.agent),
        ]
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .collect()
    }
}

/// A suggested next action on Case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaseNext {
    pub label: String,
    pub hint: String,
}

/// Snapshot of the Case home board.
#[derive(Clone, Debug)]
pub struct CaseHome {
    pub targets: Vec<CaseTarget>,
    pub entities: GlanceCounts,
    pub findings: GlanceCounts,
    pub next: Vec<CaseNext>,
    pub is_empty: bool,
}

impl App {
    pub fn listed_case_targets(&self) -> Vec<CaseTarget> {
        let Some(workspace) = self.workspace.as_ref() else {
            return Vec::new();
        };
        workspace
            .data
            .pages
            .iter()
            .map(|page| case_target(self, workspace, page))
            .collect()
    }

    pub fn case_home(&self) -> CaseHome {
        let Some(workspace) = self.workspace.as_ref() else {
            return CaseHome {
                targets: Vec::new(),
                entities: GlanceCounts::default(),
                findings: GlanceCounts::default(),
                next: Vec::new(),
                is_empty: true,
            };
        };
        let targets = self.listed_case_targets();
        let mut entities = GlanceCounts::default();
        for entity in workspace.visible_entities() {
            entities.push(entity.provenance);
        }
        let mut findings = GlanceCounts::default();
        for finding in &workspace.data.findings {
            findings.push(finding.provenance);
        }
        let review = self.pending_review_count();
        let never: Vec<&CaseTarget> = targets
            .iter()
            .filter(|target| target.never_collected)
            .collect();
        let mut next = Vec::new();
        if review > 0 {
            next.push(CaseNext {
                label: format!("{review} agent proposals waiting"),
                hint: "5 review".into(),
            });
        }
        if !never.is_empty() {
            next.push(CaseNext {
                label: format!(
                    "{} target{} never collected",
                    never.len(),
                    if never.len() == 1 { "" } else { "s" }
                ),
                hint: "c collect".into(),
            });
        }
        if targets.is_empty() {
            next.push(CaseNext {
                label: "Add a target to start collecting".into(),
                hint: "n new target".into(),
            });
        }
        let is_empty = targets.is_empty() && entities.total == 0 && findings.total == 0;
        CaseHome {
            targets,
            entities,
            findings,
            next,
            is_empty,
        }
    }

    pub fn case_target_status_label(target: &CaseTarget) -> String {
        if target.running {
            return "running".into();
        }
        if target.never_collected {
            return "never".into();
        }
        if let Some(when) = target.last_collect {
            return relative_time_from(when, now());
        }
        "—".into()
    }
}

fn case_target(app: &App, workspace: &Workspace, page: &Page) -> CaseTarget {
    let entity = workspace
        .page_entity_ids(page.id)
        .into_iter()
        .find_map(|id| workspace.entity(id));
    let source_count = workspace
        .data
        .sources
        .iter()
        .filter(|source| source.page_id == Some(page.id))
        .count()
        .max(entity.map(|item| item.source_ids.len()).unwrap_or(0));
    let last_collect = workspace
        .data
        .sources
        .iter()
        .filter(|source| source.page_id == Some(page.id))
        .map(|source| source.last_seen_at.unwrap_or(source.retrieved_at))
        .max();
    let running = app
        .job_records
        .values()
        .any(|job| job.status.is_active() && job.page_id == Some(page.id))
        || workspace
            .data
            .jobs
            .iter()
            .any(|job| job.status.is_active() && job.page_id == Some(page.id));
    let never_collected = last_collect.is_none() && !running;
    CaseTarget {
        page_id: page.id,
        kind: page.kind.clone(),
        value: page.target.clone().unwrap_or_else(|| page.title.clone()),
        provenance: entity.map(|item| item.provenance),
        source_count,
        last_collect,
        running,
        never_collected,
    }
}

impl CaseTarget {
    pub fn kind_tag(&self) -> &'static str {
        match self.kind {
            PageKind::Domain => "DOM",
            PageKind::Ip => "IP ",
            PageKind::Url => "URL",
            PageKind::Email => "EML",
            PageKind::Asn => "ASN",
            PageKind::Repository => "REP",
            PageKind::File => "FIL",
            PageKind::Username => "USR",
            PageKind::Organization => "ORG",
            PageKind::Person => "PER",
            PageKind::Event => "EVT",
            PageKind::Generic | PageKind::Other(_) => "PGE",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::GlanceCounts;
    use crate::model::Provenance;

    #[test]
    fn glance_counts_tally_each_provenance() {
        let mut counts = GlanceCounts::default();
        counts.push(Provenance::Collected);
        counts.push(Provenance::Collected);
        counts.push(Provenance::Agent);
        counts.push(Provenance::Analyst);
        counts.push(Provenance::External);
        assert_eq!(counts.total, 5);
        assert_eq!(counts.collected, 2);
        assert_eq!(counts.agent, 1);
        assert_eq!(
            counts.nonzero_buckets(),
            vec![
                (Provenance::Collected, 2),
                (Provenance::External, 1),
                (Provenance::Analyst, 1),
                (Provenance::Agent, 1),
            ]
        );
        assert_eq!(counts.analyst, 1);
        assert_eq!(counts.external, 1);
        assert!(GlanceCounts::default().nonzero_buckets().is_empty());
    }
}
