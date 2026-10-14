use super::*;
use crate::model::{
    AgentAction, ArtifactKind, AuditKind, Confidence, EntityKey, EntityKind, EvidenceBatch,
    Identity, JobRecord, JobStatus, Marking, NewArtifact, NewSource, PageKind, ProposalStatus,
    Provenance, Role, SearchAroundQuery, SourceKind, Target, TargetKind, TimelineKind, Workspace,
};
use rusqlite::params;
use uuid::Uuid;

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
fn a_changed_observation_is_recorded_on_the_timeline() {
    let (mut store, investigation) = store_with_investigation();
    let context = ApplyContext {
        investigation_id: investigation.id,
        page_id: None,
        job_id: None,
    };
    store
        .apply_batch(context, &dns_batch("hash-1"))
        .expect("first");
    let changed = store
        .apply_batch(context, &dns_batch("hash-2"))
        .expect("changed");
    let data = store.load_investigation(investigation.id).expect("loads");
    assert_eq!(changed.new_sources, 1);
    assert_eq!(changed.changed_observations, 1);
    assert!(changed.describe().contains("changed observation"));
    assert!(
        data.timeline
            .iter()
            .any(|event| event.kind == TimelineKind::Changed
                && event.title.contains("DNS records for example.com"))
    );
    assert!(
        data.events.iter().any(|event| {
            event.kind == TimelineKind::Changed
                && event.fields.get("change_intelligence") == Some("observation_hash")
        }),
        "observation-hash changes must become Event objects"
    );
    let repeated = store
        .apply_batch(context, &dns_batch("hash-2"))
        .expect("repeat");
    assert_eq!(repeated.unchanged_sources, 1);
    assert_eq!(repeated.changed_observations, 0);
}

#[test]
fn collected_diagnostics_are_upserted_and_cleared() {
    let (mut store, investigation) = store_with_investigation();
    let context = ApplyContext {
        investigation_id: investigation.id,
        page_id: None,
        job_id: None,
    };
    let mut open = EvidenceBatch::new();
    let mut source = NewSource::collected(SourceKind::Dns, "dns", "DNS records for example.com");
    source.content_hash = Some("diag-1".into());
    let key = open.add_source(source);
    open.add_finding(
        "example.com spf",
        "No SPF record published",
        Confidence::High,
        Some(key),
    );
    let created = store.apply_batch(context, &open).expect("creates");
    assert_eq!(created.new_findings, 1);
    let data = store.load_investigation(investigation.id).expect("loads");
    assert_eq!(data.findings.len(), 1);
    assert_eq!(data.findings[0].provenance, Provenance::Collected);
    assert_eq!(data.findings[0].source_ids, created.source_ids);
    assert!(
        data.events.iter().any(|event| {
            event.kind == TimelineKind::Finding
                && event.fields.get("change_intelligence") == Some("diagnostic_open")
        }),
        "diagnostic open must become an Event candidate"
    );

    let mut revised = EvidenceBatch::new();
    let mut source = NewSource::collected(SourceKind::Dns, "dns", "DNS records for example.com");
    source.content_hash = Some("diag-1".into());
    let key = revised.add_source(source);
    revised.add_finding(
        "example.com spf",
        "SPF +all authorizes any sender",
        Confidence::Medium,
        Some(key),
    );
    let updated = store.apply_batch(context, &revised).expect("updates");
    assert_eq!(updated.updated_findings, 1);
    assert_eq!(updated.new_findings, 0);
    let data = store.load_investigation(investigation.id).expect("loads");
    assert_eq!(data.findings.len(), 1);
    assert!(data.findings[0].details.contains("+all"));

    let mut cleared = EvidenceBatch::new();
    let mut source = NewSource::collected(SourceKind::Dns, "dns", "DNS records for example.com");
    source.content_hash = Some("diag-1".into());
    cleared.add_source(source);
    cleared.resolve_finding("example.com spf");
    let resolved = store.apply_batch(context, &cleared).expect("clears");
    assert_eq!(resolved.resolved_findings, 1);
    let data = store.load_investigation(investigation.id).expect("loads");
    assert!(data.findings.is_empty());
    assert!(
        data.events.iter().any(|event| {
            event.kind == TimelineKind::Finding
                && event.fields.get("change_intelligence") == Some("diagnostic_clear")
        }),
        "diagnostic clear must become an Event candidate"
    );
}

#[test]
fn recollection_without_a_page_keeps_the_finding_page() {
    let (mut store, investigation) = store_with_investigation();
    let page = store
        .create_page(NewPage::new(
            investigation.id,
            "",
            PageKind::Domain,
            Some("example.com"),
        ))
        .expect("page");
    let on_page = ApplyContext {
        investigation_id: investigation.id,
        page_id: Some(page.id),
        job_id: None,
    };
    let mut open = EvidenceBatch::new();
    let mut source = NewSource::collected(SourceKind::Dns, "dns", "DNS records for example.com");
    source.content_hash = Some("diag-page".into());
    let key = open.add_source(source);
    open.add_finding(
        "example.com spf",
        "No SPF record published",
        Confidence::High,
        Some(key),
    );
    store.apply_batch(on_page, &open).expect("creates");
    let broad = ApplyContext {
        investigation_id: investigation.id,
        page_id: None,
        job_id: None,
    };
    let mut revised = EvidenceBatch::new();
    let mut source = NewSource::collected(SourceKind::Dns, "dns", "DNS records for example.com");
    source.content_hash = Some("diag-page".into());
    let key = revised.add_source(source);
    revised.add_finding(
        "example.com spf",
        "SPF still missing after a broader collect",
        Confidence::High,
        Some(key),
    );
    store.apply_batch(broad, &revised).expect("updates");
    let data = store.load_investigation(investigation.id).expect("loads");
    assert_eq!(data.findings.len(), 1);
    assert_eq!(data.findings[0].page_id, Some(page.id));
    assert!(data.findings[0].details.contains("broader collect"));
}

