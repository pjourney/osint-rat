//! Evidence Batch JSON: the sidecar / connector payload.
//!
//! Envelope:
//!
//! ```json
//! { "format": "os-int-evidence-batch", "format_version": 1, "batch": { ... } }
//! ```
//!
//! A raw `EvidenceBatch` object (sources / entities / …) is also accepted. The archive
//! `FORMAT_VERSION` is a different constant and is not used here.

use serde_json::Value;

use super::{IngestError, IngestReport, MAX_OBJECTS};
use crate::files::sha256_hex;
use crate::model::{
    Confidence, EntityKey, EntityKind, EvidenceBatch, Fields, NewArtifact, NewCollectedFinding,
    NewEntity, NewEvent, NewRelationship, NewSource, NewTimelineEvent, Provenance, SourceKind,
    TimelineKind, Timestamp, now, parse_timestamp_untrusted,
};
use crate::paths::is_safe_relative_path;
use crate::sanitize::sanitize_text;

pub const BATCH_FORMAT: &str = "os-int-evidence-batch";
pub const BATCH_FORMAT_VERSION: u32 = 1;

/// Parses connector / file JSON into an evidence batch.
pub fn parse_batch_json(bytes: &[u8], origin: &str) -> Result<IngestReport, IngestError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| IngestError::new(format!("evidence batch JSON is invalid: {error}")))?;
    if !value.is_object() {
        return Err(IngestError::new(
            "evidence batch JSON must be an object (envelope or raw batch)",
        ));
    }
    if let Some(format) = value.get("format").and_then(Value::as_str) {
        if format != BATCH_FORMAT && format != "os-int-investigation" {
            return Err(IngestError::new(format!(
                "unknown ingest format {format:?}; expected {BATCH_FORMAT}"
            )));
        }
        if format == "os-int-investigation" {
            return Err(IngestError::new(
                "this file is an investigation archive; use `os-int import` instead of ingest",
            ));
        }
    }
    if let Some(version) = value.get("format_version").and_then(Value::as_u64)
        && version > u64::from(BATCH_FORMAT_VERSION)
    {
        return Err(IngestError::new(format!(
            "evidence batch format_version {version} is newer than {BATCH_FORMAT_VERSION}"
        )));
    }
    let batch_value = value.get("batch").unwrap_or(&value);
    let mut report = IngestReport::new(EvidenceBatch::new());
    read_batch(batch_value, origin, &mut report)?;
    if report.batch.summary.is_empty() {
        report.batch.summary = format!(
            "ingested {} source(s), {} entit(y/ies), {} relationship(s), {} event(s) from {origin}",
            report.batch.sources.len(),
            report.batch.entities.len(),
            report.batch.relationships.len(),
            report.batch.events.len()
        );
    }
    Ok(report)
}

