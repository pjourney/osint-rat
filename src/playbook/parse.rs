//! YAML playbook parser. Untrusted input: no panics, no path traversal, no provenance writes.

use std::panic::{AssertUnwindSafe, catch_unwind};

use serde_yaml::Value;

use crate::model::{
    MAX_COLLECT_STEPS, MAX_PLAYBOOK_BYTES, MAX_PLAYBOOK_STEPS, MAX_SEARCH_HOPS, MAX_TOOL_STEPS,
    PLAYBOOK_FORMAT, PLAYBOOK_FORMAT_VERSION, Playbook, PlaybookOrigin, PlaybookSeed, PlaybookStep,
};
use crate::model::{TargetKind, object_type};
use crate::paths::is_safe_relative_path;

use super::PlaybookError;

const FORBIDDEN_KEYS: &[&str] = &[
    "provenance",
    "resolved_findings",
    "resolved_finding",
    "include",
    "includes",
    "run_playbook",
    "playbooks",
    "delete",
    "unresolve",
];

/// Parse playbook YAML bytes. Returns a clear error; never panics to the caller.
pub fn parse_playbook_yaml(bytes: &[u8]) -> Result<Playbook, PlaybookError> {
    if bytes.len() as u64 > MAX_PLAYBOOK_BYTES {
        return Err(PlaybookError::new(format!(
            "playbook is {} bytes; maximum is {MAX_PLAYBOOK_BYTES}",
            bytes.len()
        )));
    }
    if bytes.contains(&0) {
        return Err(PlaybookError::new(
            "playbook YAML contains a NUL byte and was refused",
        ));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| {
        PlaybookError::new("playbook YAML is not valid UTF-8 (UTF-16 and binary files are refused)")
    })?;
    reject_unsafe_yaml(text)?;
    let value = parse_yaml_value(text)?;
    document_from_value(value)
}

fn parse_yaml_value(text: &str) -> Result<Value, PlaybookError> {
    let parsed = catch_unwind(AssertUnwindSafe(|| serde_yaml::from_str::<Value>(text)));
    match parsed {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(PlaybookError::new(format!(
            "invalid playbook YAML: {error}"
        ))),
        Err(_) => Err(PlaybookError::new(
            "playbook YAML could not be parsed safely (nested aliases or tags are refused)",
        )),
    }
}

fn reject_unsafe_yaml(text: &str) -> Result<(), PlaybookError> {
    let mut aliases = 0usize;
    let mut anchors = 0usize;
    for (index, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.contains("..") && looks_like_path_assignment(line) {
            return Err(PlaybookError::new(format!(
                "playbook YAML line {}: path traversal ('..') is refused",
                index + 1
            )));
        }
        if line.contains('\u{0000}') {
            return Err(PlaybookError::new("playbook YAML contains a NUL byte"));
        }
        if has_yaml_tag(line) {
            return Err(PlaybookError::new(format!(
                "playbook YAML line {}: custom YAML tags are refused",
                index + 1
            )));
        }
        if line.contains("<<:") || line.contains("<< :") {
            return Err(PlaybookError::new(format!(
                "playbook YAML line {}: merge keys are refused",
                index + 1
            )));
        }
        anchors += count_anchors(line);
        aliases += count_aliases(line);
        if anchors > 8 || aliases > 8 {
            return Err(PlaybookError::new(
                "playbook YAML has too many anchors or aliases (refused as a possible expansion bomb)",
            ));
        }
    }
    Ok(())
}

fn looks_like_path_assignment(line: &str) -> bool {
    let lowered = line.to_ascii_lowercase();
    lowered.contains("path:")
        || lowered.contains("file:")
        || lowered.contains("include:")
        || lowered.contains("playbook:")
}

fn has_yaml_tag(line: &str) -> bool {
    let Some((_, rest)) = line.split_once(':') else {
        return line.starts_with("!!") || (line.starts_with('!') && !line.starts_with("! "));
    };
    let rest = rest.trim();
    rest.starts_with("!!") || (rest.starts_with('!') && !rest.starts_with("! "))
}

fn count_anchors(line: &str) -> usize {
    line.split_whitespace()
        .filter(|token| token.starts_with('&') && token.len() > 1)
        .count()
}