#[test]
fn asn_page_creates_an_asn_entity() {
    let (mut store, investigation) = store_with_investigation();
    let page = store
        .create_page(NewPage::new(
            investigation.id,
            "",
            PageKind::Asn,
            Some("as13335"),
        ))
        .expect("page");
    assert_eq!(page.kind, PageKind::Asn);
    assert_eq!(page.target.as_deref(), Some("AS13335"));
    let data = store.load_investigation(investigation.id).expect("loads");
    assert_eq!(data.entities[0].kind, EntityKind::Asn);
    assert_eq!(data.entities[0].value, "AS13335");
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
    assert!(data.proposals[0].decided_by.is_some());
}

#[test]
fn apply_batch_catalogues_known_and_custom_link_types() {
    let (mut store, investigation) = store_with_investigation();
    let mut batch = dns_batch("hash-catalog");
    batch.relate(
        &crate::model::EntityKey::new(EntityKind::Domain, "example.com"),
        "spotted_near",
        &crate::model::EntityKey::new(EntityKind::Ip, "192.0.2.10"),
        None,
    );
    store
        .apply_batch(
            ApplyContext {
                investigation_id: investigation.id,
                page_id: None,
                job_id: None,
            },
            &batch,
        )
        .expect("applies");
    let catalogued = store.list_link_types(investigation.id).expect("catalogued");
    assert!(
        catalogued
            .iter()
            .any(|item| item.kind == "resolves_to" && !item.description.is_empty())
    );
    assert!(
        catalogued
            .iter()
            .any(|item| item.kind == "spotted_near" && item.is_other())
    );
}

