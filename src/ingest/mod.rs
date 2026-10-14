//! Ingest plane: turn untrusted files and connector JSON into an [`EvidenceBatch`].
//!
//! Collectors still return batches. This module is the file/sidecar path: STIX 2.1, MISP
//! events, bulk CSV/JSON observables, and the Evidence Batch envelope. Parsing never panics
//! on malformed or huge input; it returns a structured error. Apply stays in `storage::apply`.

mod batch;
mod misp;
mod observables;
mod stix;

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use crate::model::EvidenceBatch;

pub use batch::{BATCH_FORMAT, BATCH_FORMAT_VERSION, parse_batch_json};
pub use misp::parse_misp;
pub use observables::{ObservableMapping, parse_csv_observables, parse_json_observables};
pub use stix::parse_stix;

/// Default cap on an ingest file. Larger files are refused before they are fully decoded.
pub const DEFAULT_MAX_BYTES: u64 = 20 * 1024 * 1024;
/// Maximum objects/rows turned into entities from one file.
pub const MAX_OBJECTS: usize = 10_000;
/// UTF-8 BOM.
const UTF8_BOM: &[u8] = &[0xef, 0xbb, 0xbf];
const UTF16_LE_BOM: &[u8] = &[0xff, 0xfe];
const UTF16_BE_BOM: &[u8] = &[0xfe, 0xff];

/// How a file (or sidecar stdout) should be interpreted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IngestFormat {
    Batch,
    Stix,
    Misp,
    Csv,
    Json,
}

impl IngestFormat {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "batch" | "evidence" | "evidence-batch" | "os-int-evidence-batch" => Some(Self::Batch),
            "stix" | "stix2" | "stix2.1" => Some(Self::Stix),
            "misp" => Some(Self::Misp),
            "csv" => Some(Self::Csv),
            "json" | "observables" => Some(Self::Json),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Batch => "batch",
            Self::Stix => "stix",
            Self::Misp => "misp",
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }

    pub const ALL: [IngestFormat; 5] = [
        IngestFormat::Stix,
        IngestFormat::Misp,
        IngestFormat::Csv,
        IngestFormat::Json,
        IngestFormat::Batch,
    ];
}

/// A parse failure that should be shown to the analyst. Never a panic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IngestError {
    pub message: String,
}

impl IngestError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for IngestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for IngestError {}

impl From<io::Error> for IngestError {
    fn from(error: io::Error) -> Self {
        Self::new(format!("could not read ingest file: {error}"))
    }
}

/// What an ingest parse produced, including non-fatal skips.
#[derive(Clone, Debug, PartialEq)]
pub struct IngestReport {
    pub batch: EvidenceBatch,
    pub warnings: Vec<String>,
    pub skipped: usize,
}

impl IngestReport {
    pub fn new(batch: EvidenceBatch) -> Self {
        Self {
            batch,
            warnings: Vec::new(),
            skipped: 0,
        }
    }
}

/// Column mapping for CSV / JSON observables.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IngestOptions {
    pub mapping: ObservableMapping,
    pub origin: String,
}

/// Reads a file with a hard size cap. Directories, oversized files, and I/O errors fail
/// without loading the rest of the file into memory.
pub fn read_ingest_file(path: &Path, max_bytes: u64) -> Result<Vec<u8>, IngestError> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| IngestError::new(format!("could not read {}: {error}", path.display())))?;
    if metadata.is_dir() {
        return Err(IngestError::new(format!(
            "{} is a directory; pass a file",
            path.display()
        )));
    }
    if metadata.len() > max_bytes {
        return Err(IngestError::new(format!(
            "{} is {} bytes; ingest refuses files larger than {} bytes",
            path.display(),
            metadata.len(),
            max_bytes
        )));
    }
    let file = File::open(path)
        .map_err(|error| IngestError::new(format!("could not open {}: {error}", path.display())))?;
    let mut limited = file.take(max_bytes.saturating_add(1));
    let mut bytes = Vec::new();
    limited.read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(IngestError::new(format!(
            "{} exceeded the ingest size cap of {max_bytes} bytes",
            path.display()
        )));
    }
    Ok(bytes)
}

/// Decodes ingest bytes as UTF-8 (BOM stripped) or UTF-16 when a BOM is present.
pub fn decode_text(bytes: &[u8]) -> Result<String, IngestError> {
    if bytes.starts_with(UTF16_LE_BOM) {
        return decode_utf16(&bytes[2..], false);
    }
    if bytes.starts_with(UTF16_BE_BOM) {
        return decode_utf16(&bytes[2..], true);
    }
    let rest = if bytes.starts_with(UTF8_BOM) {
        &bytes[3..]
    } else {
        bytes
    };
    std::str::from_utf8(rest)
        .map(|text| text.to_string())
        .map_err(|_| {
            IngestError::new(
                "ingest file is not valid UTF-8 (decode as UTF-8, or include a UTF-16 BOM)",
            )
        })
}

fn decode_utf16(bytes: &[u8], big_endian: bool) -> Result<String, IngestError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(IngestError::new("truncated UTF-16 ingest file"));
    }
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|chunk| {
            if big_endian {
                u16::from_be_bytes(*chunk)
            } else {
                u16::from_le_bytes(*chunk)
            }
        })
        .collect();
    String::from_utf16(&units).map_err(|_| IngestError::new("invalid UTF-16 ingest file"))
}

