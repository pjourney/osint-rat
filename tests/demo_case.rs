//! The bundled demo archive imports into a cited graph with resolution and playbook output.

use std::path::Path;
use std::process::Command;

use os_int::demo::{demo_case_data, demo_case_manifest, demo_domain_id};
use os_int::exchange::{self, FORMAT_VERSION};
use os_int::ingest::parse_batch_json;
use os_int::model::{Provenance, SearchAroundQuery, TimelineKind, Workspace, search_around};
use os_int::paths::AppPaths;
use os_int::playbook::parse_playbook_yaml;
use os_int::storage::{ApplyContext, Store};

fn demo_archive_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/demo-case/investigation.osint.json")
}

#[test]
fn checked_in_demo_archive_matches_builder() {
    let text = std::fs::read_to_string(demo_archive_path()).expect("demo archive");
    let file: exchange::ArchiveManifest = serde_json::from_str(&text).expect("parse archive");
    let built = demo_case_manifest();
    assert_eq!(file.format, "os-int-investigation");
    assert_eq!(file.format_version, FORMAT_VERSION);
    assert_eq!(file.data, built.data);
}

#[test]
fn import_demo_case_shows_graph_search_around_and_cited_findings() {
    let directory = tempfile::tempdir().expect("tempdir");
    let mut store = Store::open_in_memory().expect("store");
    let paths = AppPaths::new(directory.path().join("cfg"), directory.path().join("data"));
    paths.ensure_dirs().expect("dirs");
    let imported = exchange::import_investigation(&mut store, &demo_archive_path(), &paths, false)
        .expect("import demo");
    assert_eq!(imported.investigation.name, "Example Corp");
    assert_eq!(imported, demo_case_data());
    assert!(
        imported
            .findings
            .iter()
            .all(|finding| !finding.source_ids.is_empty())
    );
    assert!(
        imported
            .findings
            .iter()
            .all(|finding| finding.provenance == Provenance::Collected)
    );
    let cited = imported
        .findings
        .iter()
        .filter(|finding| !finding.source_ids.is_empty())
        .count();
    assert_eq!(cited, imported.findings.len());
    assert_eq!(cited, 3);
    assert!(imported.timeline.iter().any(
        |event| event.kind == TimelineKind::PlaybookRun && event.title.contains("domain-infra")
    ));
    assert_eq!(imported.resolutions.len(), 1);
    assert!(imported.resolutions[0].is_active());
    let workspace = Workspace::new(imported.clone());
    let domain = workspace
        .entity(demo_domain_id())
        .expect("canonical domain");
    assert_eq!(domain.value, "example.com");
    assert_eq!(workspace.constituents_of(domain.id).len(), 1);
    let around = search_around(&imported, &SearchAroundQuery::new(vec![domain.id], 2));
    assert!(around.hops_used >= 2);
    assert!(
        around.uncapped_node_count >= 10,
        "Search Around should show connected infra, got {}",
        around.uncapped_node_count
    );
    let view = os_int::model::ObjectView::for_entity(&workspace, domain);
    assert!(
        view.groups
            .iter()
            .any(|group| !group.properties.is_empty() || group.id == "identity")
    );
    assert!(view.links.iter().any(|group| !group.links.is_empty()));
}

#[test]
fn hello_batch_connector_still_applies_to_the_demo_case() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/connectors/hello-batch/hello_batch.py");
    let output = Command::new("python3")
        .args([
            script.to_str().expect("utf-8"),
            "--target",
            "example.com",
            "--kind",
            "domain",
        ])
        .output()
        .expect("python3 hello_batch.py");
    assert!(
        output.status.success(),
        "hello-batch failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = parse_batch_json(&output.stdout, "hello-batch").expect("batch");
    assert_eq!(report.batch.sources[0].provenance, Provenance::External);
    let directory = tempfile::tempdir().expect("tempdir");
    let mut store = Store::open_in_memory().expect("store");
    let paths = AppPaths::new(directory.path().join("cfg"), directory.path().join("data"));
    paths.ensure_dirs().expect("dirs");
    let imported = exchange::import_investigation(&mut store, &demo_archive_path(), &paths, false)
        .expect("import");
    store
        .apply_batch(
            ApplyContext {
                investigation_id: imported.investigation.id,
                page_id: imported.pages.first().map(|page| page.id),
                job_id: None,
            },
            &report.batch,
        )
        .expect("apply connector");
    let after = store
        .load_investigation(imported.investigation.id)
        .expect("reload");
    assert!(
        after
            .sources
            .iter()
            .any(|source| source.provenance == Provenance::External
                && source.collector.as_deref() == Some("connector:hello-batch"))
    );
    assert!(
        after
            .findings
            .iter()
            .filter(|finding| finding.provenance == Provenance::Collected)
            .all(|finding| !finding.source_ids.is_empty())
    );
}

#[test]
fn community_plugin_templates_parse() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/community");
    let playbook = std::fs::read(root.join("template-playbook.yaml")).expect("playbook");
    let parsed = parse_playbook_yaml(&playbook).expect("yaml");
    assert_eq!(parsed.id, "community-template");
    assert!(!parsed.steps.is_empty());
    let script = root.join("template-collector/collect.py");
    let output = Command::new("python3")
        .args([
            script.to_str().expect("utf-8"),
            "--target",
            "example.com",
            "--kind",
            "domain",
        ])
        .output()
        .expect("python3 collect.py");
    assert!(
        output.status.success(),
        "template collector failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = parse_batch_json(&output.stdout, "template").expect("batch");
    assert_eq!(report.batch.sources[0].provenance, Provenance::External);
    assert!(!report.batch.entities.is_empty());
    assert!(!report.batch.relationships.is_empty());
}

#[test]
fn install_and_release_docs_cover_one_liners() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let install = std::fs::read_to_string(root.join("scripts/install.sh")).expect("install.sh");
    assert!(install.contains("pjourney/osint-rat"));
    assert!(install.contains("x86_64-unknown-linux-gnu"));
    assert!(install.contains("aarch64-apple-darwin"));
    assert!(install.contains("sha256"));
    let release =
        std::fs::read_to_string(root.join(".github/workflows/release.yml")).expect("release");
    assert!(release.contains("x86_64-unknown-linux-gnu"));
    assert!(release.contains("x86_64-pc-windows-msvc"));
    assert!(
        release.contains("sha256sum")
            || release.contains("Get-FileHash")
            || release.contains("shasum")
    );
    let ci = std::fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("ci");
    assert!(ci.contains("cargo audit") || ci.contains("audit-check"));
    let dependabot =
        std::fs::read_to_string(root.join(".github/dependabot.yml")).expect("dependabot");
    assert!(dependabot.contains("cargo"));
    let cargo = std::fs::read_to_string(root.join("Cargo.toml")).expect("cargo");
    assert!(cargo.contains("[package.metadata.binstall]"));
    assert!(cargo.contains("{ name }-{ target }/{ bin }{ binary-ext }"));
}
