//! STIX 2.1 bundle import (subset: indicator, observed-data, malware, identity,
//! cyber observables, relationship).

use std::collections::HashMap;

use serde_json::Value;

use super::stix_export::marking_from_stix_id;
use super::{IngestError, IngestReport, MAX_OBJECTS};
use crate::files::sha256_hex;
use crate::model::{
    Confidence, EntityKey, EntityKind, EvidenceBatch, Fields, Marking, NewEvent, NewSource,
    OBJECT_MARKING_FIELD, SourceKind, TimelineKind, parse_timestamp_untrusted, relation,
};
use crate::sanitize::{sanitize_line, sanitize_text};

const PATTERN: &str = r#"(?i)\[(?:[a-z0-9-]+):(?:value|name|number)\s*=\s*['"]([^'"]+)['"]\]"#;

/// Parses a STIX 2.1 bundle (or a single SDOs object) into an evidence batch.
pub fn parse_stix(text: &str, origin: &str) -> Result<IngestReport, IngestError> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| IngestError::new(format!("STIX JSON is invalid: {error}")))?;
    let objects = stix_objects(&value)?;
    let mut report = IngestReport::new(EvidenceBatch::new());
    let mut source = NewSource::external(
        SourceKind::Stix,
        "ingest:stix",
        format!("STIX bundle from {origin}"),
    );
    source.content = sanitize_text(&truncate(text, 32 * 1024)).into_owned();
    source.content_hash = Some(sha256_hex(text.as_bytes()));
    source.fields.push("origin", origin);
    source
        .fields
        .push("spec_version", spec_version(&value).unwrap_or("2.1"));
    source
        .fields
        .push("object_count", objects.len().to_string());
    let key = report.batch.add_source(source);
    let mut id_to_key: Vec<(String, EntityKey)> = Vec::new();
    let marking_defs = collect_marking_definitions(&objects);
    if objects.len() > MAX_OBJECTS {
        report.skipped += objects.len() - MAX_OBJECTS;
        report
            .warnings
            .push(format!("truncated STIX objects at {MAX_OBJECTS}"));
    }
    for object in objects.iter().take(MAX_OBJECTS) {
        let type_name = object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match type_name {
            "indicator" => ingest_indicator(object, key, &mut report, &mut id_to_key),
            "observed-data" => ingest_observed_data(object, key, &mut report, &mut id_to_key),
            "malware" => ingest_malware(object, key, &mut report, &mut id_to_key),
            "identity" => ingest_identity(object, key, &mut report, &mut id_to_key),
            "domain-name" | "hostname" | "ipv4-addr" | "ipv6-addr" | "email-addr" | "url"
            | "user-account" | "autonomous-system" | "file" | "x509-certificate" => {
                ingest_top_level_sco(object, key, &mut report, &mut id_to_key);
            }
            "relationship" | "bundle" | "marking-definition" | "" => {}
            other => {
                report.skipped += 1;
                report
                    .warnings
                    .push(format!("skipped STIX object type {other}"));
            }
        }
    }
    for object in objects.iter().take(MAX_OBJECTS) {
        apply_imported_marking(object, &marking_defs, &id_to_key, &mut report);
    }
    for object in objects.iter().take(MAX_OBJECTS) {
        if object.get("type").and_then(Value::as_str) == Some("relationship") {
            ingest_relationship(object, key, &mut report, &id_to_key);
        }
    }
    report.batch.summary = format!(
        "STIX import: {} entit(y/ies), {} relationship(s) from {origin}",
        report.batch.entities.len(),
        report.batch.relationships.len()
    );
    if report.batch.entities.is_empty() && report.batch.findings.is_empty() {
        return Err(IngestError::new(
            "STIX bundle contained no supported objects (indicator, observed-data, malware, identity, cyber observable, relationship)",
        ));
    }
    Ok(report)
}

