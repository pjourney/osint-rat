//! STIX 2.1 bundle export (complement of [`super::stix`] import).

use std::collections::BTreeSet;

use serde_json::{Value, json};
use uuid::Uuid;

use crate::model::{
    Entity, EntityKind, Finding, InvestigationData, Marking, Relationship, relation,
};

/// OASIS STIX 2.1 TLP:WHITE (TLP 2.0 CLEAR).
pub(crate) const TLP_WHITE_ID: &str = "marking-definition--613f2e26-407d-48c7-9eca-b8e91df99dc9";
/// OASIS STIX 2.1 TLP:GREEN.
pub(crate) const TLP_GREEN_ID: &str = "marking-definition--34098fce-860f-48ae-8e50-ebd3cc5e41da";
/// OASIS STIX 2.1 TLP:AMBER.
pub(crate) const TLP_AMBER_ID: &str = "marking-definition--f88d31f6-486f-44da-b317-01333bde0b82";
/// OASIS STIX 2.1 TLP:RED.
pub(crate) const TLP_RED_ID: &str = "marking-definition--5e57c739-391a-4eb3-b6be-7d15ca92d5ed";

/// Namespace used to mint stable STIX IDs from os-int UUIDs.
const STIX_NAMESPACE: Uuid = Uuid::from_bytes([
    0x6b, 0xa7, 0xb8, 0x11, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8,
]);

/// Builds a STIX 2.1 bundle from an investigation snapshot.
pub fn export_stix_bundle(data: &InvestigationData) -> Value {
    let case = data.investigation.marking;
    let mut used_markings = BTreeSet::new();
    let mut objects = Vec::new();
    let mut id_map: Vec<(Uuid, String)> = Vec::new();
    for entity in &data.entities {
        if data.is_absorbed(entity.id) {
            continue;
        }
        if let Some((stix_id, object)) = entity_to_stix(entity, case, &mut used_markings) {
            id_map.push((entity.id, stix_id));
            objects.push(object);
        }
    }
    for finding in &data.findings {
        objects.push(finding_to_indicator(
            finding,
            data,
            case,
            &mut used_markings,
        ));
    }
    for event in &data.events {
        if let Some(object) = event_to_observed_data(event, data, case, &mut used_markings) {
            objects.push(object);
        }
    }
    for relationship in &data.relationships {
        if let Some(object) = relationship_to_stix(relationship, data, &id_map, &mut used_markings)
        {
            objects.push(object);
        }
    }
    let mut bundle_objects: Vec<Value> = used_markings
        .into_iter()
        .filter_map(tlp_marking_definition)
        .collect();
    bundle_objects.extend(objects);
    json!({
        "type": "bundle",
        "id": format!("bundle--{}", Uuid::new_v5(&STIX_NAMESPACE, data.investigation.id.as_bytes())),
        "spec_version": "2.1",
        "objects": bundle_objects,
    })
}

fn entity_to_stix(
    entity: &Entity,
    case: Marking,
    used: &mut BTreeSet<Marking>,
) -> Option<(String, Value)> {
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
    attach_marking(&mut object.1, case.restrict(entity.marking()), used);
    Some(object)
}

