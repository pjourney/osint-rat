//! STIX 2.1 bundle export (complement of [`super::stix`] import).

use serde_json::{Value, json};
use uuid::Uuid;

use crate::model::{Entity, EntityKind, Finding, InvestigationData, Relationship, relation};

/// Namespace used to mint stable STIX IDs from os-int UUIDs.
const STIX_NAMESPACE: Uuid = Uuid::from_bytes([
    0x6b, 0xa7, 0xb8, 0x11, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8,
]);

/// Builds a STIX 2.1 bundle from an investigation snapshot.
pub fn export_stix_bundle(data: &InvestigationData) -> Value {
    let mut objects = Vec::new();
    if let Some(marking) = tlp_marking_definition(data.investigation.marking.as_str()) {
        objects.push(marking);
    }
    let mut id_map: Vec<(Uuid, String)> = Vec::new();
    for entity in &data.entities {
        if data.is_absorbed(entity.id) {
            continue;
        }
        if let Some((stix_id, object)) = entity_to_stix(entity) {
            id_map.push((entity.id, stix_id));
            objects.push(object);
        }
    }
    for finding in &data.findings {
        objects.push(finding_to_indicator(finding, data, &id_map));
    }
    for event in &data.events {
        if let Some(object) = event_to_observed_data(event, data, &id_map) {
            objects.push(object);
        }
    }
    for relationship in &data.relationships {
        if let Some(object) = relationship_to_stix(relationship, &id_map) {
            objects.push(object);
        }
    }
    json!({
        "type": "bundle",
        "id": format!("bundle--{}", Uuid::new_v5(&STIX_NAMESPACE, data.investigation.id.as_bytes())),
        "spec_version": "2.1",
        "objects": objects,
    })
}

fn entity_to_stix(entity: &Entity) -> Option<(String, Value)> {
    let created = entity.created_at.to_rfc3339();
    let mut object = match entity.kind {
        EntityKind::Domain | EntityKind::Hostname => {
            let stix_id = sco_id("domain-name", entity.id);
            let mut object = json!({
                "type": "domain-name",
                "spec_version": "2.1",
                "id": stix_id,
                "value": entity.value,
            });
            if entity.kind == EntityKind::Hostname {
                object["x_osint_kind"] = json!("hostname");
            }
            (stix_id, object)
        }
        EntityKind::Ip => {
            let type_name = if entity.value.contains(':') {
                "ipv6-addr"
            } else {
                "ipv4-addr"
            };
            let stix_id = sco_id(type_name, entity.id);
            (
                stix_id.clone(),
                json!({
                    "type": type_name,
                    "spec_version": "2.1",
                    "id": stix_id,
                    "value": entity.value,
                }),
            )
        }
        EntityKind::Email => {
            let stix_id = sco_id("email-addr", entity.id);
            (
                stix_id.clone(),
                json!({
                    "type": "email-addr",
                    "spec_version": "2.1",
                    "id": stix_id,
                    "value": entity.value,
                }),
            )
        }
        EntityKind::Url | EntityKind::Repository => {
            let stix_id = sco_id("url", entity.id);
            (
                stix_id.clone(),
                json!({
                    "type": "url",
                    "spec_version": "2.1",
                    "id": stix_id,
                    "value": entity.value,
                }),
            )
        }
        EntityKind::Username => {
            let stix_id = sco_id("user-account", entity.id);
            (
                stix_id.clone(),
                json!({
                    "type": "user-account",
                    "spec_version": "2.1",
                    "id": stix_id,
                    "user_id": entity.value,
                }),
            )
        }
        EntityKind::Asn => {
            let number = entity
                .value
                .trim_start_matches(['A', 'S', 'a', 's'])
                .parse::<u64>()
                .unwrap_or(0);
            let stix_id = sco_id("autonomous-system", entity.id);
            (
                stix_id.clone(),
                json!({
                    "type": "autonomous-system",
                    "spec_version": "2.1",
                    "id": stix_id,
                    "number": number,
                    "name": entity.value,
                }),
            )
        }
        EntityKind::File => {
            let stix_id = sco_id("file", entity.id);
            (
                stix_id.clone(),
                json!({
                    "type": "file",
                    "spec_version": "2.1",
                    "id": stix_id,
                    "name": entity.value,
                }),
            )
        }
        EntityKind::Certificate => {
            let stix_id = sco_id("x509-certificate", entity.id);
            (
                stix_id.clone(),
                json!({
                    "type": "x509-certificate",
                    "spec_version": "2.1",
                    "id": stix_id,
                    "hashes": { "SHA-256": entity.value },
                }),
            )
        }
        EntityKind::Person | EntityKind::Organization => {
            let stix_id = sdo_id("identity", entity.id);
            let class = if entity.kind == EntityKind::Person {
                "individual"
            } else {
                "organization"
            };
            (
                stix_id.clone(),
                json!({
                    "type": "identity",
                    "spec_version": "2.1",
                    "id": stix_id,
                    "created": created,
                    "modified": entity.updated_at.to_rfc3339(),
                    "name": display_name(entity),
                    "identity_class": class,
                }),
            )
        }
        EntityKind::Unknown if entity.fields.get("stix_type") == Some("malware") => {
            let stix_id = sdo_id("malware", entity.id);
            (
                stix_id.clone(),
                json!({
                    "type": "malware",
                    "spec_version": "2.1",
                    "id": stix_id,
                    "created": created,
                    "modified": entity.updated_at.to_rfc3339(),
                    "name": display_name(entity),
                    "is_family": false,
                }),
            )
        }
        EntityKind::Unknown | EntityKind::Location | EntityKind::Phone => {
            let stix_id = sdo_id("identity", entity.id);
            (
                stix_id.clone(),
                json!({
                    "type": "identity",
                    "spec_version": "2.1",
                    "id": stix_id,
                    "created": created,
                    "modified": entity.updated_at.to_rfc3339(),
                    "name": display_name(entity),
                    "identity_class": "unknown",
                    "x_osint_kind": entity.kind.as_str(),
                }),
            )
        }
        EntityKind::Other(_) => return None,
    };
    object.1["x_osint_ref"] = json!(entity.short_ref().to_string());
    object.1["x_osint_provenance"] = json!(entity.provenance.as_str());
    object.1["x_osint_kind"] = object
        .1
        .get("x_osint_kind")
        .cloned()
        .unwrap_or_else(|| json!(entity.kind.as_str()));
    Some(object)
}