fn stix_objects(value: &Value) -> Result<Vec<&Value>, IngestError> {
    if let Some(objects) = value.get("objects").and_then(Value::as_array) {
        return Ok(objects.iter().collect());
    }
    if value.get("type").and_then(Value::as_str) == Some("bundle") {
        return Ok(Vec::new());
    }
    if value.get("type").is_some() {
        return Ok(vec![value]);
    }
    Err(IngestError::new(
        "STIX document is missing `objects` (expected a bundle)",
    ))
}

fn spec_version(value: &Value) -> Option<&str> {
    value
        .get("spec_version")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("objects")
                .and_then(Value::as_array)
                .and_then(|objects| objects.first())
                .and_then(|object| object.get("spec_version"))
                .and_then(Value::as_str)
        })
}

fn ingest_indicator(
    object: &Value,
    source: usize,
    report: &mut IngestReport,
    ids: &mut Vec<(String, EntityKey)>,
) {
    let name = string(object, "name").unwrap_or_else(|| "indicator".into());
    let pattern = string(object, "pattern").unwrap_or_default();
    let details = format!(
        "{}\npattern: {pattern}",
        string(object, "description").unwrap_or_default()
    );
    report.batch.add_external_finding(
        sanitize_line(&name).into_owned(),
        sanitize_text(&details).into_owned(),
        Confidence::Medium,
        Some(source),
    );
    for (kind, value) in extract_pattern_values(&pattern) {
        let key = report.batch.entity(kind, &value, Some(source));
        remember_id(object, key, ids);
    }
}

fn ingest_observed_data(
    object: &Value,
    source: usize,
    report: &mut IngestReport,
    ids: &mut Vec<(String, EntityKey)>,
) {
    let sco = object.get("objects").and_then(Value::as_object);
    let mut first_sco = None;
    let mut keys = Vec::new();
    if let Some(map) = sco {
        for nested in map.values() {
            if let Some(key) = sco_to_entity(nested, source, report) {
                if first_sco.is_none() {
                    first_sco = Some(key.clone());
                }
                remember_id(nested, key.clone(), ids);
                keys.push(key);
            }
        }
    }
    if let Some(first) = first_sco {
        remember_id(object, first, ids);
    }
    ingest_observed_window(object, source, &keys, report);
}

fn ingest_observed_window(
    object: &Value,
    source: usize,
    keys: &[EntityKey],
    report: &mut IngestReport,
) {
    let Some(started_at) =
        string(object, "first_observed").and_then(|text| parse_timestamp_untrusted(&text))
    else {
        return;
    };
    let ended_at = super::interval_end_or_skip(
        started_at,
        string(object, "last_observed").and_then(|text| parse_timestamp_untrusted(&text)),
        report,
        "observed-data last_observed",
    );
    let title = string(object, "id").unwrap_or_else(|| "STIX observed-data".into());
    let mut fields = Fields::new();
    fields.set("stix_type", "observed-data");
    if let Some(stix_id) = string(object, "id") {
        fields.set("stix_id", stix_id);
    }
    report.batch.case_event(NewEvent {
        started_at,
        ended_at,
        kind: TimelineKind::Observed,
        title,
        description: string(object, "description").unwrap_or_default(),
        source: Some(source),
        entities: keys.to_vec(),
        fields,
    });
}

fn ingest_malware(
    object: &Value,
    source: usize,
    report: &mut IngestReport,
    ids: &mut Vec<(String, EntityKey)>,
) {
    let name = string(object, "name").unwrap_or_else(|| fallback_id(object));
    let key = report
        .batch
        .entity(EntityKind::Unknown, &name, Some(source));
    report.batch.annotate(&key, "stix_type", "malware");
    if let Some(description) = string(object, "description") {
        report.batch.annotate(&key, "description", description);
    }
    report.batch.label(&key, name.clone());
    report.batch.add_external_finding(
        format!("malware: {name}"),
        string(object, "description").unwrap_or_default(),
        Confidence::Medium,
        Some(source),
    );
    remember_id(object, key, ids);
}