/// Parses bytes according to `format`.
pub fn ingest(
    format: IngestFormat,
    bytes: &[u8],
    options: &IngestOptions,
) -> Result<IngestReport, IngestError> {
    let text = decode_text(bytes)?;
    if text.trim().is_empty() {
        return Err(IngestError::new("ingest file is empty"));
    }
    match format {
        IngestFormat::Batch => parse_batch_json(text.as_bytes(), &options.origin),
        IngestFormat::Stix => parse_stix(&text, &options.origin),
        IngestFormat::Misp => parse_misp(&text, &options.origin),
        IngestFormat::Csv => parse_csv_observables(&text, &options.mapping, &options.origin),
        IngestFormat::Json => parse_json_observables(&text, &options.mapping, &options.origin),
    }
}

/// Guesses a format from a file name when the analyst did not pass `--format`.
pub fn guess_format(path: &Path) -> Option<IngestFormat> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name.contains("stix") {
        return Some(IngestFormat::Stix);
    }
    if name.contains("misp") {
        return Some(IngestFormat::Misp);
    }
    if name.ends_with(".csv") {
        return Some(IngestFormat::Csv);
    }
    if name.contains("batch") || name.ends_with(".osint.json") {
        return Some(IngestFormat::Batch);
    }
    if name.ends_with(".json") {
        return Some(IngestFormat::Json);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_parse_accepts_aliases() {
        assert_eq!(IngestFormat::parse("STIX 2.1"), None);
        assert_eq!(IngestFormat::parse("stix2.1"), Some(IngestFormat::Stix));
        assert_eq!(
            IngestFormat::parse("evidence-batch"),
            Some(IngestFormat::Batch)
        );
    }

    #[test]
    fn decode_text_strips_utf8_bom_and_reads_utf16() {
        let mut bom = UTF8_BOM.to_vec();
        bom.extend_from_slice(b"hello");
        assert_eq!(decode_text(&bom).expect("utf8"), "hello");
        let mut utf16 = UTF16_LE_BOM.to_vec();
        for unit in "ok".encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode_text(&utf16).expect("utf16"), "ok");
        assert!(decode_text(&[0xff, 0xfe, 0x00]).is_err());
        assert!(decode_text(&[0x80, 0x81]).is_err());
    }

    #[test]
    fn read_ingest_file_refuses_oversized_and_missing() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("tiny.txt");
        std::fs::write(&path, b"abc").expect("write");
        let actual = read_ingest_file(&path, 2);
        assert!(actual.expect_err("cap").message.contains("bytes"));
        let missing = read_ingest_file(&directory.path().join("nope"), 100);
        assert!(missing.is_err());
        let dir_err = read_ingest_file(directory.path(), 100);
        assert!(dir_err.expect_err("dir").message.contains("directory"));
    }

    #[test]
    fn fixtures_import_with_cited_sources() {
        let stix = include_str!("../../tests/fixtures/ingest/stix-bundle.json");
        let misp = include_str!("../../tests/fixtures/ingest/misp-event.json");
        let csv = include_str!("../../tests/fixtures/ingest/observables.csv");
        let stix_report = parse_stix(stix, "stix-bundle.json").expect("stix");
        let misp_report = parse_misp(misp, "misp-event.json").expect("misp");
        let csv_report =
            parse_csv_observables(csv, &ObservableMapping::default(), "observables.csv")
                .expect("csv");
        assert_eq!(
            stix_report.batch.sources[0].provenance,
            crate::model::Provenance::External
        );
        assert!(!stix_report.batch.entities.is_empty());
        assert!(
            !stix_report.batch.sources[0]
                .content_hash
                .as_deref()
                .unwrap_or("")
                .is_empty()
        );
        assert_eq!(
            misp_report.batch.sources[0].kind,
            crate::model::SourceKind::Misp
        );
        assert!(
            csv_report
                .batch
                .entities
                .iter()
                .any(|entity| entity.key.value == "example.com")
        );
        let mut store = crate::storage::Store::open_in_memory().expect("store");
        let investigation = store
            .create_investigation("ingest-fixtures", "")
            .expect("case");
        let context = crate::storage::ApplyContext {
            investigation_id: investigation.id,
            page_id: None,
            job_id: None,
        };
        store
            .apply_batch(context, &stix_report.batch)
            .expect("stix apply");
        store
            .apply_batch(context, &misp_report.batch)
            .expect("misp apply");
        store
            .apply_batch(context, &csv_report.batch)
            .expect("csv apply");
        let data = store.load_investigation(investigation.id).expect("load");
        assert!(
            data.sources
                .iter()
                .any(|source| source.kind == crate::model::SourceKind::Stix)
        );
        assert!(
            data.sources
                .iter()
                .any(|source| source.kind == crate::model::SourceKind::Misp)
        );
        assert!(
            data.entities
                .iter()
                .any(|entity| entity.value == "evil.example")
        );
        assert!(
            data.relationships
                .iter()
                .any(|rel| !rel.source_ids.is_empty())
        );
        store
            .apply_batch(context, &stix_report.batch)
            .expect("stix reingest");
        let again = store.load_investigation(investigation.id).expect("again");
        let stix_sources = again
            .sources
            .iter()
            .filter(|source| source.collector.as_deref() == Some("ingest:stix"))
            .count();
        assert_eq!(stix_sources, 1);
    }
}
