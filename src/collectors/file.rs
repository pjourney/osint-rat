//! Local evidence file inspection: type, hash, timestamps, and ExifTool metadata when available.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;

use super::{CollectResult, CollectionOutput, Collector, CollectorContext};
use crate::exec::{CommandSpec, ExecError, run};
use crate::files::{sha256_file, sha256_hex};
use crate::model::{
    ArtifactKind, EvidenceBatch, Fields, NewArtifact, NewSource, SourceKind, Target, TargetKind,
    TimelineKind,
};
use crate::sanitize::{sanitize_line, sanitize_text};

const EXIFTOOL_TIMEOUT: Duration = Duration::from_secs(30);
const EXIFTOOL_MAX_OUTPUT: usize = 256 * 1024;

pub struct FileCollector;

impl Collector for FileCollector {
    fn id(&self) -> &'static str {
        "file"
    }

    fn label(&self) -> &'static str {
        "File"
    }

    fn description(&self) -> &'static str {
        "Hash, MIME type, timestamps, and ExifTool metadata for a local evidence file"
    }

    fn accepts(&self, kind: TargetKind) -> bool {
        matches!(kind, TargetKind::File)
    }

    fn collect<'a>(
        &'a self,
        target: &'a Target,
        context: &'a CollectorContext,
    ) -> BoxFuture<'a, CollectResult> {
        Box::pin(collect(target, context))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileObservation {
    pub original_path: String,
    pub file_name: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub mime: Option<String>,
    pub created: Option<DateTime<Utc>>,
    pub modified: Option<DateTime<Utc>>,
    pub accessed: Option<DateTime<Utc>>,
    pub stored: Option<crate::files::StoredBlob>,
    pub exiftool: Option<String>,
}

async fn collect(target: &Target, context: &CollectorContext) -> CollectResult {
    let Target::File(path) = target else {
        return Err(format!(
            "the file collector does not accept {}",
            target.kind()
        ));
    };
    context
        .job
        .progress(format!("Inspecting {}", path.display()));
    let mut observation = inspect(path, context.settings.max_file_import_bytes)?;
    context
        .files
        .ensure()
        .map_err(|error| format!("could not create investigation directory: {error}"))?;
    observation.stored = Some(
        context
            .files
            .import_file(path, context.settings.max_file_import_bytes)
            .map_err(|error| format!("could not copy the evidence file: {error}"))?,
    );
    if context.tools.is_installed("exiftool") {
        context.job.progress("Running ExifTool");
        match run_exiftool(path, context).await {
            Ok(text) => observation.exiftool = Some(text),
            Err(message) => context.job.log(format!("warning: {message}")),
        }
    }
    Ok(CollectionOutput::new(build_batch(&observation)))
}

fn inspect(path: &Path, max_bytes: u64) -> Result<FileObservation, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if metadata.len() > max_bytes {
        return Err(format!(
            "{} is {} bytes, above the import limit of {max_bytes} bytes",
            path.display(),
            metadata.len()
        ));
    }
    let (sha256, size_bytes) =
        sha256_file(path).map_err(|error| format!("could not hash {}: {error}", path.display()))?;
    let mime = infer::get_from_path(path)
        .ok()
        .flatten()
        .map(|kind| kind.mime_type().to_string())
        .or_else(|| mime_from_extension(path));
    Ok(FileObservation {
        original_path: path.display().to_string(),
        file_name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unnamed".into()),
        size_bytes,
        sha256,
        mime,
        created: timestamp(created_time(&metadata)),
        modified: timestamp(metadata.modified().ok()),
        accessed: timestamp(metadata.accessed().ok()),
        stored: None,
        exiftool: None,
    })
}

#[cfg(unix)]
fn created_time(metadata: &fs::Metadata) -> Option<SystemTime> {
    metadata.created().ok().or_else(|| {
        use std::os::unix::fs::MetadataExt;
        let seconds = metadata.ctime();
        if seconds <= 0 {
            return None;
        }
        SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(seconds as u64))
    })
}

#[cfg(not(unix))]
fn created_time(metadata: &fs::Metadata) -> Option<SystemTime> {
    metadata.created().ok()
}

fn timestamp(value: Option<SystemTime>) -> Option<DateTime<Utc>> {
    let time = value?;
    let duration = time.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    DateTime::from_timestamp(duration.as_secs() as i64, duration.subsec_nanos())
}

fn mime_from_extension(path: &Path) -> Option<String> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "txt" | "log" | "md" => Some("text/plain".into()),
        "html" | "htm" => Some("text/html".into()),
        "json" => Some("application/json".into()),
        "xml" => Some("application/xml".into()),
        "csv" => Some("text/csv".into()),
        "pdf" => Some("application/pdf".into()),
        "eml" => Some("message/rfc822".into()),
        _ => None,
    }
}