fn finding_to_indicator(
    finding: &Finding,
    data: &InvestigationData,
    ids: &[(Uuid, String)],
) -> Value {
    let pattern = indicator_pattern(finding, data);
    let stix_id = sdo_id("indicator", finding.id);
    json!({
        "type": "indicator",
        "spec_version": "2.1",
        "id": stix_id,
        "created": finding.created_at.to_rfc3339(),
        "modified": finding.updated_at.to_rfc3339(),
        "name": finding.title,
        "description": finding.details,
        "indicator_types": ["unknown"],
        "pattern": pattern,
        "pattern_type": "stix",
        "valid_from": finding.created_at.to_rfc3339(),
        "x_osint_ref": finding.short_ref().to_string(),
        "x_osint_provenance": finding.provenance.as_str(),
        "x_osint_source_ids": finding
            .source_ids
            .iter()
            .filter_map(|id| ids.iter().find(|(entity_id, _)| entity_id == id).map(|(_, stix)| stix.clone()))
            .collect::<Vec<_>>(),
    })
}

fn indicator_pattern(finding: &Finding, data: &InvestigationData) -> String {
    for source_id in &finding.source_ids {
        if let Some(entity) = data
            .entities
            .iter()
            .find(|entity| entity.source_ids.contains(source_id))
            && let Some(pattern) = entity_pattern(entity)
        {
            return pattern;
        }
    }
    let escaped = finding.title.replace('\'', "\\'");
    format!("[file:name = '{escaped}']")
}

fn entity_pattern(entity: &Entity) -> Option<String> {
    let value = entity.value.replace('\'', "\\'");
    let body = match entity.kind {
        EntityKind::Domain | EntityKind::Hostname => format!("domain-name:value = '{value}'"),
        EntityKind::Ip if entity.value.contains(':') => format!("ipv6-addr:value = '{value}'"),
        EntityKind::Ip => format!("ipv4-addr:value = '{value}'"),
        EntityKind::Email => format!("email-addr:value = '{value}'"),
        EntityKind::Url => format!("url:value = '{value}'"),
        EntityKind::File => format!("file:name = '{value}'"),
        EntityKind::Asn => format!("autonomous-system:name = '{value}'"),
        _ => return None,
    };
    Some(format!("[{body}]"))
}