#[test]
fn backfill_link_catalog_uses_original_relationship_time() {
    let (mut store, investigation) = store_with_investigation();
    store
        .apply_batch(
            ApplyContext {
                investigation_id: investigation.id,
                page_id: None,
                job_id: None,
            },
            &dns_batch("hash-backfill"),
        )
        .expect("applies");
    let original = "2020-06-15T12:34:56.000Z";
    store
        .connection()
        .execute(
            "UPDATE relationships SET created_at = ?1 WHERE investigation_id = ?2",
            params![original, investigation.id.to_string()],
        )
        .expect("backdate");
    store
        .connection()
        .execute("DELETE FROM link_types", [])
        .expect("clear catalog");
    graph::backfill_link_catalog(store.connection()).expect("backfill");
    let seen: String = store
        .connection()
        .query_row(
            "SELECT first_seen_at FROM link_types WHERE investigation_id = ?1 AND kind = 'resolves_to'",
            params![investigation.id.to_string()],
            |row| row.get(0),
        )
        .expect("first_seen_at");
    assert_eq!(seen, original);
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

fn infra_batch() -> EvidenceBatch {
    let mut batch = EvidenceBatch::new();
    let mut source =
        NewSource::collected(SourceKind::Dns, "dns", "DNS and routing for example.com");
    source.content = "example.com A 192.0.2.10".into();
    source.content_hash = Some("infra-hash".into());
    let key = batch.add_source(source);
    let domain = EntityKey::new(EntityKind::Domain, "example.com");
    let ip = EntityKey::new(EntityKind::Ip, "192.0.2.10");
    let nameserver = EntityKey::new(EntityKind::Hostname, "ns1.example.net");
    let asn = EntityKey::new(EntityKind::Asn, "AS64496");
    let ns_ip = EntityKey::new(EntityKind::Ip, "192.0.2.53");
    batch.relate(&domain, "resolves_to", &ip, Some(key));
    batch.relate(&domain, "uses_nameserver", &nameserver, Some(key));
    batch.relate(&ip, "announced_by", &asn, Some(key));
    batch.relate(&nameserver, "resolves_to", &ns_ip, Some(key));
    batch
}

#[test]
fn search_around_sqlite_fixture_matches_hop_counts_and_caps() {
    let (mut store, investigation) = store_with_investigation();
    store
        .apply_batch(
            ApplyContext {
                investigation_id: investigation.id,
                page_id: None,
                job_id: None,
            },
            &infra_batch(),
        )
        .expect("applies");
    let data = store.load_investigation(investigation.id).expect("loads");
    let domain = data
        .entities
        .iter()
        .find(|entity| entity.kind == EntityKind::Domain)
        .expect("domain");
    let asn = data
        .entities
        .iter()
        .find(|entity| entity.kind == EntityKind::Asn)
        .expect("asn");
    let one = store
        .search_around(
            investigation.id,
            &SearchAroundQuery::new(vec![domain.id], 1),
        )
        .expect("one hop");
    assert_eq!(one.nodes.len(), 3);
    assert!(!one.node_ids().any(|id| id == asn.id));
    let two = store
        .search_around(
            investigation.id,
            &SearchAroundQuery::new(vec![domain.id], 2),
        )
        .expect("two hops");
    assert_eq!(two.nodes.len(), 5);
    assert!(two.node_ids().any(|id| id == asn.id));
    let mut capped = SearchAroundQuery::new(vec![domain.id], 2);
    capped.node_cap = 2;
    let limited = store
        .search_around(investigation.id, &capped)
        .expect("capped");
    assert_eq!(limited.nodes.len(), 2);
    assert!(limited.truncated);
    assert!(limited.requires_confirm());
    assert_eq!(limited.uncapped_node_count, 5);
}

#[test]
fn resolve_merges_sources_and_unresolve_restores_without_orphans() {
    let (mut store, investigation) = store_with_investigation();
    store
        .apply_batch(
            ApplyContext {
                investigation_id: investigation.id,
                page_id: None,
                job_id: None,
            },
            &dns_batch("hash-resolve"),
        )
        .expect("batch");
    let host = store
        .add_entity(
            investigation.id,
            EntityKind::Hostname,
            "example.com",
            Provenance::Collected,
            None,
        )
        .expect("host");
    let before = store.load_investigation(investigation.id).expect("load");
    let domain = before
        .entities
        .iter()
        .find(|entity| entity.kind == EntityKind::Domain)
        .expect("domain")
        .clone();
    let host_entity = before
        .entities
        .iter()
        .find(|entity| entity.id == host)
        .expect("host row")
        .clone();
    store
        .create_finding(NewFinding {
            investigation_id: investigation.id,
            page_id: None,
            title: "Host observed".into(),
            details: String::new(),
            confidence: Confidence::Medium,
            provenance: Provenance::Analyst,
            source_ids: host_entity.source_ids.clone(),
        })
        .expect("finding");
    let resolution = store
        .resolve_entities(
            investigation.id,
            &[domain.id, host],
            domain.id,
            "same canonical host",
            "tester",
        )
        .expect("merge");
    let merged = store.load_investigation(investigation.id).expect("merged");
    let workspace = Workspace::new(merged.clone());
    assert_eq!(
        workspace.visible_entities().len(),
        merged.entities.len() - 1
    );
    assert!(merged.is_absorbed(host));
    assert_eq!(merged.canonical_id(host), domain.id);
    let view = crate::model::ObjectView::for_entity(
        &workspace,
        workspace.entity(domain.id).expect("canonical"),
    );
    assert!(
        view.constituents
            .iter()
            .any(|constituent| constituent.value == "example.com")
    );
    let graph = store
        .search_around(
            investigation.id,
            &SearchAroundQuery::new(vec![domain.id], 2),
        )
        .expect("graph");
    assert!(!graph.node_ids().any(|id| id == host));
    assert!(graph.node_ids().any(|id| id == domain.id));
    assert!(
        merged
            .timeline
            .iter()
            .any(|event| event.kind == TimelineKind::EntitiesResolved)
    );
    let suggestions = crate::model::suggest_resolutions(&merged);
    assert!(
        suggestions
            .iter()
            .all(|item| !item.entity_ids.contains(&domain.id) || !item.entity_ids.contains(&host)),
        "auto-merge must not occur; resolved pairs stay out of suggestions: {suggestions:?}"
    );
    store
        .unresolve_entities(resolution.id, "tester")
        .expect("split");
    let restored = store
        .load_investigation(investigation.id)
        .expect("restored");
    assert!(!restored.is_absorbed(host));
    assert_eq!(store.integrity_check().expect("integrity"), "ok");
    assert!(store.foreign_key_violations().expect("fk").is_empty());
    assert!(
        restored
            .timeline
            .iter()
            .any(|event| event.kind == TimelineKind::EntitiesUnresolved)
    );
    let restored_workspace = Workspace::new(restored.clone());
    assert_eq!(
        restored_workspace.visible_entities().len(),
        restored.entities.len()
    );
}

#[test]
fn suggestions_appear_for_known_duplicates_without_merging() {
    let (mut store, investigation) = store_with_investigation();
    store
        .add_entity(
            investigation.id,
            EntityKind::Username,
            "octocat",
            Provenance::Analyst,
            None,
        )
        .expect("user");
    store
        .add_entity(
            investigation.id,
            EntityKind::Url,
            "https://github.com/octocat",
            Provenance::External,
            None,
        )
        .expect("url");
    let data = store.load_investigation(investigation.id).expect("load");
    let suggestions = crate::model::suggest_resolutions(&data);
    assert!(
        suggestions
            .iter()
            .any(|item| item.rule == crate::model::SuggestionRule::UsernameProfileUrl),
        "{suggestions:?}"
    );
    assert!(data.resolutions.is_empty());
    assert_eq!(data.entities.len(), 2);
}

#[test]
fn unresolve_keeps_post_merge_property_and_source_edits() {
    let (mut store, investigation) = store_with_investigation();
    let domain = store
        .add_entity(
            investigation.id,
            EntityKind::Domain,
            "example.com",
            Provenance::Analyst,
            None,
        )
        .expect("domain");
    let host = store
        .add_entity(
            investigation.id,
            EntityKind::Hostname,
            "example.com",
            Provenance::Collected,
            None,
        )
        .expect("host");
    store
        .update_entity(host, "Host Label", "")
        .expect("host label");
    let mut host_batch = EvidenceBatch::new();
    let host_key = host_batch.add_source(NewSource::collected(
        SourceKind::Dns,
        "dns",
        "host observation",
    ));
    host_batch.entity(EntityKind::Hostname, "example.com", Some(host_key));
    let host_applied = store
        .apply_batch(
            ApplyContext {
                investigation_id: investigation.id,
                page_id: None,
                job_id: None,
            },
            &host_batch,
        )
        .expect("host source");
    let copied_source = host_applied.source_ids[0];
    let resolution = store
        .resolve_entities(
            investigation.id,
            &[domain, host],
            domain,
            "same host",
            "tester",
        )
        .expect("merge");
    assert!(
        resolution
            .property_writes
            .iter()
            .any(|write| write.key == crate::model::LABEL_WRITE_KEY
                && write.previous.is_none()
                && write.new_value == "Host Label")
    );
    assert!(resolution.source_writes.contains(&copied_source));
    store
        .update_entity(domain, "Analyst edit", "kept notes")
        .expect("post-merge label");
    let mut post_batch = EvidenceBatch::new();
    let post_key = post_batch.add_source(NewSource::collected(
        SourceKind::Manual,
        "analyst",
        "post-merge note",
    ));
    post_batch.entity(EntityKind::Domain, "example.com", Some(post_key));
    let post_applied = store
        .apply_batch(
            ApplyContext {
                investigation_id: investigation.id,
                page_id: None,
                job_id: None,
            },
            &post_batch,
        )
        .expect("post-merge source");
    let post_source = post_applied.source_ids[0];
    store
        .unresolve_entities(resolution.id, "tester")
        .expect("split");
    let restored = store
        .load_investigation(investigation.id)
        .expect("restored");
    let canonical = restored
        .entities
        .iter()
        .find(|entity| entity.id == domain)
        .expect("canonical");
    assert_eq!(canonical.label, "Analyst edit");
    assert_eq!(canonical.notes, "kept notes");
    assert!(canonical.source_ids.contains(&post_source));
    assert!(!canonical.source_ids.contains(&copied_source));
}

#[test]
fn resolve_can_stack_an_already_canonical_winner() {
    let (mut store, investigation) = store_with_investigation();
    let first = store
        .add_entity(
            investigation.id,
            EntityKind::Domain,
            "example.com",
            Provenance::Analyst,
            None,
        )
        .expect("first");
    let second = store
        .add_entity(
            investigation.id,
            EntityKind::Hostname,
            "example.com",
            Provenance::Collected,
            None,
        )
        .expect("second");
    let third = store
        .add_entity(
            investigation.id,
            EntityKind::Url,
            "https://example.com",
            Provenance::External,
            None,
        )
        .expect("third");
    store
        .resolve_entities(
            investigation.id,
            &[first, second],
            first,
            "first bag",
            "tester",
        )
        .expect("first merge");
    store
        .resolve_entities(
            investigation.id,
            &[first, third],
            third,
            "stack winner",
            "tester",
        )
        .expect("stack existing winner");
    let data = store.load_investigation(investigation.id).expect("load");
    assert_eq!(data.canonical_id(second), third);
    assert_eq!(data.canonical_id(first), third);
    let absorbed = store
        .add_entity(
            investigation.id,
            EntityKind::Email,
            "ops@example.com",
            Provenance::Analyst,
            None,
        )
        .expect("extra");
    let err = store
        .resolve_entities(
            investigation.id,
            &[second, absorbed],
            absorbed,
            "should fail",
            "tester",
        )
        .expect_err("constituent of another bag");
    let message = format!("{err:#}");
    assert!(
        message.contains("already absorbed") && message.contains("unresolve"),
        "{message}"
    );
}

struct SharedBagFixture {
    canonical: Uuid,
    shared_source: Uuid,
    original_source: Uuid,
    bag1: crate::model::Resolution,
    bag2: crate::model::Resolution,
}

fn two_bags_sharing_source_and_asn(store: &mut Store, investigation_id: Uuid) -> SharedBagFixture {
    let canonical = store
        .add_entity(
            investigation_id,
            EntityKind::Domain,
            "example.com",
            Provenance::Analyst,
            None,
        )
        .expect("canonical");
    store
        .add_entity(
            investigation_id,
            EntityKind::Hostname,
            "ns1.example.net",
            Provenance::Collected,
            None,
        )
        .expect("ns1");
    store
        .add_entity(
            investigation_id,
            EntityKind::Hostname,
            "ns2.example.net",
            Provenance::Collected,
            None,
        )
        .expect("ns2");
    let context = ApplyContext {
        investigation_id,
        page_id: None,
        job_id: None,
    };
    let mut original = EvidenceBatch::new();
    let original_key = original.add_source(NewSource::collected(
        SourceKind::Manual,
        "analyst",
        "original on canonical",
    ));
    let domain_key = original.entity(EntityKind::Domain, "example.com", Some(original_key));
    original.annotate(&domain_key, "registrar", "Keep Me");
    let original_applied = store.apply_batch(context, &original).expect("original");
    let original_source = original_applied.source_ids[0];
    let mut shared = EvidenceBatch::new();
    let shared_key = shared.add_source(NewSource::collected(
        SourceKind::Dns,
        "dns",
        "shared nameserver source",
    ));
    let ns1_key = shared.entity(EntityKind::Hostname, "ns1.example.net", Some(shared_key));
    let ns2_key = shared.entity(EntityKind::Hostname, "ns2.example.net", Some(shared_key));
    shared.annotate(&ns1_key, "asn", "AS1");
    shared.annotate(&ns2_key, "asn", "AS1");
    let shared_applied = store.apply_batch(context, &shared).expect("shared");
    let shared_source = shared_applied.source_ids[0];
    let data = store.load_investigation(investigation_id).expect("load");
    let ns1 = data
        .entities
        .iter()
        .find(|entity| entity.value == "ns1.example.net")
        .expect("ns1 row")
        .id;
    let ns2 = data
        .entities
        .iter()
        .find(|entity| entity.value == "ns2.example.net")
        .expect("ns2 row")
        .id;
    let bag1 = store
        .resolve_entities(
            investigation_id,
            &[canonical, ns1],
            canonical,
            "first nameserver",
            "tester",
        )
        .expect("bag1");
    assert!(bag1.source_writes.contains(&shared_source));
    assert!(
        bag1.property_writes
            .iter()
            .any(|write| write.key == "asn" && write.new_value == "AS1")
    );
    let bag2 = store
        .resolve_entities(
            investigation_id,
            &[canonical, ns2],
            canonical,
            "second nameserver",
            "tester",
        )
        .expect("bag2");
    assert!(
        bag2.source_writes.is_empty(),
        "already-present sources must not be claimed by the second bag: {:?}",
        bag2.source_writes
    );
    assert!(
        bag2.property_writes.is_empty(),
        "already-present fields must not be claimed by the second bag: {:?}",
        bag2.property_writes
    );
    SharedBagFixture {
        canonical,
        shared_source,
        original_source,
        bag1,
        bag2,
    }
}

fn canonical_row(store: &Store, investigation_id: Uuid, canonical: Uuid) -> crate::model::Entity {
    store
        .load_investigation(investigation_id)
        .expect("load")
        .entities
        .into_iter()
        .find(|entity| entity.id == canonical)
        .expect("canonical")
}

#[test]
fn shared_source_write_survives_until_last_bag() {
    let (mut store, investigation) = store_with_investigation();
    let fixture = two_bags_sharing_source_and_asn(&mut store, investigation.id);
    store
        .unresolve_entities(fixture.bag1.id, "tester")
        .expect("unresolve first bag");
    let data = store.load_investigation(investigation.id).expect("load");
    let canonical = data
        .entities
        .iter()
        .find(|entity| entity.id == fixture.canonical)
        .expect("canonical");
    assert!(canonical.source_ids.contains(&fixture.shared_source));
    assert!(canonical.source_ids.contains(&fixture.original_source));
    assert!(
        data.is_absorbed(
            data.entities
                .iter()
                .find(|entity| entity.value == "ns2.example.net")
                .expect("ns2")
                .id
        )
    );
}

#[test]
fn shared_property_write_survives_until_last_bag() {
    let (mut store, investigation) = store_with_investigation();
    let fixture = two_bags_sharing_source_and_asn(&mut store, investigation.id);
    store
        .unresolve_entities(fixture.bag1.id, "tester")
        .expect("unresolve first bag");
    let canonical = canonical_row(&store, investigation.id, fixture.canonical);
    assert_eq!(canonical.fields.get("asn"), Some("AS1"));
    assert_eq!(canonical.fields.get("registrar"), Some("Keep Me"));
}

#[test]
fn unresolve_last_bag_removes_shared_writes_and_keeps_originals() {
    let (mut store, investigation) = store_with_investigation();
    let fixture = two_bags_sharing_source_and_asn(&mut store, investigation.id);
    store
        .unresolve_entities(fixture.bag1.id, "tester")
        .expect("unresolve first");
    store
        .unresolve_entities(fixture.bag2.id, "tester")
        .expect("unresolve last");
    let data = store.load_investigation(investigation.id).expect("load");
    let canonical = data
        .entities
        .iter()
        .find(|entity| entity.id == fixture.canonical)
        .expect("canonical");
    assert!(canonical.source_ids.contains(&fixture.original_source));
    assert!(!canonical.source_ids.contains(&fixture.shared_source));
    assert_eq!(canonical.fields.get("registrar"), Some("Keep Me"));
    assert!(canonical.fields.get("asn").is_none());
    assert!(
        !data.is_absorbed(
            data.entities
                .iter()
                .find(|entity| entity.value == "ns1.example.net")
                .expect("ns1")
                .id
        )
    );
    assert!(
        !data.is_absorbed(
            data.entities
                .iter()
                .find(|entity| entity.value == "ns2.example.net")
                .expect("ns2")
                .id
        )
    );
}

#[test]
fn ingest_after_merge_cites_constituents_and_unresolve_stays_clean() {
    let (mut store, investigation) = store_with_investigation();
    let domain = store
        .add_entity(
            investigation.id,
            EntityKind::Domain,
            "evil.example",
            Provenance::Analyst,
            None,
        )
        .expect("domain");
    let host = store
        .add_entity(
            investigation.id,
            EntityKind::Hostname,
            "www.evil.example",
            Provenance::Collected,
            None,
        )
        .expect("host");
    store
        .resolve_entities(
            investigation.id,
            &[domain, host],
            domain,
            "same site",
            "tester",
        )
        .expect("merge");
    let report = crate::ingest::parse_stix(
        r#"{
          "type": "bundle",
          "objects": [
            {"type": "indicator", "name": "c2", "pattern": "[domain-name:value = 'evil.example']"},
            {"type": "identity", "name": "Example Org", "identity_class": "organization"}
          ]
        }"#,
        "stix.json",
    )
    .expect("stix");
    store
        .apply_batch(
            ApplyContext {
                investigation_id: investigation.id,
                page_id: None,
                job_id: None,
            },
            &report.batch,
        )
        .expect("ingest");
    let data = store.load_investigation(investigation.id).expect("load");
    let workspace = Workspace::new(data.clone());
    assert_eq!(workspace.canonical_id(host), domain);
    assert!(workspace.data.is_absorbed(host));
    let view = crate::model::ObjectView::for_entity(
        &workspace,
        workspace.entity(domain).expect("canonical"),
    );
    assert!(!view.sources.is_empty());
    store
        .apply_batch(
            ApplyContext {
                investigation_id: investigation.id,
                page_id: None,
                job_id: None,
            },
            &report.batch,
        )
        .expect("reingest");
    let again = store.load_investigation(investigation.id).expect("again");
    let stix_sources = again
        .sources
        .iter()
        .filter(|source| source.collector.as_deref() == Some("ingest:stix"))
        .count();
    assert_eq!(
        stix_sources, 1,
        "identical re-ingest must not duplicate sources"
    );
    let resolution_id = again.resolutions[0].id;
    store
        .unresolve_entities(resolution_id, "tester")
        .expect("unresolve");
    assert_eq!(store.integrity_check().expect("integrity"), "ok");
    assert!(store.foreign_key_violations().expect("fk").is_empty());
}

