//! MISP event JSON import. Attributes become objects and links best-effort.

use serde_json::Value;

use super::{IngestError, IngestReport, MAX_OBJECTS};
use crate::files::sha256_hex;
use crate::model::{EntityKey, EntityKind, EvidenceBatch, NewSource, SourceKind, relation};
use crate::sanitize::{sanitize_line, sanitize_text};

/// Parses a MISP event document (`{"Event": {...}}` or a bare event object).
pub fn parse_misp(text: &str, origin: &str) -> Result<IngestReport, IngestError> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| IngestError::new(format!("MISP JSON is invalid: {error}")))?;
    let event = value.get("Event").unwrap_or(&value);
    if !event.is_object() {
        return Err(IngestError::new("MISP document has no Event object"));
    }
    let info = event
        .get("info")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|info| !info.is_empty())
        .unwrap_or("MISP event");
    let mut report = IngestReport::new(EvidenceBatch::new());
    let mut source = NewSource::external(
        SourceKind::Misp,
        "ingest:misp",
        format!("MISP event: {}", sanitize_line(info)),
    );
    source.content = sanitize_text(&truncate(text, 32 * 1024)).into_owned();
    source.content_hash = Some(sha256_hex(text.as_bytes()));
    source.fields.push("origin", origin);
    if let Some(uuid) = event.get("uuid").and_then(Value::as_str) {
        source.fields.push("misp_uuid", uuid);
    }
    if let Some(id) = event.get("id").and_then(|id| {
        id.as_str()
            .map(ToString::to_string)
            .or_else(|| id.as_u64().map(|n| n.to_string()))
    }) {
        source.fields.push("misp_id", id);
    }
    let key = report.batch.add_source(source);
    let mut attributes: Vec<&Value> = Vec::new();
    push_attributes(event.get("Attribute"), &mut attributes);
    if let Some(objects) = event.get("Object").and_then(Value::as_array) {
        for object in objects {
            push_attributes(object.get("Attribute"), &mut attributes);
            ingest_object_links(object, key, &mut report);
        }
    }
    for (index, attribute) in attributes.iter().enumerate() {
        if index >= MAX_OBJECTS {
            report.skipped += attributes.len() - MAX_OBJECTS;
            report
                .warnings
                .push(format!("truncated MISP attributes at {MAX_OBJECTS}"));
            break;
        }
        match ingest_attribute(attribute, key, &mut report) {
            Ok(()) => {}
            Err(error) => {
                report.skipped += 1;
                report
                    .warnings
                    .push(format!("attribute {index}: {}", error.message));
            }
        }
    }
    if report.batch.entities.is_empty() {
        return Err(IngestError::new(
            "MISP event contained no attributes that map to entities",
        ));
    }
    report.batch.summary = format!(
        "MISP import: {} entit(y/ies), {} relationship(s) from {origin}",
        report.batch.entities.len(),
        report.batch.relationships.len()
    );
    Ok(report)
}

fn push_attributes<'a>(value: Option<&'a Value>, output: &mut Vec<&'a Value>) {
    if let Some(items) = value.and_then(Value::as_array) {
        output.extend(items);
    }
}

fn ingest_attribute(
    attribute: &Value,
    source: usize,
    report: &mut IngestReport,
) -> Result<(), IngestError> {
    let type_name = attribute
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let raw = attribute
        .get("value")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| IngestError::new("attribute has no value"))?;
    let Some((kind, value)) = map_misp_type(type_name, raw) else {
        return Err(IngestError::new(format!(
            "unsupported MISP attribute type {type_name}"
        )));
    };
    if value.len() > 2048 {
        return Err(IngestError::new("attribute value is too long"));
    }
    let key = report.batch.entity(kind.clone(), &value, Some(source));
    if let Some(category) = attribute.get("category").and_then(Value::as_str) {
        report.batch.annotate(&key, "misp_category", category);
    }
    if let Some(comment) = attribute.get("comment").and_then(Value::as_str) {
        report
            .batch
            .annotate(&key, "comment", sanitize_line(comment).into_owned());
    }
    if kind == EntityKind::Email
        && let Some((_, domain)) = value.rsplit_once('@')
    {
        let domain_key = report
            .batch
            .entity(EntityKind::Domain, domain, Some(source));
        report
            .batch
            .relate(&key, relation::HOSTED_ON, &domain_key, Some(source));
    }
    Ok(())
}