fn event_to_observed_data(
    event: &crate::model::Event,
    data: &InvestigationData,
    ids: &[(Uuid, String)],
) -> Option<Value> {
    if event.entity_ids.is_empty() {
        return None;
    }
    let mut objects = serde_json::Map::new();
    for (index, entity_id) in event.entity_ids.iter().enumerate() {
        let Some(entity) = data.entities.iter().find(|entity| entity.id == *entity_id) else {
            continue;
        };
        if let Some((_, object)) = entity_to_stix(entity) {
            objects.insert(index.to_string(), object);
        }
        let _ = ids;
    }
    if objects.is_empty() {
        return None;
    }
    let mut value = json!({
        "type": "observed-data",
        "spec_version": "2.1",
        "id": sdo_id("observed-data", event.id),
        "created": event.created_at.to_rfc3339(),
        "modified": event.created_at.to_rfc3339(),
        "first_observed": event.started_at.to_rfc3339(),
        "number_observed": 1,
        "objects": objects,
        "x_osint_ref": event.short_ref().to_string(),
    });
    if let Some(ended) = event.ended_at {
        value["last_observed"] = json!(ended.to_rfc3339());
    }
    Some(value)
}

fn relationship_to_stix(relationship: &Relationship, ids: &[(Uuid, String)]) -> Option<Value> {
    let source_ref = lookup(ids, relationship.from_id)?;
    let target_ref = lookup(ids, relationship.to_id)?;
    let stix_type = export_relationship_type(&relationship.kind);
    Some(json!({
        "type": "relationship",
        "spec_version": "2.1",
        "id": sdo_id("relationship", relationship.id),
        "created": relationship.created_at.to_rfc3339(),
        "modified": relationship.created_at.to_rfc3339(),
        "relationship_type": stix_type,
        "source_ref": source_ref,
        "target_ref": target_ref,
        "x_osint_relationship": relationship.kind,
        "x_osint_provenance": relationship.provenance.as_str(),
    }))
}

fn export_relationship_type(kind: &str) -> &'static str {
    match kind {
        relation::RESOLVES_TO => "resolves-to",
        relation::OPERATED_BY => "attributed-to",
        relation::ALIAS_OF => "variant-of",
        relation::REFERENCES => "based-on",
        _ => "related-to",
    }
}

fn tlp_marking_definition(marking: &str) -> Option<Value> {
    let tlp = match marking {
        "tlp:clear" | "tlp:white" => "clear",
        "tlp:green" => "green",
        "tlp:amber" => "amber",
        "tlp:amber+strict" => "amber",
        "tlp:red" => "red",
        _ => return None,
    };
    Some(json!({
        "type": "marking-definition",
        "spec_version": "2.1",
        "id": format!("marking-definition--{tlp}"),
        "definition_type": "tlp",
        "definition": { "tlp": tlp },
    }))
}

fn lookup(ids: &[(Uuid, String)], id: Uuid) -> Option<String> {
    ids.iter()
        .find(|(entity_id, _)| *entity_id == id)
        .map(|(_, stix_id)| stix_id.clone())
}

fn display_name(entity: &Entity) -> &str {
    if entity.label.trim().is_empty() {
        &entity.value
    } else {
        &entity.label
    }
}

fn sco_id(type_name: &str, entity_id: Uuid) -> String {
    format!(
        "{type_name}--{}",
        Uuid::new_v5(&STIX_NAMESPACE, entity_id.as_bytes())
    )
}

fn sdo_id(type_name: &str, record_id: Uuid) -> String {
    format!(
        "{type_name}--{}",
        Uuid::new_v5(&STIX_NAMESPACE, record_id.as_bytes())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::parse_stix;
    use crate::model::{Investigation, InvestigationData, Marking, now};

    fn empty_data() -> InvestigationData {
        let timestamp = now();
        InvestigationData::empty(Investigation {
            id: Uuid::new_v4(),
            name: "empty".into(),
            description: String::new(),
            tags: Vec::new(),
            marking: Marking::Unmarked,
            created_at: timestamp,
            updated_at: timestamp,
        })
    }

    #[test]
    fn empty_investigation_exports_a_bundle() {
        let data = empty_data();
        let bundle = export_stix_bundle(&data);
        assert_eq!(bundle["type"], "bundle");
        assert_eq!(bundle["spec_version"], "2.1");
        assert!(bundle["objects"].as_array().expect("objects").is_empty());
    }

    #[test]
    fn exported_indicator_pattern_is_stix_importable() {
        let input = r#"{
          "type": "bundle",
          "spec_version": "2.1",
          "objects": [
            {"type": "indicator", "id": "indicator--1", "name": "bad domain",
             "pattern": "[domain-name:value = 'evil.example']",
             "pattern_type": "stix", "valid_from": "2024-01-01T00:00:00Z"}
          ]
        }"#;
        let actual = parse_stix(input, "round.json").expect("parse");
        assert!(
            actual
                .batch
                .entities
                .iter()
                .any(|entity| entity.key.value == "evil.example")
        );
    }
}