fn read_batch(value: &Value, origin: &str, report: &mut IngestReport) -> Result<(), IngestError> {
    let Some(object) = value.as_object() else {
        return Err(IngestError::new("batch body must be a JSON object"));
    };
    if let Some(sources) = object.get("sources").and_then(Value::as_array) {
        for (index, source) in sources.iter().take(MAX_OBJECTS).enumerate() {
            match read_source(source, origin) {
                Ok(source) => {
                    report.batch.sources.push(source);
                }
                Err(error) => {
                    report.skipped += 1;
                    report
                        .warnings
                        .push(format!("source {index}: {}", error.message));
                }
            }
        }
        if sources.len() > MAX_OBJECTS {
            report.skipped += sources.len() - MAX_OBJECTS;
            report.warnings.push(format!(
                "truncated sources at {MAX_OBJECTS} (file listed {})",
                sources.len()
            ));
        }
    }
    if let Some(entities) = object.get("entities").and_then(Value::as_array) {
        for (index, entity) in entities.iter().take(MAX_OBJECTS).enumerate() {
            match read_entity(entity) {
                Ok(entity) => report.batch.entities.push(entity),
                Err(error) => {
                    report.skipped += 1;
                    report
                        .warnings
                        .push(format!("entity {index}: {}", error.message));
                }
            }
        }
    }
    if let Some(relationships) = object.get("relationships").and_then(Value::as_array) {
        for (index, relationship) in relationships.iter().take(MAX_OBJECTS).enumerate() {
            match read_relationship(relationship) {
                Ok(relationship) => report.batch.relationships.push(relationship),
                Err(error) => {
                    report.skipped += 1;
                    report
                        .warnings
                        .push(format!("relationship {index}: {}", error.message));
                }
            }
        }
    }
    if let Some(timeline) = object.get("timeline").and_then(Value::as_array) {
        for event in timeline.iter().take(MAX_OBJECTS) {
            if let Ok(event) = read_event(event) {
                report.batch.timeline.push(event);
            } else {
                report.skipped += 1;
            }
        }
    }
    if let Some(events) = object.get("events").and_then(Value::as_array) {
        for (index, event) in events.iter().take(MAX_OBJECTS).enumerate() {
            match read_case_event(event, report, index) {
                Ok(event) => report.batch.events.push(event),
                Err(error) => {
                    report.skipped += 1;
                    report
                        .warnings
                        .push(format!("event {index}: {}", error.message));
                }
            }
        }
    }
    if let Some(artifacts) = object.get("artifacts").and_then(Value::as_array) {
        for artifact in artifacts.iter().take(MAX_OBJECTS) {
            if let Ok(artifact) = read_artifact(artifact) {
                report.batch.artifacts.push(artifact);
            } else {
                report.skipped += 1;
            }
        }
    }
    if let Some(findings) = object.get("findings").and_then(Value::as_array) {
        for finding in findings.iter().take(MAX_OBJECTS) {
            if let Ok(finding) = read_finding(finding) {
                report.batch.findings.push(finding);
            }
        }
    }
    if let Some(resolved) = object.get("resolved_findings").and_then(Value::as_array)
        && resolved
            .iter()
            .any(|title| title.as_str().is_some_and(|title| !title.trim().is_empty()))
    {
        report.warnings.push(
            "ignored resolved_findings on file/sidecar ingest (honored only from built-in collectors)"
                .into(),
        );
    }
    if let Some(summary) = object.get("summary").and_then(Value::as_str) {
        report.batch.summary = sanitize_text(summary).into_owned();
    }
    Ok(())
}

fn read_source(value: &Value, origin: &str) -> Result<NewSource, IngestError> {
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .map(SourceKind::parse)
        .unwrap_or(SourceKind::Document);
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("Ingested source from {origin}"));
    let mut source = NewSource::external(kind, "ingest:batch", title);
    source.url = string_field(value, "url");
    source.author = string_field(value, "author");
    source.collector = string_field(value, "collector").or(Some("ingest:batch".into()));
    source.content = value
        .get("content")
        .and_then(Value::as_str)
        .map(|content| sanitize_text(content).into_owned())
        .unwrap_or_default();
    source.content_hash = string_field(value, "content_hash").or_else(|| {
        if source.content.is_empty() {
            None
        } else {
            Some(sha256_hex(source.content.as_bytes()))
        }
    });
    if let Some(path) = string_field(value, "snapshot_path") {
        if !is_safe_relative_path(&path) {
            return Err(IngestError::new(format!(
                "source snapshot_path {path:?} is not a safe relative path"
            )));
        }
        source.snapshot_path = Some(path);
    }
    source.fields = read_fields(value.get("fields"));
    source.provenance = Provenance::External;
    if let Some(retrieved) = value.get("retrieved_at").and_then(parse_timestamp_value) {
        source.retrieved_at = retrieved;
    } else {
        source.retrieved_at = now();
    }
    Ok(source)
}