fn ingest_identity(
    object: &Value,
    source: usize,
    report: &mut IngestReport,
    ids: &mut Vec<(String, EntityKey)>,
) {
    let name = string(object, "name").unwrap_or_else(|| fallback_id(object));
    let class = string(object, "identity_class").unwrap_or_default();
    let mut kind = if class.eq_ignore_ascii_case("individual") {
        EntityKind::Person
    } else {
        EntityKind::Organization
    };
    if let Some(override_kind) = string(object, "x_osint_kind") {
        kind = EntityKind::parse(&override_kind);
    }
    let key = report.batch.entity(kind, &name, Some(source));
    report.batch.label(&key, name);
    remember_id(object, key, ids);
}

fn ingest_relationship(
    object: &Value,
    source: usize,
    report: &mut IngestReport,
    ids: &[(String, EntityKey)],
) {
    let Some(from_id) = string(object, "source_ref") else {
        report.skipped += 1;
        return;
    };
    let Some(to_id) = string(object, "target_ref") else {
        report.skipped += 1;
        return;
    };
    let Some(from) = lookup(ids, &from_id) else {
        report.skipped += 1;
        report
            .warnings
            .push(format!("STIX relationship source_ref {from_id} is unknown"));
        return;
    };
    let Some(to) = lookup(ids, &to_id) else {
        report.skipped += 1;
        report
            .warnings
            .push(format!("STIX relationship target_ref {to_id} is unknown"));
        return;
    };
    let mapped =
        map_stix_relationship(string(object, "relationship_type").as_deref().unwrap_or(""));
    let custom = string(object, "x_osint_relationship");
    let kind = custom
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(mapped);
    report.batch.relate(&from, kind, &to, Some(source));
}

fn collect_marking_definitions(objects: &[&Value]) -> HashMap<String, Marking> {
    let mut defs = HashMap::new();
    for object in objects.iter().take(MAX_OBJECTS) {
        if object.get("type").and_then(Value::as_str) != Some("marking-definition") {
            continue;
        }
        let Some(id) = string(object, "id") else {
            continue;
        };
        if let Some(marking) = parse_marking_definition(object) {
            defs.insert(id, marking);
        }
    }
    defs
}

fn parse_marking_definition(object: &Value) -> Option<Marking> {
    if let Some(custom) = string(object, "x_osint_marking")
        && let Ok(parsed) = Marking::parse(&custom)
    {
        return Some(parsed);
    }
    if let Some(id) = string(object, "id")
        && let Some(marking) = marking_from_stix_id(&id)
    {
        return Some(marking);
    }
    let definition = object.get("definition")?;
    if let Some(tlp) = definition.get("tlp").and_then(Value::as_str) {
        return Marking::parse(tlp).ok();
    }
    if let Some(statement) = definition.get("statement").and_then(Value::as_str) {
        return Marking::parse(statement).ok();
    }
    object
        .get("name")
        .and_then(Value::as_str)
        .and_then(|name| Marking::parse(name).ok())
}

fn marking_from_refs(object: &Value, defs: &HashMap<String, Marking>) -> Marking {
    let mut marking = Marking::Unmarked;
    if let Some(custom) = string(object, "x_osint_marking")
        && let Ok(parsed) = Marking::parse(&custom)
    {
        marking = marking.restrict(parsed);
    }
    let Some(refs) = object.get("object_marking_refs").and_then(Value::as_array) else {
        return marking;
    };
    for value in refs {
        let Some(id) = value.as_str() else {
            continue;
        };
        if let Some(&def) = defs.get(id) {
            marking = marking.restrict(def);
        } else if let Some(from_id) = marking_from_stix_id(id) {
            marking = marking.restrict(from_id);
        }
    }
    marking
}

