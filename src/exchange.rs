//! Versioned investigation import/export. Archives are directories (or a `.osint.json` plus a
//! `files/` tree) whose paths are relative, so they can be shared without the originating
//! computer's absolute paths.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, WrapErr, bail};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::files::{InvestigationFiles, write_atomically};
use crate::model::{InvestigationData, now};
use crate::paths::{is_safe_relative_path, sanitize_filename};
use crate::storage::Store;

/// On-disk investigation archive version (`os-int-investigation`).
///
/// Import rejects any archive whose `format_version` is greater than this value.
/// Version 4 adds `data.audit`, `data.members`, and `investigation.marking`.
/// Version 3 added `data.events`. Version 2 added `data.resolutions`.
/// Bump rules: `docs/ontology-exchange.md`.
pub const FORMAT_VERSION: u32 = 4;
pub const MANIFEST_NAME: &str = "investigation.osint.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArchiveManifest {
    pub format: String,
    pub format_version: u32,
    pub exported_at: String,
    pub data: InvestigationData,
}

impl ArchiveManifest {
    fn wrap(data: InvestigationData) -> Self {
        Self {
            format: "os-int-investigation".into(),
            format_version: FORMAT_VERSION,
            exported_at: now().to_rfc3339(),
            data,
        }
    }
}

/// Writes `path/investigation.osint.json` plus a copy of the investigation files directory.
pub fn export_investigation(
    store: &mut Store,
    investigation_id: Uuid,
    files_root: &Path,
    destination: &Path,
) -> Result<PathBuf> {
    store.record_export(investigation_id, &destination.display().to_string())?;
    let data = store.load_investigation(investigation_id)?;
    export_data(&data, files_root, destination)
}

pub fn export_data(
    data: &InvestigationData,
    files_root: &Path,
    destination: &Path,
) -> Result<PathBuf> {
    fs::create_dir_all(destination)
        .wrap_err_with(|| format!("could not create {}", destination.display()))?;
    let manifest = ArchiveManifest::wrap(data.clone());
    let json =
        serde_json::to_string_pretty(&manifest).wrap_err("could not serialize investigation")?;
    let manifest_path = destination.join(MANIFEST_NAME);
    write_atomically(&manifest_path, json.as_bytes())
        .wrap_err_with(|| format!("could not write {}", manifest_path.display()))?;
    if files_root.exists() {
        copy_tree(files_root, &destination.join("files"))?;
    }
    Ok(manifest_path)
}

/// Suggested directory name for an export.
pub fn suggested_export_name(data: &InvestigationData) -> String {
    format!(
        "{}-{}",
        sanitize_filename(&data.investigation.name),
        data.investigation.id.simple()
    )
}

/// Imports an archive. If an investigation with the same ID exists it is rejected unless
/// `rename` is set, in which case every record UUID is remapped and files are copied under
/// the new investigation id.
pub fn import_investigation(
    store: &mut Store,
    archive: &Path,
    data_files: &crate::paths::AppPaths,
    rename: bool,
) -> Result<InvestigationData> {
    let (manifest_path, files_dir) = locate_archive(archive)?;
    let text = fs::read_to_string(&manifest_path)
        .wrap_err_with(|| format!("could not read {}", manifest_path.display()))?;
    let mut manifest: ArchiveManifest =
        serde_json::from_str(&text).wrap_err("investigation archive is not valid JSON")?;
    if manifest.format_version > FORMAT_VERSION {
        bail!(
            "this archive uses format version {}, but os-int understands up to {FORMAT_VERSION}",
            manifest.format_version
        );
    }
    validate_paths(&manifest.data)?;
    if store
        .get_investigation(manifest.data.investigation.id)?
        .is_some()
    {
        if !rename {
            bail!(
                "investigation {} already exists; pass --rename to import a copy",
                manifest.data.investigation.name
            );
        }
        reassign_ids(&mut manifest.data);
    }
    if store
        .find_investigation(&manifest.data.investigation.name)?
        .is_some()
    {
        manifest.data.investigation.name = unique_name(store, &manifest.data.investigation.name)?;
    }
    store.insert_investigation_data(&manifest.data)?;
    let destination = InvestigationFiles::new(data_files, manifest.data.investigation.id);
    destination
        .ensure()
        .wrap_err("could not create investigation directory")?;
    if files_dir.exists() {
        copy_tree(&files_dir, destination.root())?;
    }
    Ok(manifest.data)
}

