//! Playbooks: parse YAML, plan Actions, execute collect/Search Around/tools, watch sets.
//!
//! Collectors still return batches. Playbooks never execute tools marked `active`, never
//! auto-merge, and never write finding templates until the analyst accepts a proposal.
//! Untrusted files are size-capped and parsed without panicking.

mod bundle;
mod execute;
mod parse;
mod plan;
mod watch;

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use crate::model::{MAX_PLAYBOOK_BYTES, Playbook};
use crate::paths::is_safe_relative_path;

pub use bundle::{BUNDLED, bundled, bundled_ids, bundled_prompt};
pub use execute::{ExecuteOptions, commit, execute, execute_prepared, gather};
pub use parse::parse_playbook_yaml;
pub use plan::{bind_resolve_plan, plan_playbook};
pub use watch::{automation_plan, change_proposals};

/// A parse or execute failure that should be shown to the analyst. Never a panic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaybookError {
    pub message: String,
}

impl PlaybookError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for PlaybookError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PlaybookError {}

impl From<io::Error> for PlaybookError {
    fn from(error: io::Error) -> Self {
        Self::new(format!("could not read playbook: {error}"))
    }
}

/// Load a bundled id (`domain-infra`) or a filesystem path.
pub fn load_playbook(spec: &str) -> Result<Playbook, PlaybookError> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err(PlaybookError::new(
            "playbook is empty; pass a bundled id (domain-infra, mail-policy, cert-estate, repo-surface) or a YAML file",
        ));
    }
    if let Some(playbook) = bundled(trimmed) {
        return Ok(playbook);
    }
    if trimmed.contains("..") || trimmed.contains('\0') {
        return Err(PlaybookError::new(
            "playbook path must not contain '..' or NUL (path traversal is refused)",
        ));
    }
    let path = Path::new(trimmed);
    read_playbook_file(path)
}

/// Read an untrusted playbook file: size-capped, files only, no panic.
pub fn read_playbook_file(path: &Path) -> Result<Playbook, PlaybookError> {
    let bytes = read_playbook_bytes(path)?;
    let mut playbook = parse_playbook_yaml(&bytes)?;
    playbook.origin = crate::model::PlaybookOrigin::File(display_path(path));
    Ok(playbook)
}

fn read_playbook_bytes(path: &Path) -> Result<Vec<u8>, PlaybookError> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            PlaybookError::new(format!(
                "playbook file not found: {}; bundled ids are {}",
                path.display(),
                bundled_ids().join(", ")
            ))
        } else {
            PlaybookError::new(format!(
                "could not read playbook {}: {error}",
                path.display()
            ))
        }
    })?;
    if metadata.is_dir() {
        return Err(PlaybookError::new(format!(
            "{} is a directory; pass a YAML file",
            path.display()
        )));
    }
    if metadata.len() > MAX_PLAYBOOK_BYTES {
        return Err(PlaybookError::new(format!(
            "playbook {} is {} bytes; maximum is {MAX_PLAYBOOK_BYTES}",
            path.display(),
            metadata.len()
        )));
    }
    let file = File::open(path).map_err(|error| {
        PlaybookError::new(format!(
            "could not open playbook {}: {error}",
            path.display()
        ))
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_PLAYBOOK_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            PlaybookError::new(format!(
                "could not read playbook {}: {error}",
                path.display()
            ))
        })?;
    if bytes.len() as u64 > MAX_PLAYBOOK_BYTES {
        return Err(PlaybookError::new(format!(
            "playbook {} exceeds {MAX_PLAYBOOK_BYTES} bytes",
            path.display()
        )));
    }
    Ok(bytes)
}

fn display_path(path: &Path) -> String {
    let rendered = path.display().to_string();
    if is_safe_relative_path(&rendered) {
        rendered
    } else {
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("playbook.yaml")
            .to_string()
    }
}

/// True when `spec` is a bundled playbook id (agent `RUN_PLAYBOOK` may only use these).
pub fn is_bundled_id(spec: &str) -> bool {
    bundled(spec.trim()).is_some()
}