fn count_aliases(line: &str) -> usize {
    line.split_whitespace()
        .filter(|token| token.starts_with('*') && token.len() > 1)
        .count()
}

fn document_from_value(value: Value) -> Result<Playbook, PlaybookError> {
    let Value::Mapping(map) = value else {
        return Err(PlaybookError::new(
            "playbook YAML must be a mapping (format, id, seed, steps)",
        ));
    };
    reject_forbidden_keys(&map, "")?;
    if let Some(format) = string_field(&map, "format") {
        if format != PLAYBOOK_FORMAT {
            return Err(PlaybookError::new(format!(
                "unknown playbook format {format:?}; expected {PLAYBOOK_FORMAT}"
            )));
        }
    } else {
        return Err(PlaybookError::new(format!(
            "playbook YAML needs format: {PLAYBOOK_FORMAT}"
        )));
    }
    let version = match field(&map, "format_version") {
        Some(Value::Number(number)) => number.as_u64().unwrap_or(u64::MAX),
        Some(Value::String(text)) => text.parse().unwrap_or(u64::MAX),
        None => {
            return Err(PlaybookError::new("playbook YAML needs format_version: 1"));
        }
        Some(_) => {
            return Err(PlaybookError::new(
                "playbook format_version must be a number",
            ));
        }
    };
    if version > u64::from(PLAYBOOK_FORMAT_VERSION) {
        return Err(PlaybookError::new(format!(
            "playbook format_version {version} is newer than {PLAYBOOK_FORMAT_VERSION}"
        )));
    }
    if version == 0 {
        return Err(PlaybookError::new(
            "playbook format_version 0 is invalid; use 1",
        ));
    }
    let id = required_id(&map)?;
    let name = string_field(&map, "name").unwrap_or_else(|| id.clone());
    let description = string_field(&map, "description").unwrap_or_default();
    let seed = parse_seed(field(&map, "seed"))?;
    let steps = parse_steps(field(&map, "steps"))?;
    Ok(Playbook {
        id,
        name,
        description,
        seed,
        steps,
        origin: PlaybookOrigin::File("yaml".into()),
    })
}

fn reject_forbidden_keys(map: &serde_yaml::Mapping, prefix: &str) -> Result<(), PlaybookError> {
    for key in map.keys() {
        let Some(name) = key.as_str() else {
            return Err(PlaybookError::new("playbook YAML keys must be strings"));
        };
        let lowered = name.to_ascii_lowercase();
        if FORBIDDEN_KEYS.contains(&lowered.as_str()) {
            let where_ = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}.{name}")
            };
            return Err(PlaybookError::new(format!(
                "playbook field {where_} is refused (playbooks cannot set provenance, delete findings, include files, or nest playbooks)"
            )));
        }
        if let Some(Value::Mapping(nested)) = map.get(key) {
            let next = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}.{name}")
            };
            reject_forbidden_keys(nested, &next)?;
        }
        if let Some(Value::Sequence(items)) = map.get(key) {
            for item in items {
                if let Value::Mapping(nested) = item {
                    reject_forbidden_keys(nested, prefix)?;
                }
            }
        }
        if let Some(Value::String(text)) = map.get(key)
            && (text.contains("..") || text.contains('\0'))
            && !is_safe_relative_path(text)
            && looks_like_path_assignment(&format!("{name}: {text}"))
        {
            return Err(PlaybookError::new(format!(
                "playbook field {name} contains an unsafe path"
            )));
        }
    }
    Ok(())
}

fn required_id(map: &serde_yaml::Mapping) -> Result<String, PlaybookError> {
    let Some(id) = string_field(map, "id") else {
        return Err(PlaybookError::new("playbook YAML needs an id"));
    };
    if id.len() > 64 {
        return Err(PlaybookError::new(
            "playbook id must be at most 64 characters",
        ));
    }
    let valid = id.chars().enumerate().all(|(index, ch)| {
        if index == 0 {
            ch.is_ascii_lowercase()
        } else {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-'
        }
    });
    if !valid {
        return Err(PlaybookError::new(format!(
            "playbook id {id:?} must be kebab-case (lowercase letters, digits, hyphen)"
        )));
    }
    Ok(id)
}