#[test]
fn untrusted_resolved_findings_do_not_delete_collected_diagnostics() {
    let (mut store, investigation) = store_with_investigation();
    let context = ApplyContext {
        investigation_id: investigation.id,
        page_id: None,
        job_id: None,
    };
    let mut open = EvidenceBatch::new();
    let mut source = NewSource::collected(SourceKind::Dns, "dns", "DNS records for example.com");
    source.content_hash = Some("diag-lock".into());
    let key = open.add_source(source);
    open.add_finding(
        "example.com spf",
        "No SPF record published",
        Confidence::High,
        Some(key),
    );
    store.apply_batch(context, &open).expect("creates");

    let mut hostile = EvidenceBatch::new();
    let mut source = NewSource::external(SourceKind::Document, "ingest:batch", "hostile");
    source.content_hash = Some("hostile".into());
    hostile.add_source(source);
    hostile.resolved_findings.push("example.com spf".into());
    let applied = store.apply_batch(context, &hostile).expect("hostile");
    assert_eq!(applied.resolved_findings, 0);
    let data = store.load_investigation(investigation.id).expect("load");
    assert_eq!(data.findings.len(), 1);
    assert_eq!(data.findings[0].provenance, Provenance::Collected);
}

#[test]
fn ingest_findings_are_external_and_do_not_upsert_collected() {
    let (mut store, investigation) = store_with_investigation();
    let context = ApplyContext {
        investigation_id: investigation.id,
        page_id: None,
        job_id: None,
    };
    let mut collected = EvidenceBatch::new();
    let mut source = NewSource::collected(SourceKind::Dns, "dns", "DNS records");
    source.content_hash = Some("col".into());
    let key = collected.add_source(source);
    collected.add_finding(
        "same title",
        "collected details",
        Confidence::High,
        Some(key),
    );
    store.apply_batch(context, &collected).expect("collected");

    let mut ingested = EvidenceBatch::new();
    let mut source = NewSource::external(SourceKind::Stix, "ingest:stix", "STIX");
    source.content_hash = Some("stix".into());
    let key = ingested.add_source(source);
    ingested.add_external_finding("same title", "stix details", Confidence::Medium, Some(key));
    store.apply_batch(context, &ingested).expect("ingest");
    let data = store.load_investigation(investigation.id).expect("load");
    assert_eq!(data.findings.len(), 2);
    let collected = data
        .findings
        .iter()
        .find(|finding| finding.provenance == Provenance::Collected)
        .expect("collected");
    assert_eq!(collected.details, "collected details");
    let external = data
        .findings
        .iter()
        .find(|finding| finding.provenance == Provenance::External)
        .expect("external");
    assert_eq!(external.details, "stix details");
}

