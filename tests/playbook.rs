//! Integration tests for playbook YAML on disk and transactional apply.

use os_int::config::Config;
use os_int::model::{
    Confidence, EntityKey, EntityKind, EvidenceBatch, NewEntity, NewRelationship, NewSource,
    PageKind, Provenance, SourceKind, Target, TargetKind, relation,
};
use os_int::playbook::{bundled, execute_prepared, load_playbook, parse_playbook_yaml};
use os_int::storage::{ApplyContext, NewPage, Store};
use os_int::tools::ToolRegistry;

fn fixture_batch() -> EvidenceBatch {
    let mut batch = EvidenceBatch::new();
    batch.sources.push({
        let mut source = NewSource::collected(SourceKind::Dns, "dns", "DNS example.com");
        source.content = "A 192.0.2.1".into();
        source.content_hash = Some("abc123".into());
        source
    });
    let domain = EntityKey::new(EntityKind::Domain, "example.com");
    let ip = EntityKey::new(EntityKind::Ip, "192.0.2.1");
    batch.entities.push(NewEntity {
        key: domain.clone(),
        label: None,
        fields: os_int::model::Fields::new(),
        sources: vec![0],
        link_to_page: true,
    });
    batch.entities.push(NewEntity {
        key: ip.clone(),
        label: None,
        fields: os_int::model::Fields::new(),
        sources: vec![0],
        link_to_page: false,
    });
    batch.relationships.push(NewRelationship {
        from: domain,
        kind: relation::RESOLVES_TO.into(),
        to: ip,
        sources: vec![0],
        confidence: Confidence::High,
    });
    batch.summary = "fixture domain infra".into();
    batch
}

#[test]
fn bundled_yaml_files_on_disk_parse() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("playbooks");
    for id in ["domain-infra", "mail-policy", "cert-estate", "repo-surface"] {
        let path = root.join(format!("{id}.yaml"));
        let from_file = load_playbook(path.to_str().expect("utf-8")).expect("file");
        let from_bundle = bundled(id).expect("bundled");
        assert_eq!(from_file.id, from_bundle.id);
        assert_eq!(from_file.seed.kind, from_bundle.seed.kind);
        assert_eq!(from_file.steps.len(), from_bundle.steps.len());
    }
}

#[test]
fn domain_infra_fixture_is_cited_graph_with_pending_proposals_only() {
    let mut store = Store::open_in_memory().expect("store");
    let investigation = store
        .create_investigation("Example Corp", "")
        .expect("investigation");
    let page = store
        .create_page(NewPage::new(
            investigation.id,
            "example.com",
            PageKind::Domain,
            Some("example.com"),
        ))
        .expect("page");
    let playbook = bundled("domain-infra").expect("playbook");
    let seed = Target::parse(TargetKind::Domain, "example.com").expect("seed");
    let tools = ToolRegistry::detect(&Config::default().tools);
    let output = execute_prepared(
        &mut store,
        &playbook,
        &seed,
        ApplyContext {
            investigation_id: investigation.id,
            page_id: Some(page.id),
            job_id: None,
        },
        vec![fixture_batch()],
        &tools,
        os_int::playbook::ExecuteOptions::default(),
    )
    .expect("execute");
    assert!(!output.dry_run);
    let data = store.load_investigation(investigation.id).expect("load");
    assert!(
        data.entities
            .iter()
            .any(|entity| entity.value == "example.com")
    );
    assert!(
        data.relationships
            .iter()
            .any(|relationship| relationship.kind == relation::RESOLVES_TO)
    );
    assert!(
        data.sources
            .iter()
            .any(|source| source.provenance == Provenance::Collected)
    );
    assert!(data.proposals.iter().any(|proposal| proposal.action.name()
        == "ASSERT_FINDING_TEMPLATE"
        && proposal.status == os_int::model::ProposalStatus::Pending));
    assert!(
        !data
            .findings
            .iter()
            .any(|finding| finding.title == "Review domain infrastructure")
    );
    assert!(data.resolutions.is_empty());
}

#[test]
fn invalid_yaml_never_reaches_apply() {
    let actual = parse_playbook_yaml(b"[unterminated: : :").expect_err("invalid");
    assert!(
        actual.message.to_ascii_lowercase().contains("yaml")
            || actual.message.contains("mapping")
            || actual.message.contains("format")
    );
}

#[test]
fn path_traversal_spec_is_refused() {
    let actual = load_playbook("../evil.yaml").expect_err("traverse");
    assert!(
        actual.message.contains("traversal")
            || actual.message.contains("..")
            || actual.message.contains("bundled")
    );
}