fn parse_seed(value: Option<&Value>) -> Result<PlaybookSeed, PlaybookError> {
    let Some(Value::Mapping(map)) = value else {
        return Err(PlaybookError::new(
            "playbook YAML needs seed.type (domain, ip, url, email, repository, file, username, or asn)",
        ));
    };
    let kind_raw = string_field(map, "type")
        .or_else(|| string_field(map, "kind"))
        .ok_or_else(|| PlaybookError::new("playbook seed needs type (for example type: domain)"))?;
    let Some(kind) = TargetKind::parse(&kind_raw) else {
        if object_type(&kind_raw).is_some() {
            return Err(PlaybookError::new(format!(
                "playbook seed type {kind_raw:?} is an object type that collectors cannot run against; use domain, ip, url, email, repository, file, username, or asn"
            )));
        }
        return Err(PlaybookError::new(format!(
            "unknown playbook seed type {kind_raw:?}"
        )));
    };
    Ok(PlaybookSeed { kind })
}

fn parse_steps(value: Option<&Value>) -> Result<Vec<PlaybookStep>, PlaybookError> {
    let Some(Value::Sequence(items)) = value else {
        return Err(PlaybookError::new(
            "playbook YAML needs a steps list (collect, search_around, run_tool, assert_finding_template, propose_resolve)",
        ));
    };
    if items.is_empty() {
        return Err(PlaybookError::new("playbook steps list is empty"));
    }
    if items.len() > MAX_PLAYBOOK_STEPS {
        return Err(PlaybookError::new(format!(
            "playbook has {} steps; maximum is {MAX_PLAYBOOK_STEPS}",
            items.len()
        )));
    }
    let mut steps = Vec::with_capacity(items.len());
    let mut collect_count = 0usize;
    let mut tool_count = 0usize;
    for (index, item) in items.iter().enumerate() {
        let step = parse_step(item, index)?;
        match &step {
            PlaybookStep::Collect { .. } => {
                collect_count += 1;
                if collect_count > MAX_COLLECT_STEPS {
                    return Err(PlaybookError::new(format!(
                        "playbook has more than {MAX_COLLECT_STEPS} collect steps"
                    )));
                }
            }
            PlaybookStep::RunTool { .. } => {
                tool_count += 1;
                if tool_count > MAX_TOOL_STEPS {
                    return Err(PlaybookError::new(format!(
                        "playbook has more than {MAX_TOOL_STEPS} run_tool steps"
                    )));
                }
            }
            _ => {}
        }
        steps.push(step);
    }
    Ok(steps)
}

fn parse_step(value: &Value, index: usize) -> Result<PlaybookStep, PlaybookError> {
    let Value::Mapping(map) = value else {
        return Err(PlaybookError::new(format!(
            "step {}: each step must be a mapping (for example `- collect: dns`)",
            index + 1
        )));
    };
    reject_forbidden_keys(map, &format!("steps[{}]", index + 1))?;
    let mut keys: Vec<&str> = map.keys().filter_map(Value::as_str).collect();
    keys.sort_unstable();
    if keys.is_empty() {
        return Err(PlaybookError::new(format!(
            "step {}: empty mapping",
            index + 1
        )));
    }
    let primary = keys[0];
    if keys.len() > 1 {
        return Err(PlaybookError::new(format!(
            "step {}: extra keys {} (each step is a single mapping)",
            index + 1,
            keys[1..].join(", ")
        )));
    }
    match primary {
        "collect" => parse_collect(field(map, "collect"), index),
        "run_tool" => parse_run_tool(field(map, "run_tool"), index),
        "search_around" => parse_search_around(field(map, "search_around"), index),
        "assert_finding_template" => parse_assert(field(map, "assert_finding_template"), index),
        "propose_resolve" => parse_propose_resolve(field(map, "propose_resolve"), index),
        other => Err(PlaybookError::new(format!(
            "step {}: unknown step type {other:?}; allowed: collect, search_around, run_tool, assert_finding_template, propose_resolve",
            index + 1
        ))),
    }
}