#[test]
fn apply_rejects_unsafe_snapshot_and_artifact_paths() {
    let (mut store, investigation) = store_with_investigation();
    let context = ApplyContext {
        investigation_id: investigation.id,
        page_id: None,
        job_id: None,
    };
    let mut batch = EvidenceBatch::new();
    let mut source = NewSource::collected(SourceKind::Document, "file", "hostile path");
    source.content_hash = Some("path-safety".into());
    source.snapshot_path = Some("../../etc/passwd".into());
    batch.add_source(source);
    batch.artifacts.push(NewArtifact {
        kind: ArtifactKind::File,
        name: "passwd".into(),
        path: "../../etc/passwd".into(),
        mime: None,
        size_bytes: 0,
        sha256: None,
        fields: crate::model::Fields::new(),
        source: Some(0),
    });
    store.apply_batch(context, &batch).expect("apply");
    let data = store.load_investigation(investigation.id).expect("load");
    assert!(
        data.sources[0]
            .snapshot_path
            .as_deref()
            .is_none_or(|path| path != "../../etc/passwd")
    );
    assert!(data.sources[0].snapshot_path.is_none());
    assert!(data.artifacts.is_empty());
}

#[test]
fn marking_defaults_to_unmarked_and_persists() {
    let (mut store, investigation) = store_with_investigation();
    let loaded = store
        .get_investigation(investigation.id)
        .expect("get")
        .expect("present");
    assert_eq!(loaded.marking, Marking::Unmarked);
    store
        .set_marking(investigation.id, Marking::TlpAmber)
        .expect("set");
    let loaded = store
        .get_investigation(investigation.id)
        .expect("get")
        .expect("present");
    assert_eq!(loaded.marking, Marking::TlpAmber);
    let audit = store.list_audit(investigation.id).expect("audit");
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].kind, AuditKind::MarkingChanged);
    assert_eq!(audit[0].details.get("to"), Some("tlp:amber"));
}

