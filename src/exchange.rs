//! Versioned investigation import/export. Archives are directories (or a `.osint.json` plus a
//! `files/` tree) whose paths are relative, so they can be shared without the originating
//! computer's absolute paths.

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
/// Bump rules and reserved v2 keys: `docs/ontology-exchange.md`.
pub const FORMAT_VERSION: u32 = 1;
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
    store: &Store,
    investigation_id: Uuid,
    files_root: &Path,
    destination: &Path,
) -> Result<PathBuf> {
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
/// `rename` is set, in which case a new ID is assigned and files are copied under it.
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
    let new_id = Uuid::new_v4();
    data.investigation.id = new_id;
    data.investigation.name = format!("{} (imported)", data.investigation.name);
    for page in &mut data.pages {
        page.investigation_id = new_id;
    }
    for source in &mut data.sources {
        source.investigation_id = new_id;
    }
    for finding in &mut data.findings {
        finding.investigation_id = new_id;
    }
    for entity in &mut data.entities {
        entity.investigation_id = new_id;
    }
    for relationship in &mut data.relationships {
        relationship.investigation_id = new_id;
    }
    for note in &mut data.notes {
        note.investigation_id = new_id;
    }
    for artifact in &mut data.artifacts {
        artifact.investigation_id = new_id;
    }
    for event in &mut data.timeline {
        event.investigation_id = new_id;
    }
    for job in &mut data.jobs {
        job.investigation_id = Some(new_id);
    }
    for run in &mut data.agent_runs {
        run.investigation_id = new_id;
    }
    for proposal in &mut data.proposals {
        proposal.investigation_id = new_id;
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
        let export_dir = directory.path().join("re-export");
        export_data(&imported, directory.path(), &export_dir).expect("re-export");
        let text = fs::read_to_string(export_dir.join(MANIFEST_NAME)).expect("manifest");
        let manifest: ArchiveManifest = serde_json::from_str(&text).expect("parse");
        assert_eq!(manifest.format_version, FORMAT_VERSION);
        assert_eq!(FORMAT_VERSION, 1);
    }

    #[test]
    fn import_rejects_path_escape() {
        let directory = tempfile::tempdir().expect("tempdir");
        let mut data = InvestigationData::empty(Investigation {
            id: Uuid::new_v4(),
            name: "Bad".into(),
            description: String::new(),
            tags: Vec::new(),
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
}