fn apply_imported_marking(
    object: &Value,
    defs: &HashMap<String, Marking>,
    ids: &[(String, EntityKey)],
    report: &mut IngestReport,
) {
    let Some(stix_id) = string(object, "id") else {
        return;
    };
    let Some(key) = lookup(ids, &stix_id) else {
        return;
    };
    let marking = marking_from_refs(object, defs);
    if marking == Marking::Unmarked {
        return;
    }
    report
        .batch
        .annotate(&key, OBJECT_MARKING_FIELD, marking.as_str());
}

fn ingest_top_level_sco(
    object: &Value,
    source: usize,
    report: &mut IngestReport,
    ids: &mut Vec<(String, EntityKey)>,
) {
    if let Some(entity_key) = sco_to_entity(object, source, report) {
        remember_id(object, entity_key, ids);
        return;
    }
    report.skipped += 1;
    report.warnings.push(format!(
        "skipped STIX cyber observable {}",
        fallback_id(object)
    ));
}

fn sco_to_entity(object: &Value, source: usize, report: &mut IngestReport) -> Option<EntityKey> {
    let type_name = object.get("type").and_then(Value::as_str)?;
    let (mut kind, raw) = match type_name {
        "domain-name" | "hostname" => (
            if type_name == "hostname" {
                EntityKind::Hostname
            } else {
                EntityKind::Domain
            },
            string(object, "value")?,
        ),
        "ipv4-addr" | "ipv6-addr" => (EntityKind::Ip, string(object, "value")?),
        "email-addr" => (EntityKind::Email, string(object, "value")?),
        "url" => (EntityKind::Url, string(object, "value")?),
        "user-account" => (
            EntityKind::Username,
            string(object, "user_id")
                .or_else(|| string(object, "account_login"))
                .or_else(|| string(object, "display_name"))?,
        ),
        "autonomous-system" => (
            EntityKind::Asn,
            object
                .get("number")
                .and_then(|number| {
                    number
                        .as_u64()
                        .map(|n| n.to_string())
                        .or_else(|| number.as_str().map(ToString::to_string))
                })
                .or_else(|| string(object, "name"))?,
        ),
        "file" => (
            EntityKind::File,
            string(object, "name").or_else(|| hash_value(object))?,
        ),
        "x509-certificate" => (
            EntityKind::Certificate,
            hash_value(object).or_else(|| string(object, "subject"))?,
        ),
        _ => return None,
    };
    if let Some(override_kind) = string(object, "x_osint_kind") {
        kind = EntityKind::parse(&override_kind);
    }
    Some(report.batch.entity(kind, &raw, Some(source)))
}

fn hash_value(object: &Value) -> Option<String> {
    object
        .get("hashes")
        .and_then(Value::as_object)
        .and_then(|hashes| {
            hashes
                .get("SHA-256")
                .or_else(|| hashes.get("MD5"))
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
}

fn extract_pattern_values(pattern: &str) -> Vec<(EntityKind, String)> {
    let Some(regex) = regex::Regex::new(PATTERN).ok() else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for capture in regex.captures_iter(pattern) {
        let Some(raw) = capture.get(1).map(|matched| matched.as_str()) else {
            continue;
        };
        let haystack = capture.get(0).map(|matched| matched.as_str()).unwrap_or("");
        let kind = if haystack.contains("domain-name") {
            EntityKind::Domain
        } else if haystack.contains("hostname") {
            EntityKind::Hostname
        } else if haystack.contains("ipv4-addr") || haystack.contains("ipv6-addr") {
            EntityKind::Ip
        } else if haystack.contains("email-addr") {
            EntityKind::Email
        } else if haystack.contains("url") {
            EntityKind::Url
        } else if haystack.contains("file") {
            EntityKind::File
        } else if haystack.contains("autonomous-system") {
            EntityKind::Asn
        } else {
            EntityKind::Unknown
        };
        found.push((kind, raw.to_string()));
        if found.len() >= 80 {
            break;
        }
    }
    found
}

fn map_stix_relationship(kind: &str) -> &'static str {
    match kind.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "resolves-to" => relation::RESOLVES_TO,
        "based-on" | "derived-from" => relation::REFERENCES,
        "attributed-to" => relation::OPERATED_BY,
        "variant-of" | "duplicate-of" => relation::ALIAS_OF,
        "indicates" | "related-to" | "uses" | "located-at" => relation::ASSOCIATED_WITH,
        _ => relation::ASSOCIATED_WITH,
    }
}

