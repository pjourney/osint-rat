//! Bulk CSV / JSON observable import with column mapping.

use serde_json::Value;

use super::{IngestError, IngestReport, MAX_OBJECTS};
use crate::files::sha256_hex;
use crate::model::{EntityKind, EvidenceBatch, NewSource, SourceKind, relation};
use crate::sanitize::sanitize_text;

/// How CSV columns or JSON keys map onto entity kind/value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObservableMapping {
    /// Header name or 0-based column index for the kind (`domain`, `ip`, …).
    pub kind_column: Option<String>,
    /// Header name or 0-based column index for the observable value.
    pub value_column: Option<String>,
    /// Used when the file has a single value column (or `--kind` was passed).
    pub default_kind: Option<EntityKind>,
}

impl ObservableMapping {
    /// Parses `kind=type,value=indicator` or `0=kind,1=value`.
    pub fn parse(spec: &str) -> Result<Self, IngestError> {
        let mut mapping = Self::default();
        for part in spec.split([',', ';']) {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let Some((key, value)) = part.split_once('=') else {
                return Err(IngestError::new(format!(
                    "invalid mapping {part:?}; expected kind=COLUMN,value=COLUMN"
                )));
            };
            match key.trim().to_ascii_lowercase().as_str() {
                "kind" | "type" | "k" => mapping.kind_column = Some(value.trim().to_string()),
                "value" | "observable" | "indicator" | "v" => {
                    mapping.value_column = Some(value.trim().to_string())
                }
                "default" | "default_kind" => {
                    mapping.default_kind = Some(EntityKind::parse(value.trim()));
                }
                other => {
                    return Err(IngestError::new(format!(
                        "unknown mapping key {other:?} (use kind, value, default)"
                    )));
                }
            }
        }
        Ok(mapping)
    }
}

/// Parses CSV text. The first row is a header when it looks like one.
pub fn parse_csv_observables(
    text: &str,
    mapping: &ObservableMapping,
    origin: &str,
) -> Result<IngestReport, IngestError> {
    let rows = parse_csv(text)?;
    if rows.is_empty() {
        return Err(IngestError::new("CSV file has no rows"));
    }
    let (header, data_rows) = split_header(&rows);
    let kind_idx = column_index(
        mapping.kind_column.as_deref(),
        header.as_ref(),
        &["kind", "type"],
    );
    let value_idx = column_index(
        mapping.value_column.as_deref(),
        header.as_ref(),
        &["value", "observable", "indicator", "name"],
    );
    ingest_rows(
        data_rows,
        &ObservableRows {
            kind_idx,
            value_idx,
            default_kind: mapping.default_kind.clone(),
            origin,
            collector: "csv",
            source_kind: SourceKind::Document,
        },
        text,
    )
}