fn parse_collect(value: Option<&Value>, index: usize) -> Result<PlaybookStep, PlaybookError> {
    let collector = match value {
        Some(Value::String(text)) => text.trim().to_string(),
        Some(Value::Mapping(map)) => {
            reject_unknown_keys(map, &["collector", "id"], index, "collect")?;
            string_field(map, "collector")
                .or_else(|| string_field(map, "id"))
                .unwrap_or_default()
        }
        _ => String::new(),
    };
    if collector.is_empty() {
        return Err(PlaybookError::new(format!(
            "step {}: collect needs a collector id (dns, rdap, http, …)",
            index + 1
        )));
    }
    if !is_safe_id(&collector) {
        return Err(PlaybookError::new(format!(
            "step {}: collector id {collector:?} is not a safe identifier",
            index + 1
        )));
    }
    Ok(PlaybookStep::Collect { collector })
}

fn parse_run_tool(value: Option<&Value>, index: usize) -> Result<PlaybookStep, PlaybookError> {
    let tool = match value {
        Some(Value::String(text)) => text.trim().to_string(),
        Some(Value::Mapping(map)) => {
            reject_unknown_keys(map, &["id", "tool", "name"], index, "run_tool")?;
            string_field(map, "id")
                .or_else(|| string_field(map, "tool"))
                .or_else(|| string_field(map, "name"))
                .unwrap_or_default()
        }
        _ => String::new(),
    };
    if tool.is_empty() {
        return Err(PlaybookError::new(format!(
            "step {}: run_tool needs a tool id",
            index + 1
        )));
    }
    if !is_safe_id(&tool) {
        return Err(PlaybookError::new(format!(
            "step {}: tool id {tool:?} is not a safe identifier",
            index + 1
        )));
    }
    Ok(PlaybookStep::RunTool { tool })
}

fn parse_search_around(value: Option<&Value>, index: usize) -> Result<PlaybookStep, PlaybookError> {
    let (hops, link_types, entity_kinds) = match value {
        None | Some(Value::Null) | Some(Value::Bool(true)) => (2, Vec::new(), Vec::new()),
        Some(Value::Mapping(map)) => {
            reject_unknown_keys(
                map,
                &["hops", "link_types", "kinds", "entity_kinds"],
                index,
                "search_around",
            )?;
            let hops = integer_field(map, "hops").unwrap_or(2);
            if hops == 0 || hops > u64::from(MAX_SEARCH_HOPS) {
                return Err(PlaybookError::new(format!(
                    "step {}: search_around hops must be 1..={MAX_SEARCH_HOPS}",
                    index + 1
                )));
            }
            (
                hops as u32,
                string_list(map, "link_types"),
                string_list(map, "kinds")
                    .into_iter()
                    .chain(string_list(map, "entity_kinds"))
                    .collect(),
            )
        }
        Some(Value::Number(number)) => {
            let hops = number.as_u64().unwrap_or(0);
            if hops == 0 || hops > u64::from(MAX_SEARCH_HOPS) {
                return Err(PlaybookError::new(format!(
                    "step {}: search_around hops must be 1..={MAX_SEARCH_HOPS}",
                    index + 1
                )));
            }
            (hops as u32, Vec::new(), Vec::new())
        }
        _ => {
            return Err(PlaybookError::new(format!(
                "step {}: search_around must be a mapping or omitted",
                index + 1
            )));
        }
    };
    for name in link_types.iter().chain(entity_kinds.iter()) {
        if !is_safe_id(name) {
            return Err(PlaybookError::new(format!(
                "step {}: {name:?} is not a safe filter id",
                index + 1
            )));
        }
    }
    Ok(PlaybookStep::SearchAround {
        hops,
        link_types,
        entity_kinds,
    })
}

fn parse_assert(value: Option<&Value>, index: usize) -> Result<PlaybookStep, PlaybookError> {
    let Some(Value::Mapping(map)) = value else {
        return Err(PlaybookError::new(format!(
            "step {}: assert_finding_template needs title (and optional details)",
            index + 1
        )));
    };
    reject_unknown_keys(map, &["title", "details"], index, "assert_finding_template")?;
    let title = string_field(map, "title").unwrap_or_default();
    if title.trim().is_empty() {
        return Err(PlaybookError::new(format!(
            "step {}: assert_finding_template needs a title",
            index + 1
        )));
    }
    if title.len() > 500 {
        return Err(PlaybookError::new(format!(
            "step {}: finding template title is too long",
            index + 1
        )));
    }
    let details = string_field(map, "details").unwrap_or_default();
    Ok(PlaybookStep::AssertFindingTemplate { title, details })
}

