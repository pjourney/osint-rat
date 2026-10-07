use super::*;
use crate::model::{
    AgentAction, Confidence, EntityKey, EntityKind, EvidenceBatch, JobRecord, JobStatus, NewSource,
    PageKind, ProposalStatus, Provenance, SourceKind, TimelineKind, Workspace,
};

fn store_with_investigation() -> (Store, Investigation) {
    let store = Store::open_in_memory().expect("store opens");
    let investigation = store
        .create_investigation("Operation Example", "Test investigation")
        .expect("investigation created");
    (store, investigation)
}

fn dns_batch(content_hash: &str) -> EvidenceBatch {
    let mut batch = EvidenceBatch::new();
    let mut source = NewSource::collected(SourceKind::Dns, "dns", "DNS records for example.com");
    source.content = "example.com. 300 IN A 192.0.2.10".into();
    source.content_hash = Some(content_hash.into());
    let key = batch.add_source(source);
    let domain = EntityKey::new(EntityKind::Domain, "example.com");
    let ip = EntityKey::new(EntityKind::Ip, "192.0.2.10");
    let nameserver = EntityKey::new(EntityKind::Hostname, "ns1.example.net");
    batch.relate(&domain, "resolves_to", &ip, Some(key));
    batch.relate(&domain, "uses_nameserver", &nameserver, Some(key));
    batch.event(
        "2020-01-01T00:00:00Z".parse().expect("timestamp"),
        TimelineKind::Registration,
        "example.com registered",
        Some(key),
        Some(domain),
    );
    batch
}

#[test]
fn investigation_crud_round_trips() {
    let (store, mut investigation) = store_with_investigation();
    investigation.description = "Updated".into();
    investigation.tags = vec!["phishing".into()];
    store.update_investigation(&investigation).expect("updates");
    let actual = store
        .get_investigation(investigation.id)
        .expect("query")
        .expect("exists");
    assert_eq!(actual.description, "Updated");
    assert_eq!(actual.tags, vec!["phishing".to_string()]);
    store
        .delete_investigation(investigation.id)
        .expect("deletes");
    assert!(
        store
            .get_investigation(investigation.id)
            .expect("query")
            .is_none()
    );
}

#[test]
fn investigation_names_are_unique_case_insensitively() {
    let (store, _) = store_with_investigation();
    let actual = store.create_investigation("operation EXAMPLE", "");
    assert!(actual.is_err());
}

#[test]
fn find_investigation_by_name_id_and_prefix() {
    let (store, investigation) = store_with_investigation();
    let prefix = &investigation.id.to_string()[..8];
    let by_name = store
        .find_investigation("operation example")
        .expect("query");
    let by_id = store
        .find_investigation(&investigation.id.to_string())
        .expect("query");
    let by_prefix = store.find_investigation(prefix).expect("query");
    assert_eq!(by_name.map(|found| found.id), Some(investigation.id));
    assert_eq!(by_id.map(|found| found.id), Some(investigation.id));
    assert_eq!(by_prefix.map(|found| found.id), Some(investigation.id));
    assert!(
        store
            .find_investigation("missing")
            .expect("query")
            .is_none()
    );
}

#[test]
fn creating_a_domain_page_normalizes_target_and_creates_entity() {
    let (mut store, investigation) = store_with_investigation();
    let input = NewPage::new(investigation.id, "", PageKind::Domain, Some("Example.COM."));
    let page = store.create_page(input).expect("page created");
    let data = store.load_investigation(investigation.id).expect("loads");
    assert_eq!(page.title, "example.com");
    assert_eq!(page.target.as_deref(), Some("example.com"));
    assert_eq!(page.seq, 1);
    assert_eq!(data.entities.len(), 1);
    assert_eq!(data.entities[0].value, "example.com");
    assert_eq!(page.entity_id, Some(data.entities[0].id));
}

#[test]
fn invalid_page_targets_are_rejected() {
    let (mut store, investigation) = store_with_investigation();
    let input = NewPage::new(investigation.id, "Bad", PageKind::Ip, Some("not-an-ip"));
    assert!(store.create_page(input).is_err());
}

#[test]
fn findings_reference_sources() {
    let (mut store, investigation) = store_with_investigation();
    let summary = store
        .apply_batch(
            ApplyContext {
                investigation_id: investigation.id,
                page_id: None,
                job_id: None,
            },
            &dns_batch("hash-1"),
        )
        .expect("applies");
    let finding = store
        .create_finding(NewFinding {
            investigation_id: investigation.id,
            page_id: None,
            title: "example.com resolves to a documentation range".into(),
            details: String::new(),
            confidence: Confidence::High,
            provenance: Provenance::Analyst,
            source_ids: summary.source_ids.clone(),
        })
        .expect("finding created");
    let data = store.load_investigation(investigation.id).expect("loads");
    assert_eq!(data.findings[0].source_ids, summary.source_ids);
    assert_eq!(finding.short_ref().to_string(), "F-1");
    store
        .delete_source(summary.source_ids[0])
        .expect("deletes source");
    let after = store.load_investigation(investigation.id).expect("loads");
    assert!(after.findings[0].source_ids.is_empty());
}