#[test]
fn audit_records_merge_accept_and_export() {
    let (mut store, investigation) = store_with_investigation();
    store.set_identity(Identity {
        user_id: "analyst".into(),
        machine_name: "lab".into(),
    });
    let domain = store
        .add_entity(
            investigation.id,
            EntityKind::Domain,
            "example.com",
            Provenance::Analyst,
            None,
        )
        .expect("domain");
    let host = store
        .add_entity(
            investigation.id,
            EntityKind::Hostname,
            "example.com",
            Provenance::Collected,
            None,
        )
        .expect("host");
    store
        .resolve_entities(
            investigation.id,
            &[domain, host],
            domain,
            "same host",
            "analyst",
        )
        .expect("merge");
    let mut run = crate::model::AgentRun {
        id: Uuid::new_v4(),
        investigation_id: investigation.id,
        seq: 0,
        thread_id: Uuid::new_v4(),
        page_id: None,
        agent_name: "test".into(),
        program: "claude".into(),
        args: Vec::new(),
        question: "q".into(),
        response: String::new(),
        status: JobStatus::Succeeded,
        error: None,
        focus: None,
        context_artifact_id: None,
        job_id: None,
        model: None,
        cost_usd: None,
        created_at: crate::model::now(),
        finished_at: None,
    };
    store.insert_agent_run(&mut run).expect("run");
    let proposals = store
        .insert_proposals(
            investigation.id,
            run.id,
            vec![(
                AgentAction::AddNote {
                    body: "cited".into(),
                },
                ProposalStatus::Pending,
                None,
            )],
        )
        .expect("proposals");
    store
        .update_proposal(proposals[0].id, ProposalStatus::Accepted, None)
        .expect("accept");
    store
        .record_export(investigation.id, "/tmp/handoff")
        .expect("export audit");
    let kinds: Vec<_> = store
        .list_audit(investigation.id)
        .expect("audit")
        .into_iter()
        .map(|entry| entry.kind)
        .collect();
    assert!(kinds.contains(&AuditKind::EntitiesMerged));
    assert!(kinds.contains(&AuditKind::ActionAccepted));
    assert!(kinds.contains(&AuditKind::Export));
    let export_row = store
        .list_audit(investigation.id)
        .expect("audit")
        .into_iter()
        .find(|entry| entry.kind == AuditKind::Export)
        .expect("export row");
    assert_eq!(export_row.actor, "analyst");
    assert_eq!(export_row.machine, "lab");
}

#[test]
fn audit_log_rejects_updates() {
    let (mut store, investigation) = store_with_investigation();
    store
        .set_marking(investigation.id, Marking::TlpGreen)
        .expect("mark");
    let error = store
        .connection()
        .execute("UPDATE audit_log SET summary = 'rewritten'", [])
        .expect_err("append-only");
    assert!(
        error.to_string().contains("append-only"),
        "unexpected error: {error}"
    );
}

#[test]
fn empty_members_is_single_user_owner() {
    let (store, investigation) = store_with_investigation();
    assert!(
        store
            .list_members(investigation.id)
            .expect("members")
            .is_empty()
    );
    assert_eq!(store.role_for(investigation.id).expect("role"), Role::Owner);
}