fn parse_propose_resolve(
    value: Option<&Value>,
    index: usize,
) -> Result<PlaybookStep, PlaybookError> {
    let reason = match value {
        None | Some(Value::Null) | Some(Value::Bool(true)) | Some(Value::Mapping(_)) => {
            if let Some(Value::Mapping(map)) = value {
                reject_unknown_keys(map, &["reason"], index, "propose_resolve")?;
                string_field(map, "reason").unwrap_or_default()
            } else {
                String::new()
            }
        }
        Some(Value::String(text)) => text.clone(),
        _ => {
            return Err(PlaybookError::new(format!(
                "step {}: propose_resolve must be a mapping or omitted body",
                index + 1
            )));
        }
    };
    Ok(PlaybookStep::ProposeResolve { reason })
}

fn reject_unknown_keys(
    map: &serde_yaml::Mapping,
    allowed: &[&str],
    index: usize,
    step: &str,
) -> Result<(), PlaybookError> {
    for key in map.keys() {
        let Some(name) = key.as_str() else {
            return Err(PlaybookError::new(format!(
                "step {}: keys must be strings",
                index + 1
            )));
        };
        if !allowed.contains(&name) {
            return Err(PlaybookError::new(format!(
                "step {}: unknown key {name:?} on {step} (allowed: {})",
                index + 1,
                allowed.join(", ")
            )));
        }
    }
    Ok(())
}

fn field<'a>(map: &'a serde_yaml::Mapping, key: &str) -> Option<&'a Value> {
    map.get(Value::String(key.into()))
}

fn string_field(map: &serde_yaml::Mapping, key: &str) -> Option<String> {
    match field(map, key) {
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Some(Value::Number(number)) => Some(number.to_string()),
        _ => None,
    }
}

fn integer_field(map: &serde_yaml::Mapping, key: &str) -> Option<u64> {
    match field(map, key) {
        Some(Value::Number(number)) => number.as_u64(),
        Some(Value::String(text)) => text.trim().parse().ok(),
        _ => None,
    }
}