#[test]
fn apply_batch_creates_entities_relationships_and_timeline() {
    let (mut store, investigation) = store_with_investigation();
    let page = store
        .create_page(NewPage::new(
            investigation.id,
            "",
            PageKind::Domain,
            Some("example.com"),
        ))
        .expect("page");
    let summary = store
        .apply_batch(
            ApplyContext {
                investigation_id: investigation.id,
                page_id: Some(page.id),
                job_id: None,
            },
            &dns_batch("hash-1"),
        )
        .expect("applies");
    let data = store.load_investigation(investigation.id).expect("loads");
    let workspace = Workspace::new(data);
    let domain = workspace
        .entity_by_value(&EntityKind::Domain, "example.com")
        .expect("domain entity");
    let outgoing = workspace.outgoing(domain.id);
    assert_eq!(summary.new_sources, 1);
    assert_eq!(
        summary.new_entities, 2,
        "domain already existed from the page"
    );
    assert_eq!(summary.new_relationships, 2);
    assert_eq!(outgoing.len(), 2);
    assert!(
        outgoing
            .iter()
            .all(|relationship| relationship.source_ids == summary.source_ids)
    );
    assert_eq!(workspace.data.timeline.len(), 1);
    assert_eq!(workspace.page_entity_ids(page.id).len(), 3);
    assert_eq!(domain.source_ids, summary.source_ids);
}

#[test]
fn identical_recollection_reuses_source() {
    let (mut store, investigation) = store_with_investigation();
    let context = ApplyContext {
        investigation_id: investigation.id,
        page_id: None,
        job_id: None,
    };
    let first = store
        .apply_batch(context, &dns_batch("same-hash"))
        .expect("first");
    let second = store
        .apply_batch(context, &dns_batch("same-hash"))
        .expect("second");
    let third = store
        .apply_batch(context, &dns_batch("different-hash"))
        .expect("third");
    let data = store.load_investigation(investigation.id).expect("loads");
    assert_eq!(second.unchanged_sources, 1);
    assert_eq!(second.source_ids, first.source_ids);
    assert_eq!(second.new_relationships, 0);
    assert_eq!(second.new_events, 0);
    assert_eq!(third.new_sources, 1);
    assert_eq!(data.sources.len(), 2);
    assert!(data.sources[0].last_seen_at.is_some());
}

#[test]
fn manual_entity_links_record_sources() {
    let (mut store, investigation) = store_with_investigation();
    let summary = store
        .apply_batch(
            ApplyContext {
                investigation_id: investigation.id,
                page_id: None,
                job_id: None,
            },
            &dns_batch("hash"),
        )
        .expect("applies");
    let person = store
        .add_entity(
            investigation.id,
            EntityKind::Person,
            "Alice Example",
            Provenance::Analyst,
            None,
        )
        .expect("person");
    let organization = store
        .add_entity(
            investigation.id,
            EntityKind::Organization,
            "Example Corp",
            Provenance::Analyst,
            None,
        )
        .expect("organization");
    let relationship = store
        .link_entities(
            investigation.id,
            person,
            "Associated With",
            organization,
            Provenance::Analyst,
            Confidence::Medium,
            &summary.source_ids,
        )
        .expect("links");
    let again = store
        .link_entities(
            investigation.id,
            person,
            "associated_with",
            organization,
            Provenance::Analyst,
            Confidence::Low,
            &[],
        )
        .expect("links again");
    let data = store.load_investigation(investigation.id).expect("loads");
    let stored = data
        .relationships
        .iter()
        .find(|candidate| candidate.id == relationship)
        .expect("stored");
    assert_eq!(relationship, again);
    assert_eq!(stored.kind, "associated_with");
    assert_eq!(stored.source_ids, summary.source_ids);
    assert!(
        store
            .link_entities(
                investigation.id,
                person,
                "self",
                person,
                Provenance::Analyst,
                Confidence::Low,
                &[]
            )
            .is_err()
    );
}