fn ingest_object_links(object: &Value, source: usize, report: &mut IngestReport) {
    let Some(attributes) = object.get("Attribute").and_then(Value::as_array) else {
        return;
    };
    let mut keys: Vec<EntityKey> = Vec::new();
    for attribute in attributes {
        let type_name = attribute.get("type").and_then(Value::as_str).unwrap_or("");
        let Some(value) = attribute.get("value").and_then(Value::as_str) else {
            continue;
        };
        if let Some((kind, raw)) = map_misp_type(type_name, value) {
            keys.push(EntityKey::new(kind, &raw));
        }
    }
    for pair in keys.windows(2) {
        report
            .batch
            .relate(&pair[0], relation::ASSOCIATED_WITH, &pair[1], Some(source));
    }
}

fn map_misp_type(type_name: &str, raw: &str) -> Option<(EntityKind, String)> {
    let value = raw.split('|').next().unwrap_or(raw).trim();
    if value.is_empty() {
        return None;
    }
    let kind = match type_name {
        "ip-src" | "ip-dst" | "ip-src|port" | "ip-dst|port" | "ip" => EntityKind::Ip,
        "domain" | "domain|ip" => EntityKind::Domain,
        "hostname" | "hostname|port" => EntityKind::Hostname,
        "email"
        | "email-src"
        | "email-dst"
        | "email-src-display-name"
        | "whois-registrant-email" => EntityKind::Email,
        "url" | "uri" | "link" => EntityKind::Url,
        "filename" | "filename|md5" | "filename|sha256" | "sha256" | "md5" | "sha1" => {
            EntityKind::File
        }
        "github-username" | "twitter-id" | "twitter-screen-name" => EntityKind::Username,
        "github-organisation" => EntityKind::Organization,
        "AS" | "asn" | "as" => EntityKind::Asn,
        "phone-number" | "whois-registrant-phone" => EntityKind::Phone,
        "whois-registrant-name" => EntityKind::Person,
        "whois-registrar" | "whois-registrant-org" | "organization" => EntityKind::Organization,
        _ => return None,
    };
    Some((kind, value.to_string()))
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

    const EVENT: &str = r#"{
      "Event": {
        "id": "42",
        "uuid": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
        "info": "Toy campaign",
        "Attribute": [
          {"type": "domain", "value": "evil.example", "category": "Network activity"},
          {"type": "ip-dst", "value": "192.0.2.88", "comment": "c2"},
          {"type": "email-src", "value": "bad@evil.example"},
          {"type": "github-username", "value": "octocat"},
          {"type": "comment", "value": "ignore me"}
        ],
        "Object": [
          {"name": "domain-ip", "Attribute": [
            {"type": "domain", "value": "c2.evil.example"},
            {"type": "ip-dst", "value": "192.0.2.89"}
          ]}
        ]
      }
    }"#;

    #[test]
    fn misp_attributes_become_cited_entities() {
        let actual = parse_misp(EVENT, "event.json").expect("parse");
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
                .any(|entity| entity.key.kind == EntityKind::Ip)
        );
        assert!(
            actual
                .batch
                .relationships
                .iter()
                .any(|relationship| relationship.kind == relation::HOSTED_ON)
        );
        assert_eq!(actual.batch.sources[0].kind, SourceKind::Misp);
        assert_eq!(actual.batch.sources[0].provenance, Provenance::External);
        assert!(actual.skipped >= 1);
    }

    #[test]
    fn malformed_misp_is_an_error() {
        assert!(parse_misp("{", "x").is_err());
        assert!(parse_misp(r#"{"Event":{"Attribute":[]}}"#, "x").is_err());
    }
}