fn read_entity(value: &Value) -> Result<NewEntity, IngestError> {
    let key_value = value.get("key").unwrap_or(value);
    let key = read_entity_key(key_value)?;
    let sources = read_source_keys(value.get("sources"));
    Ok(NewEntity {
        key,
        label: string_field(value, "label"),
        fields: read_fields(value.get("fields")),
        sources,
        link_to_page: value
            .get("link_to_page")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

fn read_relationship(value: &Value) -> Result<NewRelationship, IngestError> {
    let from = value
        .get("from")
        .ok_or_else(|| IngestError::new("relationship is missing `from`"))?;
    let to = value
        .get("to")
        .ok_or_else(|| IngestError::new("relationship is missing `to`"))?;
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
        .ok_or_else(|| IngestError::new("relationship is missing `kind`"))?;
    Ok(NewRelationship {
        from: read_entity_key(from)?,
        kind: crate::model::normalize_relationship_kind(kind),
        to: read_entity_key(to)?,
        sources: read_source_keys(value.get("sources")),
        confidence: value
            .get("confidence")
            .and_then(Value::as_str)
            .and_then(Confidence::parse)
            .unwrap_or(Confidence::High),
    })
}

fn read_event(value: &Value) -> Result<NewTimelineEvent, IngestError> {
    let occurred_at = value
        .get("occurred_at")
        .and_then(parse_timestamp_value)
        .ok_or_else(|| IngestError::new("timeline event is missing occurred_at"))?;
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .map(TimelineKind::parse)
        .unwrap_or(TimelineKind::Observed);
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("imported event")
        .to_string();
    Ok(NewTimelineEvent {
        occurred_at,
        kind,
        title: sanitize_text(&title).into_owned(),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .map(|text| sanitize_text(text).into_owned())
            .unwrap_or_default(),
        source: value
            .get("source")
            .and_then(Value::as_u64)
            .map(|key| key as usize),
        entity: value
            .get("entity")
            .and_then(|entity| read_entity_key(entity).ok()),
    })
}

fn read_case_event(
    value: &Value,
    report: &mut IngestReport,
    index: usize,
) -> Result<NewEvent, IngestError> {
    let started_at = value
        .get("started_at")
        .or_else(|| value.get("occurred_at"))
        .and_then(parse_timestamp_value)
        .ok_or_else(|| IngestError::new("event is missing started_at"))?;
    let ended_at = crate::ingest::interval_end_or_skip(
        started_at,
        value.get("ended_at").and_then(parse_timestamp_value),
        report,
        &format!("event {index}"),
    );
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .map(TimelineKind::parse)
        .unwrap_or(TimelineKind::Observed);
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("imported event")
        .to_string();
    let mut entities = Vec::new();
    if let Some(items) = value.get("entities").and_then(Value::as_array) {
        for item in items {
            if let Ok(key) = read_entity_key(item) {
                entities.push(key);
            }
        }
    } else if let Some(entity) = value
        .get("entity")
        .and_then(|entity| read_entity_key(entity).ok())
    {
        entities.push(entity);
    }
    Ok(NewEvent {
        started_at,
        ended_at,
        kind,
        title: sanitize_text(&title).into_owned(),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .map(|text| sanitize_text(text).into_owned())
            .unwrap_or_default(),
        source: value
            .get("source")
            .and_then(Value::as_u64)
            .map(|key| key as usize),
        entities,
        fields: read_fields(value.get("fields")),
    })
}

fn parse_timestamp_value(value: &Value) -> Option<Timestamp> {
    match value {
        Value::String(text) => parse_timestamp_untrusted(text),
        Value::Number(number) => number
            .as_i64()
            .and_then(|unix| parse_timestamp_untrusted(&unix.to_string())),
        _ => None,
    }
}

fn read_artifact(value: &Value) -> Result<NewArtifact, IngestError> {
    let name = string_field(value, "name").unwrap_or_else(|| "artifact".into());
    let path = string_field(value, "path")
        .ok_or_else(|| IngestError::new("artifact is missing a relative path"))?;
    if !is_safe_relative_path(&path) {
        return Err(IngestError::new(format!(
            "artifact path {path:?} is not a safe relative path"
        )));
    }
    Ok(NewArtifact {
        kind: value
            .get("kind")
            .and_then(Value::as_str)
            .map(crate::model::ArtifactKind::parse)
            .unwrap_or(crate::model::ArtifactKind::File),
        name,
        path,
        mime: string_field(value, "mime"),
        size_bytes: value.get("size_bytes").and_then(Value::as_i64).unwrap_or(0),
        sha256: string_field(value, "sha256"),
        fields: read_fields(value.get("fields")),
        source: value
            .get("source")
            .and_then(Value::as_u64)
            .map(|key| key as usize),
    })
}

fn read_finding(value: &Value) -> Result<NewCollectedFinding, IngestError> {
    let title = string_field(value, "title")
        .ok_or_else(|| IngestError::new("finding is missing a title"))?;
    Ok(NewCollectedFinding {
        title,
        details: value
            .get("details")
            .and_then(Value::as_str)
            .map(|text| sanitize_text(text).into_owned())
            .unwrap_or_default(),
        confidence: value
            .get("confidence")
            .and_then(Value::as_str)
            .and_then(Confidence::parse)
            .unwrap_or(Confidence::Medium),
        source: value
            .get("source")
            .and_then(Value::as_u64)
            .map(|key| key as usize),
        provenance: Provenance::External,
    })
}

pub(crate) fn read_entity_key(value: &Value) -> Result<EntityKey, IngestError> {
    if let Some(text) = value.as_str() {
        return parse_kind_value("unknown", text);
    }
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let raw = value
        .get("value")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| IngestError::new("entity is missing a value"))?;
    parse_kind_value(kind, raw)
}

fn parse_kind_value(kind: &str, raw: &str) -> Result<EntityKey, IngestError> {
    if raw.len() > 2048 {
        return Err(IngestError::new(
            "entity value is longer than 2048 characters",
        ));
    }
    Ok(EntityKey::new(EntityKind::parse(kind), raw))
}

fn read_source_keys(value: Option<&Value>) -> Vec<usize> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_u64)
                .map(|key| key as usize)
                .collect()
        })
        .unwrap_or_default()
}