async fn run_exiftool(path: &Path, context: &CollectorContext) -> Result<String, String> {
    let program = context
        .tools
        .path_of("exiftool")
        .unwrap_or_else(|| PathBuf::from("exiftool"));
    let mut spec = CommandSpec::new(
        program.display().to_string(),
        vec![
            "-G".into(),
            "-s".into(),
            "-a".into(),
            "-n".into(),
            path.display().to_string(),
        ],
        EXIFTOOL_TIMEOUT,
    );
    spec.max_output_bytes = EXIFTOOL_MAX_OUTPUT;
    context.job.command(&spec.program, &spec.args);
    match run(&spec, &context.job.cancel, |line| {
        context.job.log(sanitize_line(&line.text).into_owned());
    })
    .await
    {
        Ok(outcome) if outcome.success => Ok(sanitize_text(&outcome.stdout).into_owned()),
        Ok(outcome) => Err(format!(
            "exiftool exited {}: {}",
            outcome.exit_code.unwrap_or(-1),
            sanitize_line(&outcome.stderr)
        )),
        Err(ExecError::Cancelled) => Err("cancelled".into()),
        Err(error) => Err(error.to_string()),
    }
}

/// Builds the evidence batch. Pure and unit-tested.
pub fn build_batch(observation: &FileObservation) -> EvidenceBatch {
    let mut batch = EvidenceBatch::new();
    let (content, fields) = describe(observation);
    let mut source = NewSource::collected(
        SourceKind::File,
        "file",
        format!("File {}", observation.file_name),
    );
    source.content_hash = Some(observation.sha256.clone());
    source.snapshot_path = observation
        .stored
        .as_ref()
        .map(|blob| blob.relative_path.clone());
    source.content = content;
    source.fields = fields;
    let key = batch.add_source(source);
    let file = batch.entity(
        crate::model::EntityKind::File,
        &observation.file_name,
        Some(key),
    );
    batch.annotate(&file, "sha256", observation.sha256.clone());
    if let Some(mime) = &observation.mime {
        batch.annotate(&file, "mime", mime.clone());
    }
    if let Some(modified) = observation.modified {
        batch.event(
            modified,
            TimelineKind::Updated,
            format!("{} last modified", observation.file_name),
            Some(key),
            Some(file.clone()),
        );
    }
    if let Some(created) = observation.created {
        batch.event(
            created,
            TimelineKind::Observed,
            format!("{} created", observation.file_name),
            Some(key),
            Some(file),
        );
    }
    if let Some(stored) = &observation.stored {
        batch.artifacts.push(NewArtifact {
            kind: ArtifactKind::File,
            name: observation.file_name.clone(),
            path: stored.relative_path.clone(),
            mime: observation.mime.clone(),
            size_bytes: stored.size_bytes as i64,
            sha256: Some(stored.sha256.clone()),
            fields: Fields::new(),
            source: Some(key),
        });
    }
    batch.summary = format!(
        "{} ({}{})",
        observation.file_name,
        crate::sanitize::format_bytes(observation.size_bytes as usize),
        observation
            .mime
            .as_deref()
            .map(|mime| format!(", {mime}"))
            .unwrap_or_default()
    );
    batch
}

fn describe(observation: &FileObservation) -> (String, Fields) {
    let mut lines = vec![
        format!("File name: {}", observation.file_name),
        format!("Original path: {}", observation.original_path),
        format!(
            "Size: {} ({} bytes)",
            crate::sanitize::format_bytes(observation.size_bytes as usize),
            observation.size_bytes
        ),
        format!("SHA-256: {}", observation.sha256),
        format!(
            "MIME type: {}",
            observation.mime.as_deref().unwrap_or("unknown")
        ),
    ];
    let mut fields = Fields::new();
    fields.push("file_name", observation.file_name.clone());
    fields.push("size_bytes", observation.size_bytes.to_string());
    fields.push("sha256", observation.sha256.clone());
    fields.push_opt("mime", observation.mime.clone());
    if let Some(created) = observation.created {
        lines.push(format!("Created: {}", created.to_rfc3339()));
        fields.push("created", created.to_rfc3339());
    }
    if let Some(modified) = observation.modified {
        lines.push(format!("Modified: {}", modified.to_rfc3339()));
        fields.push("modified", modified.to_rfc3339());
    }
    if let Some(accessed) = observation.accessed {
        lines.push(format!("Accessed: {}", accessed.to_rfc3339()));
    }
    if let Some(exiftool) = &observation.exiftool {
        lines.push(String::new());
        lines.push("ExifTool:".into());
        lines.push(exiftool.clone());
        fields.push("exiftool_hash", sha256_hex(exiftool.as_bytes()));
    }
    (lines.join("\n"), fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::StoredBlob;

    #[test]
    fn batch_records_artifact_and_hash() {
        let input = FileObservation {
            original_path: "/tmp/photo.jpg".into(),
            file_name: "photo.jpg".into(),
            size_bytes: 2048,
            sha256: "aa".repeat(32),
            mime: Some("image/jpeg".into()),
            created: DateTime::from_timestamp(1_700_000_000, 0),
            modified: DateTime::from_timestamp(1_710_000_000, 0),
            accessed: None,
            stored: Some(StoredBlob {
                relative_path: "files/aa/hash-photo.jpg".into(),
                sha256: "aa".repeat(32),
                size_bytes: 2048,
            }),
            exiftool: Some("EXIF:Make Canon".into()),
        };
        let actual = build_batch(&input);
        assert_eq!(actual.artifacts.len(), 1);
        assert_eq!(actual.artifacts[0].kind, ArtifactKind::File);
        assert_eq!(actual.timeline.len(), 2);
        assert!(actual.sources[0].content.contains("image/jpeg"));
        assert!(actual.summary.contains("2.0 KB"));
    }
}