/// Parses a JSON array of observables, `{ "observables": [...] }`, or an object map.
pub fn parse_json_observables(
    text: &str,
    mapping: &ObservableMapping,
    origin: &str,
) -> Result<IngestReport, IngestError> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| IngestError::new(format!("observable JSON is invalid: {error}")))?;
    let items: Vec<&Value> = if let Some(array) = value.as_array() {
        array.iter().collect()
    } else if let Some(array) = value.get("observables").and_then(Value::as_array) {
        array.iter().collect()
    } else if let Some(array) = value.get("data").and_then(Value::as_array) {
        array.iter().collect()
    } else if value.is_object() {
        vec![&value]
    } else {
        return Err(IngestError::new(
            "observable JSON must be an array or an object with `observables`",
        ));
    };
    let mut rows = Vec::new();
    for item in items {
        if let Some(text) = item.as_str() {
            rows.push(vec![text.to_string()]);
            continue;
        }
        let kind = mapping
            .kind_column
            .as_deref()
            .and_then(|column| item.get(column))
            .or_else(|| item.get("kind"))
            .or_else(|| item.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let value = mapping
            .value_column
            .as_deref()
            .and_then(|column| item.get(column))
            .or_else(|| item.get("value"))
            .or_else(|| item.get("observable"))
            .or_else(|| item.get("indicator"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        rows.push(vec![kind, value]);
    }
    ingest_rows(
        &rows,
        &ObservableRows {
            kind_idx: Some(0),
            value_idx: Some(1),
            default_kind: mapping.default_kind.clone(),
            origin,
            collector: "json",
            source_kind: SourceKind::Document,
        },
        text,
    )
}

struct ObservableRows<'a> {
    kind_idx: Option<usize>,
    value_idx: Option<usize>,
    default_kind: Option<EntityKind>,
    origin: &'a str,
    collector: &'a str,
    source_kind: SourceKind,
}

fn ingest_rows(
    rows: &[Vec<String>],
    spec: &ObservableRows<'_>,
    raw: &str,
) -> Result<IngestReport, IngestError> {
    let mut report = IngestReport::new(EvidenceBatch::new());
    let mut source = NewSource::external(
        spec.source_kind.clone(),
        &format!("ingest:{}", spec.collector),
        format!("Observable import from {}", spec.origin),
    );
    source.content = sanitize_text(&truncate(raw, 32 * 1024)).into_owned();
    source.content_hash = Some(sha256_hex(raw.as_bytes()));
    source.fields.push("origin", spec.origin);
    let key = report.batch.add_source(source);
    for (index, row) in rows.iter().enumerate() {
        if index >= MAX_OBJECTS {
            report.skipped += rows.len() - MAX_OBJECTS;
            report
                .warnings
                .push(format!("truncated observables at {MAX_OBJECTS}"));
            break;
        }
        if row.iter().all(|cell| cell.trim().is_empty()) {
            continue;
        }
        let value = spec
            .value_idx
            .and_then(|idx| row.get(idx))
            .map(String::as_str)
            .or_else(|| row.last().map(String::as_str))
            .unwrap_or("")
            .trim();
        if value.is_empty() {
            report.skipped += 1;
            report.warnings.push(format!("row {index}: empty value"));
            continue;
        }
        if value.len() > 2048 {
            report.skipped += 1;
            report
                .warnings
                .push(format!("row {index}: value longer than 2048 characters"));
            continue;
        }
        let kind_text = spec
            .kind_idx
            .and_then(|idx| row.get(idx))
            .map(String::as_str)
            .unwrap_or("");
        let kind = if kind_text.trim().is_empty() {
            spec.default_kind
                .clone()
                .unwrap_or_else(|| guess_kind(value))
        } else {
            EntityKind::parse(kind_text)
        };
        let entity = report.batch.entity(kind.clone(), value, Some(key));
        if kind == EntityKind::Email
            && let Some((_, domain)) = entity.value.rsplit_once('@')
        {
            let domain_key = report.batch.entity(EntityKind::Domain, domain, Some(key));
            report
                .batch
                .relate(&entity, relation::HOSTED_ON, &domain_key, Some(key));
        }
    }
    if report.batch.entities.is_empty() {
        return Err(IngestError::new("no observables could be imported"));
    }
    report.batch.summary = format!(
        "imported {} observable(s) from {}",
        report.batch.entities.len(),
        spec.origin
    );
    Ok(report)
}

fn guess_kind(value: &str) -> EntityKind {
    if crate::model::Target::guess(value)
        .map(|target| target.kind())
        .ok()
        == Some(crate::model::TargetKind::Ip)
        || value.parse::<std::net::IpAddr>().is_ok()
    {
        return EntityKind::Ip;
    }
    if value.contains('@') {
        return EntityKind::Email;
    }
    if value.contains("://") {
        return EntityKind::Url;
    }
    if value.to_ascii_uppercase().starts_with("AS")
        && value.chars().skip(2).all(|c| c.is_ascii_digit())
    {
        return EntityKind::Asn;
    }
    EntityKind::Domain
}

fn split_header(rows: &[Vec<String>]) -> (Option<Vec<String>>, &[Vec<String>]) {
    let Some(first) = rows.first() else {
        return (None, rows);
    };
    let looks_like_header = first.iter().any(|cell| {
        matches!(
            cell.trim().to_ascii_lowercase().as_str(),
            "kind" | "type" | "value" | "observable" | "indicator" | "name"
        )
    });
    if looks_like_header {
        (Some(first.clone()), &rows[1..])
    } else {
        (None, rows)
    }
}

fn column_index(
    named: Option<&str>,
    header: Option<&Vec<String>>,
    aliases: &[&str],
) -> Option<usize> {
    if let Some(named) = named {
        if let Ok(index) = named.parse::<usize>() {
            return Some(index);
        }
        if let Some(header) = header {
            let wanted = named.trim().to_ascii_lowercase();
            return header
                .iter()
                .position(|cell| cell.trim().eq_ignore_ascii_case(&wanted));
        }
        return None;
    }
    header.and_then(|header| {
        header.iter().position(|cell| {
            aliases
                .iter()
                .any(|alias| cell.trim().eq_ignore_ascii_case(alias))
        })
    })
}

/// Minimal RFC 4180-ish CSV parser. Quoted fields may contain commas and doubled quotes.
fn parse_csv(text: &str) -> Result<Vec<Vec<String>>, IngestError> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut chars = text.chars().peekable();
    let mut in_quotes = false;
    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(ch);
            }
            continue;
        }
        match ch {
            '"' if field.is_empty() => in_quotes = true,
            ',' => {
                row.push(std::mem::take(&mut field));
            }
            '\n' => {
                row.push(std::mem::take(&mut field));
                if row.iter().any(|cell| !cell.is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            '\r' => {}
            other => field.push(other),
        }
        if rows.len() > MAX_OBJECTS + 2 {
            break;
        }
    }
    if in_quotes {
        return Err(IngestError::new("CSV has an unterminated quoted field"));
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
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

    #[test]
    fn csv_with_header_and_mapping() {
        let input = "kind,value\ndomain,Example.COM\nip,192.0.2.9\nemail,a@example.com\n";
        let actual =
            parse_csv_observables(input, &ObservableMapping::default(), "obs.csv").expect("parse");
        assert!(
            actual
                .batch
                .entities
                .iter()
                .any(|entity| entity.key.value == "example.com")
        );
        assert!(
            actual
                .batch
                .relationships
                .iter()
                .any(|relationship| relationship.kind == relation::HOSTED_ON)
        );
        assert_eq!(
            actual.batch.sources[0].provenance,
            crate::model::Provenance::External
        );
    }

    #[test]
    fn csv_quoted_commas_and_default_kind() {
        let input = "\"www.example.com,cdn\"\n";
        let mapping = ObservableMapping {
            default_kind: Some(EntityKind::Hostname),
            ..ObservableMapping::default()
        };
        // A quoted field containing a comma is one value.
        let rows = parse_csv("value\n\"a,b\"\n").expect("csv");
        assert_eq!(rows[1], vec!["a,b".to_string()]);
        let actual = parse_csv_observables("host.example.com\n", &mapping, "x.csv").expect("rows");
        assert_eq!(actual.batch.entities[0].key.kind, EntityKind::Hostname);
        assert!(input.contains("www.example.com"));
    }

    #[test]
    fn json_array_of_objects() {
        let input =
            r#"[{"kind":"url","value":"https://example.com/a"},{"type":"ip","value":"192.0.2.4"}]"#;
        let actual =
            parse_json_observables(input, &ObservableMapping::default(), "x.json").expect("parse");
        assert_eq!(actual.batch.entities.len(), 2);
    }

    #[test]
    fn mapping_spec_parses() {
        let actual = ObservableMapping::parse("kind=type,value=indicator").expect("map");
        assert_eq!(actual.kind_column.as_deref(), Some("type"));
        assert_eq!(actual.value_column.as_deref(), Some("indicator"));
        assert!(ObservableMapping::parse("nope").is_err());
    }

    #[test]
    fn unterminated_quote_is_an_error() {
        assert!(parse_csv("\"hello").is_err());
    }
}