fn finding_to_indicator(
    finding: &Finding,
    data: &InvestigationData,
    case: Marking,
    used: &mut BTreeSet<Marking>,
) -> Value {
    let pattern = indicator_pattern(finding, data);
    let stix_id = sdo_id("indicator", finding.id);
    let mut object = json!({
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
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
    });
    attach_marking(&mut object, case, used);
    object
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
    case: Marking,
    used: &mut BTreeSet<Marking>,
) -> Option<Value> {
    if event.entity_ids.is_empty() {
        return None;
    }
    let mut objects = serde_json::Map::new();
    let mut nested_marking = case;
    for (index, entity_id) in event.entity_ids.iter().enumerate() {
        let Some(entity) = data.entities.iter().find(|entity| entity.id == *entity_id) else {
            continue;
        };
        nested_marking = nested_marking.restrict(entity.marking());
        if let Some((_, object)) = entity_to_stix(entity, case, used) {
            objects.insert(index.to_string(), object);
        }
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
    attach_marking(&mut value, nested_marking, used);
    Some(value)
}

fn relationship_to_stix(
    relationship: &Relationship,
    data: &InvestigationData,
    ids: &[(Uuid, String)],
    used: &mut BTreeSet<Marking>,
) -> Option<Value> {
    let source_ref = lookup(ids, relationship.from_id)?;
    let target_ref = lookup(ids, relationship.to_id)?;
    let stix_type = export_relationship_type(&relationship.kind);
    let mut marking = data.investigation.marking;
    if let Some(from) = data
        .entities
        .iter()
        .find(|entity| entity.id == relationship.from_id)
    {
        marking = marking.restrict(from.marking());
    }
    if let Some(to) = data
        .entities
        .iter()
        .find(|entity| entity.id == relationship.to_id)
    {
        marking = marking.restrict(to.marking());
    }
    let mut object = json!({
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
    });
    attach_marking(&mut object, marking, used);
    Some(object)
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

fn attach_marking(object: &mut Value, marking: Marking, used: &mut BTreeSet<Marking>) {
    if marking == Marking::Unmarked {
        return;
    }
    used.insert(marking);
    object["object_marking_refs"] = json!([marking_definition_id(marking)]);
    object["x_osint_marking"] = json!(marking.as_str());
}

pub(crate) fn marking_definition_id(marking: Marking) -> String {
    match marking {
        Marking::Unmarked => String::new(),
        Marking::TlpClear => TLP_WHITE_ID.to_string(),
        Marking::TlpGreen => TLP_GREEN_ID.to_string(),
        Marking::TlpAmber => TLP_AMBER_ID.to_string(),
        Marking::TlpRed => TLP_RED_ID.to_string(),
        Marking::TlpAmberStrict => amber_strict_marking_id(),
    }
}

pub(crate) fn amber_strict_marking_id() -> String {
    format!(
        "marking-definition--{}",
        Uuid::new_v5(&STIX_NAMESPACE, b"tlp:amber+strict")
    )
}

pub(crate) fn marking_from_stix_id(id: &str) -> Option<Marking> {
    if id == TLP_WHITE_ID {
        Some(Marking::TlpClear)
    } else if id == TLP_GREEN_ID {
        Some(Marking::TlpGreen)
    } else if id == TLP_AMBER_ID {
        Some(Marking::TlpAmber)
    } else if id == TLP_RED_ID {
        Some(Marking::TlpRed)
    } else if id == amber_strict_marking_id() {
        Some(Marking::TlpAmberStrict)
    } else {
        None
    }
}

fn tlp_marking_definition(marking: Marking) -> Option<Value> {
    let created = "2017-01-20T00:00:00.000Z";
    Some(match marking {
        Marking::Unmarked => return None,
        Marking::TlpClear => json!({
            "type": "marking-definition",
            "spec_version": "2.1",
            "id": TLP_WHITE_ID,
            "created": created,
            "definition_type": "tlp",
            "name": "TLP:WHITE",
            "definition": { "tlp": "white" },
            "x_osint_marking": "tlp:clear",
        }),
        Marking::TlpGreen => json!({
            "type": "marking-definition",
            "spec_version": "2.1",
            "id": TLP_GREEN_ID,
            "created": created,
            "definition_type": "tlp",
            "name": "TLP:GREEN",
            "definition": { "tlp": "green" },
        }),
        Marking::TlpAmber => json!({
            "type": "marking-definition",
            "spec_version": "2.1",
            "id": TLP_AMBER_ID,
            "created": created,
            "definition_type": "tlp",
            "name": "TLP:AMBER",
            "definition": { "tlp": "amber" },
        }),
        Marking::TlpAmberStrict => json!({
            "type": "marking-definition",
            "spec_version": "2.1",
            "id": amber_strict_marking_id(),
            "created": "2020-11-04T00:00:00.000Z",
            "definition_type": "statement",
            "name": "TLP:AMBER+STRICT",
            "definition": { "statement": "TLP:AMBER+STRICT" },
            "x_osint_marking": "tlp:amber+strict",
        }),
        Marking::TlpRed => json!({
            "type": "marking-definition",
            "spec_version": "2.1",
            "id": TLP_RED_ID,
            "created": created,
            "definition_type": "tlp",
            "name": "TLP:RED",
            "definition": { "tlp": "red" },
        }),
    })
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
    use crate::model::{
        Entity, EntityKind, Investigation, InvestigationData, Marking, OBJECT_MARKING_FIELD,
        Provenance, now,
    };

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

    #[test]
    fn tlp_red_entity_round_trips_with_oasis_marking_id() {
        let timestamp = now();
        let mut data = empty_data();
        let mut fields = crate::model::Fields::new();
        fields.set(OBJECT_MARKING_FIELD, Marking::TlpRed.as_str());
        data.entities.push(Entity {
            id: Uuid::new_v4(),
            investigation_id: data.investigation.id,
            seq: 1,
            kind: EntityKind::Domain,
            value: "secret.example".into(),
            label: String::new(),
            notes: String::new(),
            provenance: Provenance::Analyst,
            fields,
            source_ids: Vec::new(),
            created_at: timestamp,
            updated_at: timestamp,
        });
        let bundle = export_stix_bundle(&data);
        let objects = bundle["objects"].as_array().expect("objects");
        let definition = objects
            .iter()
            .find(|object| object["id"] == TLP_RED_ID)
            .expect("oasis red marking-definition");
        assert_eq!(definition["definition"]["tlp"], "red");
        let domain = objects
            .iter()
            .find(|object| object["type"] == "domain-name")
            .expect("domain");
        assert_eq!(domain["object_marking_refs"][0], TLP_RED_ID);
        assert_eq!(domain["x_osint_marking"], "tlp:red");
        let json = serde_json::to_string(&bundle).expect("json");
        let report = parse_stix(&json, "red.json").expect("parse");
        let entity = report
            .batch
            .entities
            .iter()
            .find(|entity| entity.key.value == "secret.example")
            .expect("imported");
        assert_eq!(
            entity.fields.get(OBJECT_MARKING_FIELD),
            Some(Marking::TlpRed.as_str())
        );
    }

    #[test]
    fn amber_strict_is_not_collapsed_to_amber() {
        let timestamp = now();
        let mut data = empty_data();
        let mut fields = crate::model::Fields::new();
        fields.set(OBJECT_MARKING_FIELD, Marking::TlpAmberStrict.as_str());
        data.entities.push(Entity {
            id: Uuid::new_v4(),
            investigation_id: data.investigation.id,
            seq: 1,
            kind: EntityKind::Domain,
            value: "strict.example".into(),
            label: String::new(),
            notes: String::new(),
            provenance: Provenance::Analyst,
            fields,
            source_ids: Vec::new(),
            created_at: timestamp,
            updated_at: timestamp,
        });
        let bundle = export_stix_bundle(&data);
        let objects = bundle["objects"].as_array().expect("objects");
        let strict_id = amber_strict_marking_id();
        assert!(
            objects.iter().any(|object| object["id"] == strict_id
                && object["x_osint_marking"] == "tlp:amber+strict")
        );
        assert!(objects.iter().all(|object| object["id"] != TLP_AMBER_ID));
        let domain = objects
            .iter()
            .find(|object| object["type"] == "domain-name")
            .expect("domain");
        assert_eq!(domain["object_marking_refs"][0], strict_id);
        let json = serde_json::to_string(&bundle).expect("json");
        let report = parse_stix(&json, "strict.json").expect("parse");
        let entity = report
            .batch
            .entities
            .iter()
            .find(|entity| entity.key.value == "strict.example")
            .expect("imported");
        assert_eq!(
            entity.fields.get(OBJECT_MARKING_FIELD),
            Some(Marking::TlpAmberStrict.as_str())
        );
    }
}