#[test]
fn notes_and_page_notes_persist() {
    let (mut store, investigation) = store_with_investigation();
    let page = store
        .create_page(NewPage::new(
            investigation.id,
            "Research Notes",
            PageKind::Generic,
            None,
        ))
        .expect("page");
    store
        .update_page_notes(page.id, "# Heading\nBody")
        .expect("notes");
    let note = store
        .create_note(
            investigation.id,
            Some(page.id),
            "Check the MX records",
            Provenance::Analyst,
        )
        .expect("note");
    store
        .update_note(note.id, "Check the MX records again")
        .expect("update");
    let data = store.load_investigation(investigation.id).expect("loads");
    assert_eq!(data.pages[0].notes, "# Heading\nBody");
    assert_eq!(data.notes[0].body, "Check the MX records again");
    assert!(
        store
            .create_note(investigation.id, None, "   ", Provenance::Analyst)
            .is_err()
    );
}

#[test]
fn jobs_are_persisted_and_interrupted_jobs_marked_failed() {
    let (store, investigation) = store_with_investigation();
    let mut job = JobRecord {
        id: Uuid::new_v4(),
        investigation_id: Some(investigation.id),
        seq: 0,
        kind: "collector:dns".into(),
        label: "DNS".into(),
        description: "DNS for example.com".into(),
        status: JobStatus::Running,
        program: None,
        args: Vec::new(),
        created_at: now(),
        started_at: Some(now()),
        finished_at: None,
        progress: None,
        log: String::new(),
        stderr: String::new(),
        result: None,
        error: None,
        page_id: None,
    };
    store.insert_job(&mut job).expect("inserts");
    let interrupted = store.mark_interrupted_jobs().expect("marks");
    let data = store.load_investigation(investigation.id).expect("loads");
    assert_eq!(job.seq, 1);
    assert_eq!(interrupted, 1);
    assert_eq!(data.jobs[0].status, JobStatus::Failed);
    assert!(
        data.jobs[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("interrupted")
    );
}

#[test]
fn proposals_store_actions() {
    let (mut store, investigation) = store_with_investigation();
    let mut run = crate::model::AgentRun {
        id: Uuid::new_v4(),
        investigation_id: investigation.id,
        seq: 0,
        thread_id: Uuid::new_v4(),
        page_id: None,
        agent_name: "Claude Code".into(),
        program: "claude".into(),
        args: vec!["-p".into()],
        question: "What stands out?".into(),
        response: String::new(),
        status: JobStatus::Running,
        error: None,
        focus: None,
        context_artifact_id: None,
        job_id: None,
        model: None,
        cost_usd: None,
        created_at: now(),
        finished_at: None,
    };
    store.insert_agent_run(&mut run).expect("inserts run");
    let input = vec![(
        AgentAction::AddNote {
            body: "Look at the MX records".into(),
        },
        ProposalStatus::Pending,
        None,
    )];
    let proposals = store
        .insert_proposals(investigation.id, run.id, input)
        .expect("inserts");
    store
        .update_proposal(proposals[0].id, ProposalStatus::Accepted, None)
        .expect("updates");
    let data = store.load_investigation(investigation.id).expect("loads");
    assert_eq!(data.agent_runs.len(), 1);
    assert_eq!(data.proposals[0].status, ProposalStatus::Accepted);
    assert!(data.proposals[0].decided_at.is_some());
}

#[test]
fn ui_state_upserts() {
    let (store, investigation) = store_with_investigation();
    let scope = investigation.id.to_string();
    store.set_state(&scope, "view", "a").expect("set");
    store.set_state(&scope, "view", "b").expect("set again");
    assert_eq!(
        store.get_state(&scope, "view").expect("get"),
        Some("b".into())
    );
    assert_eq!(store.get_state(&scope, "missing").expect("get"), None);
}

#[test]
fn full_investigation_round_trips_through_insert() {
    let (mut store, investigation) = store_with_investigation();
    let page = store
        .create_page(NewPage::new(
            investigation.id,
            "",
            PageKind::Domain,
            Some("example.com"),
        ))
        .expect("page");
    store
        .apply_batch(
            ApplyContext {
                investigation_id: investigation.id,
                page_id: Some(page.id),
                job_id: None,
            },
            &dns_batch("hash"),
        )
        .expect("applies");
    let expected = store.load_investigation(investigation.id).expect("loads");
    let mut target = Store::open_in_memory().expect("target store");
    target
        .insert_investigation_data(&expected)
        .expect("inserts");
    let actual = target
        .load_investigation(investigation.id)
        .expect("loads copy");
    assert_eq!(actual, expected);
}

#[test]
fn integrity_check_reports_ok() {
    let (store, _) = store_with_investigation();
    assert_eq!(store.integrity_check().expect("check"), "ok");
    assert_eq!(
        store.schema_version().expect("version"),
        migrations::latest_version()
    );
}