fn string_list(map: &serde_yaml::Mapping, key: &str) -> Vec<String> {
    match field(map, key) {
        Some(Value::Sequence(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect(),
        Some(Value::String(text)) => text
            .split(',')
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn is_safe_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playbook::bundled;

    #[test]
    fn bundled_domain_infra_parses() {
        let actual = bundled("domain-infra").expect("bundled");
        assert_eq!(actual.id, "domain-infra");
        assert_eq!(actual.seed.kind, TargetKind::Domain);
        assert!(
            actual.steps.iter().any(
                |step| matches!(step, PlaybookStep::Collect { collector } if collector == "dns")
            )
        );
        assert!(
            actual
                .steps
                .iter()
                .any(|step| matches!(step, PlaybookStep::AssertFindingTemplate { .. }))
        );
    }

    #[test]
    fn invalid_yaml_is_a_clear_error() {
        let actual = parse_playbook_yaml(b"[unterminated: : :").expect_err("invalid");
        assert!(
            actual.message.to_ascii_lowercase().contains("yaml")
                || actual.message.contains("mapping")
                || actual.message.contains("format"),
            "{}",
            actual.message
        );
    }

    #[test]
    fn missing_steps_is_a_clear_error() {
        let input = br#"
format: os-int-playbook
format_version: 1
id: empty-pack
seed:
  type: domain
"#;
        let actual = parse_playbook_yaml(input).expect_err("steps");
        assert!(actual.message.contains("steps"));
    }

    #[test]
    fn forbidden_provenance_key_is_refused() {
        let input = br#"
format: os-int-playbook
format_version: 1
id: sneaky
provenance: agent
seed:
  type: domain
steps:
  - collect: dns
"#;
        let actual = parse_playbook_yaml(input).expect_err("provenance");
        assert!(actual.message.contains("provenance"));
    }

    #[test]
    fn nested_playbook_step_is_refused() {
        let input = br#"
format: os-int-playbook
format_version: 1
id: nested
seed:
  type: domain
steps:
  - run_playbook: domain-infra
"#;
        let actual = parse_playbook_yaml(input).expect_err("nested");
        assert!(actual.message.contains("run_playbook") || actual.message.contains("nest"));
    }

    #[test]
    fn path_traversal_in_yaml_is_refused() {
        let input = br#"
format: os-int-playbook
format_version: 1
id: traverse
include: ../../etc/passwd
seed:
  type: domain
steps:
  - collect: dns
"#;
        let actual = parse_playbook_yaml(input).expect_err("include");
        assert!(actual.message.contains("refused") || actual.message.contains("traversal"));
    }

    #[test]
    fn yaml_bomb_aliases_are_refused() {
        let input = b"a: &a [1, 2]\nb: [*a, *a, *a, *a, *a, *a, *a, *a, *a]\n";
        let actual = parse_playbook_yaml(input).expect_err("bomb");
        assert!(
            actual.message.contains("alias")
                || actual.message.contains("invalid playbook YAML")
                || actual.message.contains("format")
        );
    }

    #[test]
    fn extra_keys_on_known_steps_are_refused() {
        let sibling = br#"
format: os-int-playbook
format_version: 1
id: extra-sibling
seed:
  type: domain
steps:
  - collect: dns
    extra: true
"#;
        let actual = parse_playbook_yaml(sibling).expect_err("sibling");
        assert!(
            actual.message.contains("extra") || actual.message.contains("unknown key"),
            "{}",
            actual.message
        );
        let nested = br#"
format: os-int-playbook
format_version: 1
id: extra-nested
seed:
  type: domain
steps:
  - collect:
      collector: dns
      unexpected: 1
"#;
        let actual = parse_playbook_yaml(nested).expect_err("nested");
        assert!(
            actual.message.contains("unknown key") || actual.message.contains("unexpected"),
            "{}",
            actual.message
        );
    }

    #[test]
    fn unknown_step_is_a_clear_error() {
        let input = br#"
format: os-int-playbook
format_version: 1
id: weird
seed:
  type: domain
steps:
  - explode: true
"#;
        let actual = parse_playbook_yaml(input).expect_err("unknown");
        assert!(actual.message.contains("unknown step"));
    }

    #[test]
    fn resolved_findings_key_is_refused() {
        let input = br#"
format: os-int-playbook
format_version: 1
id: wipe
seed:
  type: domain
steps:
  - collect: dns
resolved_findings:
  - anything
"#;
        let actual = parse_playbook_yaml(input).expect_err("resolved");
        assert!(actual.message.contains("resolved_findings"));
    }

    #[test]
    fn too_many_steps_are_capped() {
        let mut yaml = String::from(
            "format: os-int-playbook\nformat_version: 1\nid: too-long\nseed:\n  type: domain\nsteps:\n",
        );
        for _ in 0..33 {
            yaml.push_str("  - collect: dns\n");
        }
        let actual = parse_playbook_yaml(yaml.as_bytes()).expect_err("cap");
        assert!(actual.message.contains("steps"));
    }

    #[test]
    fn binary_is_refused_without_panic() {
        let actual = parse_playbook_yaml(&[0xff, 0xfe, 0x00, 0x00]).expect_err("binary");
        assert!(actual.message.contains("UTF-8") || actual.message.contains("NUL"));
    }

    #[test]
    fn yaml_tags_and_merge_keys_are_refused() {
        let tagged = br#"
format: os-int-playbook
format_version: 1
id: tagged
seed:
  type: !!str domain
steps:
  - collect: dns
"#;
        let actual = parse_playbook_yaml(tagged).expect_err("tag");
        assert!(actual.message.contains("tag"));
        let merged = br#"
format: os-int-playbook
format_version: 1
id: merged
seed:
  <<: { type: domain }
steps:
  - collect: dns
"#;
        let actual = parse_playbook_yaml(merged).expect_err("merge");
        assert!(actual.message.contains("merge"));
    }

    #[test]
    fn newer_format_version_is_refused() {
        let input = br#"
format: os-int-playbook
format_version: 2
id: future
seed:
  type: domain
steps:
  - collect: dns
"#;
        let actual = parse_playbook_yaml(input).expect_err("version");
        assert!(actual.message.contains("format_version") || actual.message.contains("1"));
    }
}