#[test]
fn shared_file_reader_cannot_mutate_and_analyst_can() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("os-int.sqlite3");
    let mut owner = Store::open(&path).expect("owner");
    owner.set_identity(Identity {
        user_id: "alice".into(),
        machine_name: "desk-a".into(),
    });
    let investigation = owner.create_investigation("Shared", "").expect("created");
    owner
        .add_member(investigation.id, "bob", Role::Analyst)
        .expect("bob");
    owner
        .add_member(investigation.id, "carol", Role::Reader)
        .expect("carol");
    drop(owner);
    let mut analyst = Store::open(&path).expect("analyst");
    analyst.set_identity(Identity {
        user_id: "bob".into(),
        machine_name: "desk-b".into(),
    });
    assert_eq!(
        analyst.role_for(investigation.id).expect("role"),
        Role::Analyst
    );
    analyst
        .add_entity(
            investigation.id,
            EntityKind::Domain,
            "example.net",
            Provenance::Analyst,
            None,
        )
        .expect("analyst write");
    let mut reader = Store::open(&path).expect("reader");
    reader.set_identity(Identity {
        user_id: "carol".into(),
        machine_name: "desk-c".into(),
    });
    assert_eq!(
        reader.role_for(investigation.id).expect("role"),
        Role::Reader
    );
    let loaded = reader
        .load_investigation(investigation.id)
        .expect("reader load");
    assert_eq!(loaded.investigation.name, "Shared");
    let error = reader
        .add_entity(
            investigation.id,
            EntityKind::Ip,
            "192.0.2.9",
            Provenance::Analyst,
            None,
        )
        .expect_err("reader write");
    assert!(error.to_string().contains("reader"));
    let error = reader
        .set_marking(investigation.id, Marking::TlpRed)
        .expect_err("reader marking");
    assert!(error.to_string().contains("owner"));
}

#[test]
fn object_marking_lives_in_the_property_bag() {
    let (mut store, investigation) = store_with_investigation();
    let entity_id = store
        .add_entity(
            investigation.id,
            EntityKind::Ip,
            "192.0.2.1",
            Provenance::Collected,
            None,
        )
        .expect("entity");
    store
        .set_entity_marking(entity_id, Marking::TlpRed)
        .expect("set");
    let data = store.load_investigation(investigation.id).expect("load");
    assert_eq!(data.entities[0].marking(), Marking::TlpRed);
    assert_eq!(
        data.entities[0]
            .fields
            .get(crate::model::OBJECT_MARKING_FIELD),
        Some("tlp:red")
    );
}

#[test]
fn last_owner_cannot_be_removed() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("os-int.sqlite3");
    let mut owner = Store::open(&path).expect("owner");
    owner.set_identity(Identity {
        user_id: "alice".into(),
        machine_name: "desk-a".into(),
    });
    let investigation = owner.create_investigation("Owned", "").expect("created");
    owner
        .add_member(investigation.id, "bob", Role::Analyst)
        .expect("bob");
    let error = owner
        .remove_member(investigation.id, "alice")
        .expect_err("last owner");
    assert!(error.to_string().contains("owner"));
    owner
        .add_member(investigation.id, "dana", Role::Owner)
        .expect("second owner");
    owner
        .remove_member(investigation.id, "alice")
        .expect("can leave when another owner remains");
}

#[test]
fn analyst_cannot_change_investigation_marking() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("os-int.sqlite3");
    let mut owner = Store::open(&path).expect("owner");
    owner.set_identity(Identity {
        user_id: "alice".into(),
        machine_name: "desk-a".into(),
    });
    let investigation = owner.create_investigation("Marked", "").expect("created");
    owner
        .add_member(investigation.id, "bob", Role::Analyst)
        .expect("bob");
    drop(owner);
    let mut analyst = Store::open(&path).expect("analyst");
    analyst.set_identity(Identity {
        user_id: "bob".into(),
        machine_name: "desk-b".into(),
    });
    let error = analyst
        .set_marking(investigation.id, Marking::TlpRed)
        .expect_err("analyst marking");
    assert!(error.to_string().contains("owner"));
    let entity_id = analyst
        .add_entity(
            investigation.id,
            EntityKind::Domain,
            "example.org",
            Provenance::Analyst,
            None,
        )
        .expect("entity");
    analyst
        .set_entity_marking(entity_id, Marking::TlpGreen)
        .expect("object marking");
}

#[test]
fn reader_export_does_not_append_audit() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("os-int.sqlite3");
    let mut owner = Store::open(&path).expect("owner");
    owner.set_identity(Identity {
        user_id: "alice".into(),
        machine_name: "desk-a".into(),
    });
    let investigation = owner.create_investigation("Handoff", "").expect("created");
    owner
        .add_member(investigation.id, "carol", Role::Reader)
        .expect("carol");
    drop(owner);
    let mut reader = Store::open(&path).expect("reader");
    reader.set_identity(Identity {
        user_id: "carol".into(),
        machine_name: "desk-c".into(),
    });
    reader
        .record_export(investigation.id, "/tmp/out")
        .expect("reader export");
    let audit = reader.list_audit(investigation.id).expect("audit");
    assert!(
        !audit.iter().any(|entry| entry.kind == AuditKind::Export),
        "readers may export without writing an export row"
    );
}

#[test]
fn update_investigation_does_not_change_marking() {
    let (mut store, mut investigation) = store_with_investigation();
    store
        .set_marking(investigation.id, Marking::TlpAmber)
        .expect("set");
    investigation.name = "Renamed".into();
    investigation.marking = Marking::TlpRed;
    store.update_investigation(&investigation).expect("update");
    let loaded = store
        .get_investigation(investigation.id)
        .expect("get")
        .expect("present");
    assert_eq!(loaded.name, "Renamed");
    assert_eq!(loaded.marking, Marking::TlpAmber);
}