fn locate_archive(path: &Path) -> Result<(PathBuf, PathBuf)> {
    if path.is_file() {
        let files = path.parent().unwrap_or(Path::new(".")).join("files");
        return Ok((path.to_path_buf(), files));
    }
    let manifest = path.join(MANIFEST_NAME);
    if manifest.exists() {
        return Ok((manifest, path.join("files")));
    }
    bail!("{} is not an os-int investigation archive", path.display())
}

fn validate_paths(data: &InvestigationData) -> Result<()> {
    for artifact in &data.artifacts {
        if !is_safe_relative_path(&artifact.path) {
            bail!(
                "archive contains an unsafe artifact path: {}",
                artifact.path
            );
        }
    }
    for source in &data.sources {
        if let Some(path) = &source.snapshot_path
            && !is_safe_relative_path(path)
        {
            bail!("archive contains an unsafe snapshot path: {path}");
        }
    }
    Ok(())
}

fn unique_name(store: &Store, base: &str) -> Result<String> {
    for suffix in 2..1000 {
        let candidate = format!("{base} ({suffix})");
        if store.find_investigation(&candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    Ok(format!("{base} ({})", Uuid::new_v4().simple()))
}

fn reassign_ids(data: &mut InvestigationData) {
    let new_investigation = Uuid::new_v4();
    data.investigation.id = new_investigation;
    data.investigation.name = format!("{} (imported)", data.investigation.name);
    let mut pages = HashMap::new();
    let mut sources = HashMap::new();
    let mut findings = HashMap::new();
    let mut entities = HashMap::new();
    let mut relationships = HashMap::new();
    let mut notes = HashMap::new();
    let mut artifacts = HashMap::new();
    let mut timeline = HashMap::new();
    let mut jobs = HashMap::new();
    let mut runs = HashMap::new();
    let mut proposals = HashMap::new();
    let mut resolutions = HashMap::new();
    let mut events = HashMap::new();
    let mut audit = HashMap::new();
    let mut threads = HashMap::new();
    for page in &mut data.pages {
        page.id = next_id(&mut pages, page.id);
        page.investigation_id = new_investigation;
    }
    for source in &mut data.sources {
        source.id = next_id(&mut sources, source.id);
        source.investigation_id = new_investigation;
    }
    for finding in &mut data.findings {
        finding.id = next_id(&mut findings, finding.id);
        finding.investigation_id = new_investigation;
    }
    for entity in &mut data.entities {
        entity.id = next_id(&mut entities, entity.id);
        entity.investigation_id = new_investigation;
    }
    for relationship in &mut data.relationships {
        relationship.id = next_id(&mut relationships, relationship.id);
        relationship.investigation_id = new_investigation;
    }
    for note in &mut data.notes {
        note.id = next_id(&mut notes, note.id);
        note.investigation_id = new_investigation;
    }
    for artifact in &mut data.artifacts {
        artifact.id = next_id(&mut artifacts, artifact.id);
        artifact.investigation_id = new_investigation;
    }
    for event in &mut data.timeline {
        event.id = next_id(&mut timeline, event.id);
        event.investigation_id = new_investigation;
    }
    for job in &mut data.jobs {
        job.id = next_id(&mut jobs, job.id);
        job.investigation_id = job.investigation_id.map(|_| new_investigation);
    }
    for run in &mut data.agent_runs {
        run.id = next_id(&mut runs, run.id);
        run.investigation_id = new_investigation;
        run.thread_id = next_id(&mut threads, run.thread_id);
    }
    for proposal in &mut data.proposals {
        proposal.id = next_id(&mut proposals, proposal.id);
        proposal.investigation_id = new_investigation;
    }
    for resolution in &mut data.resolutions {
        resolution.id = next_id(&mut resolutions, resolution.id);
        resolution.investigation_id = new_investigation;
    }
    for event in &mut data.events {
        event.id = next_id(&mut events, event.id);
        event.investigation_id = new_investigation;
    }
    for entry in &mut data.audit {
        entry.id = next_id(&mut audit, entry.id);
        entry.investigation_id = new_investigation;
    }
    for member in &mut data.members {
        member.investigation_id = new_investigation;
    }
    for page in &mut data.pages {
        page.entity_id = mapped_opt(&entities, page.entity_id);
    }
    for source in &mut data.sources {
        source.page_id = mapped_opt(&pages, source.page_id);
        source.job_id = mapped_opt(&jobs, source.job_id);
    }
    for finding in &mut data.findings {
        finding.page_id = mapped_opt(&pages, finding.page_id);
        remap_uuid_list(&sources, &mut finding.source_ids);
    }
    for entity in &mut data.entities {
        remap_uuid_list(&sources, &mut entity.source_ids);
    }
    for relationship in &mut data.relationships {
        relationship.from_id = mapped(&entities, relationship.from_id);
        relationship.to_id = mapped(&entities, relationship.to_id);
        remap_uuid_list(&sources, &mut relationship.source_ids);
    }
    for note in &mut data.notes {
        note.page_id = mapped_opt(&pages, note.page_id);
    }
    for artifact in &mut data.artifacts {
        artifact.page_id = mapped_opt(&pages, artifact.page_id);
        artifact.source_id = mapped_opt(&sources, artifact.source_id);
    }
    for event in &mut data.timeline {
        event.source_id = mapped_opt(&sources, event.source_id);
        event.entity_id = mapped_opt(&entities, event.entity_id);
        event.page_id = mapped_opt(&pages, event.page_id);
    }
    for job in &mut data.jobs {
        job.page_id = mapped_opt(&pages, job.page_id);
    }
    for run in &mut data.agent_runs {
        run.page_id = mapped_opt(&pages, run.page_id);
        run.context_artifact_id = mapped_opt(&artifacts, run.context_artifact_id);
        run.job_id = mapped_opt(&jobs, run.job_id);
    }
    for proposal in &mut data.proposals {
        proposal.agent_run_id = mapped(&runs, proposal.agent_run_id);
    }
    for link in &mut data.page_entities {
        link.page_id = mapped(&pages, link.page_id);
        link.entity_id = mapped(&entities, link.entity_id);
    }
    for resolution in &mut data.resolutions {
        resolution.canonical_id = mapped(&entities, resolution.canonical_id);
        remap_uuid_list(&entities, &mut resolution.constituent_ids);
        remap_uuid_list(&sources, &mut resolution.source_writes);
    }
    for event in &mut data.events {
        event.source_id = mapped_opt(&sources, event.source_id);
        event.page_id = mapped_opt(&pages, event.page_id);
        remap_uuid_list(&entities, &mut event.entity_ids);
    }
    for entry in &mut data.audit {
        remap_uuid_list(&entities, &mut entry.target_ids);
        remap_uuid_list(&resolutions, &mut entry.target_ids);
        remap_uuid_list(&proposals, &mut entry.target_ids);
        remap_uuid_list(&audit, &mut entry.target_ids);
    }
}

fn next_id(map: &mut HashMap<Uuid, Uuid>, old: Uuid) -> Uuid {
    *map.entry(old).or_insert_with(Uuid::new_v4)
}

fn mapped(map: &HashMap<Uuid, Uuid>, old: Uuid) -> Uuid {
    map.get(&old).copied().unwrap_or(old)
}

fn mapped_opt(map: &HashMap<Uuid, Uuid>, old: Option<Uuid>) -> Option<Uuid> {
    old.map(|id| mapped(map, id))
}

fn remap_uuid_list(map: &HashMap<Uuid, Uuid>, ids: &mut [Uuid]) {
    for id in ids.iter_mut() {
        *id = mapped(map, *id);
    }
}

const MAX_COPY_DEPTH: u32 = 32;

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    copy_tree_limited(from, to, 0)
}

fn copy_tree_limited(from: &Path, to: &Path, depth: u32) -> Result<()> {
    if depth > MAX_COPY_DEPTH {
        bail!("investigation file tree is too deep to copy");
    }
    fs::create_dir_all(to).wrap_err_with(|| format!("could not create {}", to.display()))?;
    let to_canon = to.canonicalize().ok();
    for entry in
        fs::read_dir(from).wrap_err_with(|| format!("could not read {}", from.display()))?
    {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == "." || name_str == ".." || name_str.starts_with("tmp-") {
            continue;
        }
        let source = entry.path();
        let destination = to.join(&name);
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            if let (Some(to_canon), Ok(source_canon)) = (&to_canon, source.canonicalize())
                && (to_canon.starts_with(&source_canon) || source_canon.starts_with(to_canon))
            {
                continue;
            }
            copy_tree_limited(&source, &destination, depth + 1)?;
        } else if metadata.is_file() {
            fs::copy(&source, &destination).wrap_err_with(|| {
                format!(
                    "could not copy {} to {}",
                    source.display(),
                    destination.display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PageKind;
    use crate::model::{Investigation, now};
    use crate::storage::{NewPage, Store};

    #[test]
    fn export_then_import_round_trips() {
        let mut store = Store::open_in_memory().expect("store");
        let investigation = store
            .create_investigation("Export Me", "desc")
            .expect("created");
        store
            .create_page(NewPage::new(
                investigation.id,
                "",
                PageKind::Domain,
                Some("example.com"),
            ))
            .expect("page");
        let data = store.load_investigation(investigation.id).expect("load");
        let directory = tempfile::tempdir().expect("tempdir");
        let files_root = directory.path().join("orig-files");
        fs::create_dir_all(files_root.join("reports")).expect("dirs");
        fs::write(files_root.join("reports/note.md"), b"hello").expect("write");
        let export_dir = directory.path().join("archive");
        export_data(&data, &files_root, &export_dir).expect("export");
        assert!(export_dir.join(MANIFEST_NAME).exists());
        assert!(export_dir.join("files/reports/note.md").exists());
        let mut target = Store::open_in_memory().expect("target");
        let paths = crate::paths::AppPaths::new(
            directory.path().join("cfg"),
            directory.path().join("data"),
        );
        paths.ensure_dirs().expect("dirs");
        let imported =
            import_investigation(&mut target, &export_dir, &paths, false).expect("import");
        assert_eq!(imported.investigation.name, "Export Me");
        assert_eq!(imported.pages.len(), 1);
        assert!(
            paths
                .investigation_dir(imported.investigation.id)
                .join("reports/note.md")
                .exists()
        );
    }

    #[test]
    fn imports_checked_in_v1_archive_without_format_bump_fields() {
        let directory = tempfile::tempdir().expect("tempdir");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/archives/v1-minimal/investigation.osint.json");
        let mut store = Store::open_in_memory().expect("store");
        let paths = crate::paths::AppPaths::new(
            directory.path().join("cfg"),
            directory.path().join("data"),
        );
        paths.ensure_dirs().expect("dirs");
        let imported =
            import_investigation(&mut store, &fixture, &paths, false).expect("import v1");
        assert_eq!(imported.investigation.name, "v1 fixture");
        assert_eq!(imported.pages.len(), 1);
        assert_eq!(imported.entities.len(), 2);
        assert_eq!(imported.relationships[0].kind, "resolves_to");
        assert!(imported.proposals.is_empty());
        let catalogued = store
            .list_link_types(imported.investigation.id)
            .expect("catalogued");
        assert!(catalogued.iter().any(|item| item.kind == "resolves_to"));
        let view = crate::model::ObjectView::for_entity(
            &crate::model::Workspace::new(imported.clone()),
            &imported.entities[0],
        );
        assert!(view.groups.iter().any(|group| group.id == "identity"));
        assert!(view.links.iter().any(|group| group.catalogued));
        assert!(imported.resolutions.is_empty());
        let export_dir = directory.path().join("re-export");
        export_data(&imported, directory.path(), &export_dir).expect("re-export");
        let text = fs::read_to_string(export_dir.join(MANIFEST_NAME)).expect("manifest");
        let manifest: ArchiveManifest = serde_json::from_str(&text).expect("parse");
        assert_eq!(manifest.format_version, FORMAT_VERSION);
        assert_eq!(FORMAT_VERSION, 4);
        assert!(manifest.data.resolutions.is_empty());
        assert!(manifest.data.events.is_empty());
        assert!(manifest.data.audit.is_empty());
        assert_eq!(
            imported.investigation.marking,
            crate::model::Marking::Unmarked
        );
    }

    #[test]
    fn export_import_preserves_resolution_bags() {
        let mut store = Store::open_in_memory().expect("store");
        let investigation = store
            .create_investigation("Resolve Me", "")
            .expect("created");
        let domain = store
            .add_entity(
                investigation.id,
                crate::model::EntityKind::Domain,
                "example.com",
                crate::model::Provenance::Analyst,
                None,
            )
            .expect("domain");
        let host = store
            .add_entity(
                investigation.id,
                crate::model::EntityKind::Hostname,
                "example.com",
                crate::model::Provenance::Collected,
                None,
            )
            .expect("host");
        store
            .resolve_entities(
                investigation.id,
                &[domain, host],
                domain,
                "same host",
                "tester",
            )
            .expect("merge");
        let data = store.load_investigation(investigation.id).expect("load");
        assert_eq!(data.resolutions.len(), 1);
        let directory = tempfile::tempdir().expect("tempdir");
        let export_dir = directory.path().join("archive");
        export_data(&data, directory.path(), &export_dir).expect("export");
        let text = fs::read_to_string(export_dir.join(MANIFEST_NAME)).expect("manifest");
        let manifest: ArchiveManifest = serde_json::from_str(&text).expect("parse");
        assert_eq!(manifest.format_version, FORMAT_VERSION);
        assert_eq!(manifest.data.resolutions.len(), 1);
        let mut target = Store::open_in_memory().expect("target");
        let paths = crate::paths::AppPaths::new(
            directory.path().join("cfg"),
            directory.path().join("data"),
        );
        paths.ensure_dirs().expect("dirs");
        let imported =
            import_investigation(&mut target, &export_dir, &paths, false).expect("import");
        assert_eq!(imported.resolutions.len(), 1);
        assert_eq!(imported.resolutions[0].canonical_id, domain);
        assert!(imported.resolutions[0].constituent_ids.contains(&host));
        assert!(imported.is_absorbed(host));
    }

    #[test]
    fn export_import_preserves_events_and_rename_remaps_them() {
        let mut store = Store::open_in_memory().expect("store");
        let investigation = store.create_investigation("Events", "").expect("created");
        let ip = store
            .add_entity(
                investigation.id,
                crate::model::EntityKind::Ip,
                "192.0.2.1",
                crate::model::Provenance::Collected,
                None,
            )
            .expect("ip");
        let started = now();
        store
            .add_event(
                investigation.id,
                crate::model::NewEvent {
                    started_at: started,
                    ended_at: Some(started),
                    kind: crate::model::TimelineKind::Observed,
                    title: "window".into(),
                    description: "imported interval".into(),
                    source: None,
                    entities: Vec::new(),
                    fields: crate::model::Fields::new(),
                },
                crate::model::Provenance::External,
                vec![ip],
            )
            .expect("event");
        let data = store.load_investigation(investigation.id).expect("load");
        assert_eq!(data.events.len(), 1);
        let directory = tempfile::tempdir().expect("tempdir");
        let export_dir = directory.path().join("archive");
        export_data(&data, directory.path(), &export_dir).expect("export");
        let text = fs::read_to_string(export_dir.join(MANIFEST_NAME)).expect("manifest");
        let manifest: ArchiveManifest = serde_json::from_str(&text).expect("parse");
        assert_eq!(manifest.format_version, FORMAT_VERSION);
        assert_eq!(manifest.data.events.len(), 1);
        assert_eq!(manifest.data.events[0].entity_ids, vec![ip]);
        let mut target = Store::open_in_memory().expect("target");
        let paths = crate::paths::AppPaths::new(
            directory.path().join("cfg"),
            directory.path().join("data"),
        );
        paths.ensure_dirs().expect("dirs");
        let imported =
            import_investigation(&mut target, &export_dir, &paths, false).expect("import");
        assert_eq!(imported.events.len(), 1);
        assert_eq!(imported.events[0].title, "window");
        assert_eq!(imported.events[0].entity_ids, vec![ip]);
        let renamed = import_investigation(&mut store, &export_dir, &paths, true).expect("rename");
        assert_ne!(renamed.investigation.id, investigation.id);
        assert_ne!(renamed.events[0].id, data.events[0].id);
        assert_ne!(renamed.events[0].entity_ids[0], ip);
        assert!(
            renamed
                .entities
                .iter()
                .any(|entity| entity.id == renamed.events[0].entity_ids[0])
        );
    }

    #[test]
    fn rename_into_same_store_remaps_all_record_uuids() {
        let mut store = Store::open_in_memory().expect("store");
        let investigation = store
            .create_investigation("Rename Me", "")
            .expect("created");
        store
            .create_page(NewPage::new(
                investigation.id,
                "",
                PageKind::Domain,
                Some("example.com"),
            ))
            .expect("page");
        let domain = store
            .add_entity(
                investigation.id,
                crate::model::EntityKind::Domain,
                "example.com",
                crate::model::Provenance::Analyst,
                None,
            )
            .expect("domain");
        let host = store
            .add_entity(
                investigation.id,
                crate::model::EntityKind::Hostname,
                "example.com",
                crate::model::Provenance::Collected,
                None,
            )
            .expect("host");
        store
            .resolve_entities(
                investigation.id,
                &[domain, host],
                domain,
                "same host",
                "tester",
            )
            .expect("merge");
        let data = store.load_investigation(investigation.id).expect("load");
        let original_resolution = data.resolutions[0].clone();
        let directory = tempfile::tempdir().expect("tempdir");
        let export_dir = directory.path().join("archive");
        export_data(&data, directory.path(), &export_dir).expect("export");
        let paths = crate::paths::AppPaths::new(
            directory.path().join("cfg"),
            directory.path().join("data"),
        );
        paths.ensure_dirs().expect("dirs");
        let imported =
            import_investigation(&mut store, &export_dir, &paths, true).expect("rename import");
        assert_ne!(imported.investigation.id, investigation.id);
        assert!(
            imported
                .entities
                .iter()
                .all(|entity| entity.id != domain && entity.id != host)
        );
        assert_eq!(imported.resolutions.len(), 1);
        let bag = &imported.resolutions[0];
        assert_ne!(bag.id, original_resolution.id);
        assert_ne!(bag.canonical_id, original_resolution.canonical_id);
        assert!(
            imported
                .entities
                .iter()
                .any(|entity| entity.id == bag.canonical_id)
        );
        for id in &bag.constituent_ids {
            assert!(
                imported.entities.iter().any(|entity| entity.id == *id),
                "constituent {id} must remap to an imported entity"
            );
        }
        assert!(
            imported.is_absorbed(
                *bag.constituent_ids
                    .iter()
                    .find(|id| **id != bag.canonical_id)
                    .expect("constituent")
            )
        );
        let original = store
            .load_investigation(investigation.id)
            .expect("original remains");
        assert_eq!(original.resolutions[0].canonical_id, domain);
        assert!(original.is_absorbed(host));
        assert!(store.foreign_key_violations().expect("fk").is_empty());
    }

    #[test]
    fn import_rejects_path_escape() {
        let directory = tempfile::tempdir().expect("tempdir");
        let mut data = InvestigationData::empty(Investigation {
            id: Uuid::new_v4(),
            name: "Bad".into(),
            description: String::new(),
            tags: Vec::new(),
            marking: crate::model::Marking::Unmarked,
            created_at: now(),
            updated_at: now(),
        });
        data.artifacts.push(crate::model::Artifact {
            id: Uuid::new_v4(),
            investigation_id: data.investigation.id,
            seq: 1,
            page_id: None,
            kind: crate::model::ArtifactKind::File,
            name: "x".into(),
            path: "../outside".into(),
            mime: None,
            size_bytes: 0,
            sha256: None,
            fields: crate::model::Fields::new(),
            source_id: None,
            created_at: now(),
        });
        let export_dir = directory.path().join("bad");
        let files_root = directory.path().join("files-src");
        fs::create_dir_all(&files_root).expect("files");
        export_data(&data, &files_root, &export_dir).expect("export");
        let mut store = Store::open_in_memory().expect("store");
        let paths =
            crate::paths::AppPaths::new(directory.path().join("c"), directory.path().join("d"));
        paths.ensure_dirs().expect("dirs");
        assert!(import_investigation(&mut store, &export_dir, &paths, false).is_err());
    }

    #[test]
    fn export_import_preserves_marking_and_audit_and_rename_remaps_them() {
        let mut store = Store::open_in_memory().expect("store");
        store.set_identity(crate::model::Identity {
            user_id: "owner".into(),
            machine_name: "lab".into(),
        });
        let investigation = store.create_investigation("Marked", "").expect("created");
        store
            .set_marking(investigation.id, crate::model::Marking::TlpAmber)
            .expect("mark");
        let domain = store
            .add_entity(
                investigation.id,
                crate::model::EntityKind::Domain,
                "example.com",
                crate::model::Provenance::Analyst,
                None,
            )
            .expect("domain");
        let host = store
            .add_entity(
                investigation.id,
                crate::model::EntityKind::Hostname,
                "example.com",
                crate::model::Provenance::Collected,
                None,
            )
            .expect("host");
        store
            .resolve_entities(
                investigation.id,
                &[domain, host],
                domain,
                "same host",
                "owner",
            )
            .expect("merge");
        let directory = tempfile::tempdir().expect("tempdir");
        let export_dir = directory.path().join("archive");
        crate::exchange::export_investigation(
            &mut store,
            investigation.id,
            directory.path(),
            &export_dir,
        )
        .expect("export");
        let text = fs::read_to_string(export_dir.join(MANIFEST_NAME)).expect("manifest");
        let manifest: ArchiveManifest = serde_json::from_str(&text).expect("parse");
        assert_eq!(manifest.format_version, 4);
        assert_eq!(
            manifest.data.investigation.marking,
            crate::model::Marking::TlpAmber
        );
        assert!(
            manifest
                .data
                .audit
                .iter()
                .any(|entry| entry.kind == crate::model::AuditKind::EntitiesMerged)
        );
        assert!(
            manifest
                .data
                .audit
                .iter()
                .any(|entry| entry.kind == crate::model::AuditKind::Export)
        );
        let mut target = Store::open_in_memory().expect("target");
        let paths = crate::paths::AppPaths::new(
            directory.path().join("cfg"),
            directory.path().join("data"),
        );
        paths.ensure_dirs().expect("dirs");
        let imported =
            import_investigation(&mut target, &export_dir, &paths, false).expect("import");
        assert_eq!(
            imported.investigation.marking,
            crate::model::Marking::TlpAmber
        );
        assert!(!imported.audit.is_empty());
        let renamed = import_investigation(&mut store, &export_dir, &paths, true).expect("rename");
        assert_ne!(renamed.investigation.id, investigation.id);
        assert_eq!(
            renamed.investigation.marking,
            crate::model::Marking::TlpAmber
        );
        assert!(
            renamed
                .audit
                .iter()
                .all(|entry| entry.investigation_id == renamed.investigation.id)
        );
    }
}