fn read_fields(value: Option<&Value>) -> Fields {
    let mut fields = Fields::new();
    match value {
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(pair) = item.as_array()
                    && pair.len() >= 2
                    && let (Some(key), Some(val)) = (pair[0].as_str(), pair[1].as_str())
                {
                    fields.push(key, sanitize_text(val).into_owned());
                }
            }
        }
        Some(Value::Object(map)) => {
            for (key, val) in map {
                if let Some(text) = val.as_str() {
                    fields.push(key.clone(), sanitize_text(text).into_owned());
                }
            }
        }
        _ => {}
    }
    fields
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| sanitize_text(text).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn fixture_events_batch_parses_interval() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/ingest/events-batch.json");
        let bytes = std::fs::read(path).expect("fixture");
        let actual = parse_batch_json(&bytes, "events-batch.json").expect("parse");
        assert_eq!(actual.batch.events.len(), 1);
        assert_eq!(actual.batch.events[0].title, "joint window");
        assert!(actual.batch.events[0].ended_at.is_some());
        assert_eq!(actual.batch.events[0].entities.len(), 2);
        assert_eq!(actual.batch.timeline.len(), 1);
        assert_eq!(actual.batch.entities.len(), 2);
        assert_eq!(actual.batch.entities[0].fields.get("country"), Some("US"));
    }

    #[test]
    fn envelope_and_raw_batch_parse() {
        let input = r#"{
            "format": "os-int-evidence-batch",
            "format_version": 1,
            "batch": {
                "sources": [{"kind": "document", "title": "hello", "content": "body"}],
                "entities": [{"kind": "domain", "value": "Example.COM"}],
                "relationships": [{
                    "from": {"kind": "hostname", "value": "www.example.com"},
                    "kind": "subdomain_of",
                    "to": {"kind": "domain", "value": "example.com"}
                }]
            }
        }"#;
        let actual = parse_batch_json(input.as_bytes(), "hello.json").expect("parse");
        assert_eq!(actual.batch.entities.len(), 1);
        assert_eq!(actual.batch.entities[0].key.value, "example.com");
        assert_eq!(actual.batch.relationships[0].kind, "subdomain_of");
        assert_eq!(actual.batch.sources[0].provenance, Provenance::External);
        let raw = r#"{"sources":[{"title":"s"}],"entities":[{"kind":"ip","value":"192.0.2.9"}]}"#;
        let actual = parse_batch_json(raw.as_bytes(), "raw.json").expect("raw");
        assert_eq!(actual.batch.entities[0].key.value, "192.0.2.9");
    }

    #[test]
    fn newer_format_version_is_rejected() {
        let input = r#"{"format":"os-int-evidence-batch","format_version":99,"batch":{}}"#;
        let actual = parse_batch_json(input.as_bytes(), "x.json");
        assert!(actual.expect_err("future").message.contains("newer"));
    }

    #[test]
    fn investigation_archive_is_refused() {
        let input = r#"{"format":"os-int-investigation","format_version":2,"data":{}}"#;
        let actual = parse_batch_json(input.as_bytes(), "archive.json");
        assert!(actual.expect_err("archive").message.contains("import"));
    }

    #[test]
    fn malformed_json_does_not_panic() {
        assert!(parse_batch_json(b"{not json", "x").is_err());
        assert!(parse_batch_json(b"[]", "x").is_err());
        let huge_rel = r#"{"relationships":[{"from":{"kind":"domain"}}]}"#;
        let actual = parse_batch_json(huge_rel.as_bytes(), "x").expect("partial");
        assert_eq!(actual.skipped, 1);
    }

    #[test]
    fn untrusted_batch_cannot_set_agent_provenance() {
        let input = r#"{
            "sources": [{"kind": "document", "title": "claimed", "provenance": "agent", "content": "x"}],
            "findings": [{"title": "AI finding", "provenance": "agent"}]
        }"#;
        let actual = parse_batch_json(input.as_bytes(), "evil.json").expect("parse");
        assert_eq!(actual.batch.sources[0].provenance, Provenance::External);
        assert_eq!(actual.batch.findings[0].provenance, Provenance::External);
    }

    #[test]
    fn untrusted_resolved_findings_are_ignored() {
        let input = r#"{
            "sources": [{"title": "s", "content": "body"}],
            "resolved_findings": ["example.com spf"]
        }"#;
        let actual = parse_batch_json(input.as_bytes(), "evil.json").expect("parse");
        assert!(actual.batch.resolved_findings.is_empty());
        assert!(
            actual
                .warnings
                .iter()
                .any(|warning| warning.contains("resolved_findings"))
        );
    }

    #[test]
    fn malformed_timestamps_are_skipped_without_panic() {
        let input = r#"{
            "timeline": [
                {"occurred_at": "NaN", "title": "bad"},
                {"occurred_at": "2024-13-40", "title": "oor"},
                {"occurred_at": "2024-06-01T12:00:00-04:00", "title": "ok"}
            ],
            "events": [
                {"started_at": "null", "title": "null"},
                {
                    "started_at": "2024-05-01T12:00:00Z",
                    "ended_at": "2024-05-01T18:00:00Z",
                    "title": "window",
                    "entities": [{"kind": "ip", "value": "192.0.2.1"}]
                },
                {
                    "started_at": "2024-05-02T12:00:00Z",
                    "ended_at": "2024-05-01T12:00:00Z",
                    "title": "backwards"
                }
            ]
        }"#;
        let actual = parse_batch_json(input.as_bytes(), "times.json").expect("parse");
        assert_eq!(actual.batch.timeline.len(), 1);
        assert_eq!(actual.batch.timeline[0].title, "ok");
        assert_eq!(
            actual.batch.timeline[0].occurred_at,
            chrono::Utc.with_ymd_and_hms(2024, 6, 1, 16, 0, 0).unwrap()
        );
        assert_eq!(actual.batch.events.len(), 2);
        assert_eq!(actual.batch.events[0].title, "window");
        assert!(actual.batch.events[0].ended_at.is_some());
        assert_eq!(actual.batch.events[1].title, "backwards");
        assert!(actual.batch.events[1].ended_at.is_none());
        assert!(
            actual
                .warnings
                .iter()
                .any(|warning| warning.contains("ended_at precedes started_at"))
        );
        assert_eq!(actual.skipped, 4);
    }

    #[test]
    fn unsafe_snapshot_and_artifact_paths_are_rejected() {
        let snapshot = r#"{"sources":[{"title":"s","snapshot_path":"../../etc/passwd"}]}"#;
        let actual = parse_batch_json(snapshot.as_bytes(), "x.json").expect("source skipped");
        assert_eq!(actual.skipped, 1);
        assert!(actual.batch.sources.is_empty());
        let artifact = r#"{"artifacts":[{"name":"passwd","path":"../../etc/passwd"}]}"#;
        let actual = parse_batch_json(artifact.as_bytes(), "x.json").expect("artifact skipped");
        assert_eq!(actual.skipped, 1);
        assert!(actual.batch.artifacts.is_empty());
        let safe = r#"{"sources":[{"title":"s","snapshot_path":"snapshots/ab/abcdef.bin"}]}"#;
        let actual = parse_batch_json(safe.as_bytes(), "x.json").expect("safe");
        assert_eq!(
            actual.batch.sources[0].snapshot_path.as_deref(),
            Some("snapshots/ab/abcdef.bin")
        );
    }
}