fn reader_on_shared_case() -> (tempfile::TempDir, Uuid, Uuid, Store) {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("os-int.sqlite3");
    let mut owner = Store::open(&path).expect("owner");
    owner.set_identity(Identity {
        user_id: "alice".into(),
        machine_name: "desk-a".into(),
    });
    let investigation = owner.create_investigation("Shared", "").expect("created");
    owner
        .add_member(investigation.id, "carol", Role::Reader)
        .expect("carol");
    let page = owner
        .create_page(NewPage::new(
            investigation.id,
            "example.com",
            PageKind::Domain,
            Some("example.com"),
        ))
        .expect("page");
    drop(owner);
    let mut reader = Store::open(&path).expect("reader");
    reader.set_identity(Identity {
        user_id: "carol".into(),
        machine_name: "desk-c".into(),
    });
    (directory, investigation.id, page.id, reader)
}

#[test]
fn reader_mutations_are_refused_and_leave_rows_unchanged() {
    let (_directory, investigation_id, page_id, mut reader) = reader_on_shared_case();
    let before = reader.load_investigation(investigation_id).expect("load");
    assert!(
        reader
            .create_finding(NewFinding {
                investigation_id,
                page_id: Some(page_id),
                title: "reader finding".into(),
                details: String::new(),
                confidence: Confidence::Low,
                provenance: Provenance::Analyst,
                source_ids: Vec::new(),
            })
            .is_err()
    );
    assert!(
        reader
            .create_note(
                investigation_id,
                Some(page_id),
                "reader note",
                Provenance::Analyst,
            )
            .is_err()
    );
    assert!(reader.update_page_notes(page_id, "reader notes").is_err());
    let mut job = JobRecord {
        id: Uuid::new_v4(),
        investigation_id: Some(investigation_id),
        seq: 0,
        kind: "collector:dns".into(),
        label: "DNS".into(),
        description: "DNS".into(),
        status: JobStatus::Queued,
        program: None,
        args: Vec::new(),
        created_at: crate::model::now(),
        started_at: None,
        finished_at: None,
        progress: None,
        log: String::new(),
        stderr: String::new(),
        result: None,
        error: None,
        page_id: Some(page_id),
    };
    assert!(reader.insert_job(&mut job).is_err());
    assert!(
        reader
            .add_artifact(
                investigation_id,
                Some(page_id),
                &NewArtifact {
                    kind: ArtifactKind::File,
                    name: "note.txt".into(),
                    path: "files/note.txt".into(),
                    mime: None,
                    size_bytes: 0,
                    sha256: None,
                    fields: crate::model::Fields::new(),
                    source: None,
                },
            )
            .is_err()
    );
    assert!(reader.delete_investigation(investigation_id).is_err());
    let after = reader.load_investigation(investigation_id).expect("still");
    assert_eq!(after.findings.len(), before.findings.len());
    assert_eq!(after.notes.len(), before.notes.len());
    assert_eq!(after.pages[0].notes, before.pages[0].notes);
    assert_eq!(after.jobs.len(), before.jobs.len());
    assert_eq!(after.artifacts.len(), before.artifacts.len());
    assert_eq!(after.investigation.name, "Shared");
}

#[test]
fn analyst_cannot_add_member() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("os-int.sqlite3");
    let mut owner = Store::open(&path).expect("owner");
    owner.set_identity(Identity {
        user_id: "alice".into(),
        machine_name: "desk-a".into(),
    });
    let investigation = owner.create_investigation("Team", "").expect("created");
    owner
        .add_member(investigation.id, "bob", Role::Analyst)
        .expect("bob");
    drop(owner);
    let mut analyst = Store::open(&path).expect("analyst");
    analyst.set_identity(Identity {
        user_id: "bob".into(),
        machine_name: "desk-b".into(),
    });
    let error = analyst
        .add_member(investigation.id, "dana", Role::Reader)
        .expect_err("analyst team");
    assert!(error.to_string().contains("owner"));
    assert_eq!(
        analyst
            .list_members(investigation.id)
            .expect("members")
            .len(),
        2
    );
}

#[test]
fn playbook_apply_writes_playbook_run_audit() {
    let (mut store, investigation) = store_with_investigation();
    let playbook = crate::playbook::bundled("domain-infra").expect("bundled");
    let seed = Target::parse(TargetKind::Domain, "example.com").expect("seed");
    store
        .apply_playbook_outcome(
            ApplyContext {
                investigation_id: investigation.id,
                page_id: None,
                job_id: None,
            },
            &[],
            PlaybookApplyMeta {
                playbook: &playbook,
                seed: &seed,
                automation: false,
                digest: "digest",
                previous_digest: None,
            },
        )
        .expect("apply");
    let kinds: Vec<_> = store
        .list_audit(investigation.id)
        .expect("audit")
        .into_iter()
        .map(|entry| entry.kind)
        .collect();
    assert!(kinds.contains(&AuditKind::PlaybookRun));
}

#[test]
fn importer_of_a_reader_archive_becomes_owner() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("os-int.sqlite3");
    let mut owner = Store::open(&path).expect("owner");
    owner.set_identity(Identity {
        user_id: "alice".into(),
        machine_name: "desk-a".into(),
    });
    let investigation = owner.create_investigation("Handoff", "").expect("created");
    owner
        .add_member(investigation.id, "carol", Role::Reader)
        .expect("carol");
    let export_dir = directory.path().join("archive");
    crate::exchange::export_investigation(
        &mut owner,
        investigation.id,
        directory.path(),
        &export_dir,
    )
    .expect("export");
    drop(owner);
    let mut imported = Store::open_in_memory().expect("new store");
    imported.set_identity(Identity {
        user_id: "carol".into(),
        machine_name: "desk-c".into(),
    });
    let paths =
        crate::paths::AppPaths::new(directory.path().join("cfg"), directory.path().join("data"));
    paths.ensure_dirs().expect("dirs");
    let data = crate::exchange::import_investigation(&mut imported, &export_dir, &paths, false)
        .expect("import");
    assert_eq!(
        imported.role_for(data.investigation.id).expect("role"),
        Role::Owner
    );
    imported
        .create_finding(NewFinding {
            investigation_id: data.investigation.id,
            page_id: None,
            title: "importer note".into(),
            details: String::new(),
            confidence: Confidence::Low,
            provenance: Provenance::Analyst,
            source_ids: Vec::new(),
        })
        .expect("owner can write");
}