fn remember_id(object: &Value, key: EntityKey, ids: &mut Vec<(String, EntityKey)>) {
    if let Some(id) = string(object, "id") {
        ids.push((id, key));
    }
}

fn lookup(ids: &[(String, EntityKey)], id: &str) -> Option<EntityKey> {
    ids.iter()
        .find(|(candidate, _)| candidate == id)
        .map(|(_, key)| key.clone())
}

fn fallback_id(object: &Value) -> String {
    string(object, "id").unwrap_or_else(|| "stix-object".into())
}

fn string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| sanitize_line(text).into_owned())
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Provenance;

    const BUNDLE: &str = r#"{
      "type": "bundle",
      "id": "bundle--1",
      "spec_version": "2.1",
      "objects": [
        {"type": "identity", "id": "identity--org", "name": "Example Org", "identity_class": "organization"},
        {"type": "malware", "id": "malware--x", "name": "example-malware", "description": "toy"},
        {"type": "indicator", "id": "indicator--1", "name": "bad domain",
         "pattern": "[domain-name:value = 'evil.example'] AND [ipv4-addr:value = '192.0.2.55']"},
        {"type": "observed-data", "id": "observed-data--1",
         "first_observed": "2024-05-01T12:00:00Z",
         "last_observed": "2024-05-01T18:00:00Z",
         "objects": {
            "0": {"type": "url", "value": "https://evil.example/path"},
            "1": {"type": "email-addr", "value": "bad@evil.example"}
        }},
        {"type": "relationship", "id": "relationship--1", "source_ref": "malware--x",
         "target_ref": "identity--org", "relationship_type": "attributed-to"}
      ]
    }"#;

    #[test]
    fn stix_subset_maps_objects_and_links() {
        let actual = parse_stix(BUNDLE, "sample.json").expect("parse");
        assert!(
            actual
                .batch
                .entities
                .iter()
                .any(|entity| entity.key.value == "evil.example")
        );
        assert!(
            actual
                .batch
                .entities
                .iter()
                .any(|entity| entity.key.kind == EntityKind::Ip
                    && entity.key.value == "192.0.2.55")
        );
        assert!(
            actual
                .batch
                .entities
                .iter()
                .any(|entity| entity.key.kind == EntityKind::Url)
        );
        assert!(
            actual
                .batch
                .relationships
                .iter()
                .any(|relationship| relationship.kind == relation::OPERATED_BY)
        );
        assert_eq!(actual.batch.sources[0].kind, SourceKind::Stix);
        assert_eq!(actual.batch.sources[0].provenance, Provenance::External);
        assert!(
            actual
                .batch
                .findings
                .iter()
                .any(|finding| finding.title.contains("bad domain")
                    && finding.provenance == Provenance::External)
        );
        assert!(
            actual
                .batch
                .findings
                .iter()
                .all(|finding| finding.provenance == Provenance::External)
        );
        assert!(
            !actual
                .batch
                .entities
                .iter()
                .any(|entity| entity.key.kind == EntityKind::Unknown
                    && entity.key.value.starts_with("observed-data--"))
        );
        assert_eq!(actual.batch.events.len(), 1);
        assert!(actual.batch.events[0].ended_at.is_some());
        assert_eq!(actual.batch.events[0].entities.len(), 2);
    }

    #[test]
    fn malformed_observed_timestamps_do_not_panic() {
        let input = r#"{
          "type": "bundle",
          "objects": [
            {"type": "observed-data", "id": "observed-data--bad",
             "first_observed": "NaN",
             "last_observed": "not-a-date",
             "objects": {"0": {"type": "ipv4-addr", "value": "192.0.2.9"}}}
          ]
        }"#;
        let actual = parse_stix(input, "bad.json").expect("parse");
        assert!(actual.batch.events.is_empty());
        assert!(
            actual
                .batch
                .entities
                .iter()
                .any(|entity| entity.key.value == "192.0.2.9")
        );

        let inverted = r#"{
          "type": "bundle",
          "objects": [
            {"type": "observed-data", "id": "observed-data--back",
             "first_observed": "2024-05-02T00:00:00Z",
             "last_observed": "2024-05-01T00:00:00Z",
             "objects": {"0": {"type": "ipv4-addr", "value": "192.0.2.8"}}}
          ]
        }"#;
        let actual = parse_stix(inverted, "back.json").expect("parse");
        assert_eq!(actual.batch.events.len(), 1);
        assert!(actual.batch.events[0].ended_at.is_none());
        assert_eq!(actual.skipped, 1);
        assert!(
            actual
                .warnings
                .iter()
                .any(|warning| warning.contains("ended_at precedes started_at"))
        );
    }

    #[test]
    fn relationship_before_sdos_still_resolves() {
        let input = r#"{
          "type": "bundle",
          "objects": [
            {"type": "relationship", "id": "relationship--1", "source_ref": "malware--x",
             "target_ref": "identity--org", "relationship_type": "attributed-to"},
            {"type": "identity", "id": "identity--org", "name": "Example Org", "identity_class": "organization"},
            {"type": "malware", "id": "malware--x", "name": "example-malware"}
          ]
        }"#;
        let actual = parse_stix(input, "order.json").expect("parse");
        assert!(
            actual
                .batch
                .relationships
                .iter()
                .any(|relationship| relationship.kind == relation::OPERATED_BY)
        );
    }

    #[test]
    fn top_level_scos_restore_resolves_to() {
        let input = r#"{
          "type": "bundle",
          "spec_version": "2.1",
          "objects": [
            {"type": "domain-name", "id": "domain-name--1", "value": "example.com",
             "x_osint_kind": "domain"},
            {"type": "ipv4-addr", "id": "ipv4-addr--1", "value": "192.0.2.10"},
            {"type": "hostname", "id": "hostname--1", "value": "ns1.example.net",
             "x_osint_kind": "hostname"},
            {"type": "relationship", "id": "relationship--1",
             "source_ref": "domain-name--1", "target_ref": "ipv4-addr--1",
             "relationship_type": "resolves-to", "x_osint_relationship": "resolves_to"},
            {"type": "relationship", "id": "relationship--2",
             "source_ref": "domain-name--1", "target_ref": "hostname--1",
             "relationship_type": "related-to", "x_osint_relationship": "uses_nameserver"}
          ]
        }"#;
        let actual = parse_stix(input, "scos.json").expect("parse");
        assert!(actual.batch.entities.iter().any(
            |entity| entity.key.kind == EntityKind::Domain && entity.key.value == "example.com"
        ));
        assert!(
            actual
                .batch
                .entities
                .iter()
                .any(|entity| entity.key.kind == EntityKind::Hostname
                    && entity.key.value == "ns1.example.net")
        );
        assert!(
            actual
                .batch
                .relationships
                .iter()
                .any(|relationship| relationship.kind == relation::RESOLVES_TO)
        );
        assert!(
            actual
                .batch
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "uses_nameserver")
        );
    }

    #[test]
    fn malformed_stix_is_an_error() {
        assert!(parse_stix("{", "x").is_err());
        assert!(parse_stix("[]", "x").is_err());
        assert!(parse_stix(r#"{"type":"bundle","objects":[]}"#, "x").is_err());
    }
}
