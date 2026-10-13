//! Certificate transparency observation via the public crt.sh search.
//!
//! One HTTPS request asks crt.sh which certificates were logged for a name. The target itself
//! is not scanned. Results are public log data: names, issuers, and validity dates.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use serde::Deserialize;
use url::Url;

use super::net::{FetchOptions, fetch};
use super::{CollectResult, CollectionOutput, Collector, CollectorContext};
use crate::files::{StoredBlob, sha256_hex};
use crate::model::target::normalize_hostname;
use crate::model::{
    EntityKey, EntityKind, EvidenceBatch, Fields, NewSource, SourceKind, Target, TargetKind,
    TimelineKind, relation,
};
use crate::sanitize::sanitize_line;

const MAX_HOSTNAMES: usize = 150;
const MAX_CERTIFICATES: usize = 40;
const MAX_TIMELINE_EVENTS: usize = 25;

pub struct CtCollector;

impl Collector for CtCollector {
    fn id(&self) -> &'static str {
        "ct"
    }

    fn label(&self) -> &'static str {
        "CT"
    }

    fn description(&self) -> &'static str {
        "Names, issuers, and validity dates from public certificate transparency logs (crt.sh)"
    }

    fn accepts(&self, kind: TargetKind) -> bool {
        matches!(kind, TargetKind::Domain | TargetKind::Email)
    }

    fn collect<'a>(
        &'a self,
        target: &'a Target,
        context: &'a CollectorContext,
    ) -> BoxFuture<'a, CollectResult> {
        Box::pin(collect(target, context))
    }
}

/// One logged certificate reduced to the fields os-int keeps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CtCertificate {
    pub serial: String,
    pub names: Vec<String>,
    pub wildcard_names: Vec<String>,
    pub issuer_organization: Option<String>,
    pub not_before: Option<DateTime<Utc>>,
    pub not_after: Option<DateTime<Utc>>,
}

/// Parsed crt.sh results for one domain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CtObservation {
    pub domain: String,
    pub certificates: Vec<CtCertificate>,
    pub hostnames_truncated: bool,
}

#[derive(Deserialize)]
struct CtRow {
    issuer_name: Option<String>,
    name_value: Option<String>,
    not_before: Option<String>,
    not_after: Option<String>,
    serial_number: Option<String>,
    id: Option<i64>,
}

async fn collect(target: &Target, context: &CollectorContext) -> CollectResult {
    let domain = match target {
        Target::Domain(domain) => domain.clone(),
        Target::Email(_) => target.hostname().ok_or("the email address has no domain")?,
        _ => {
            return Err(format!(
                "the certificate transparency collector does not accept {}",
                target.kind()
            ));
        }
    };
    let url = Url::parse_with_params(
        "https://crt.sh/",
        &[("q", format!("%.{domain}")), ("output", "json".to_string())],
    )
    .map_err(|error| format!("could not build the crt.sh query: {error}"))?;
    context.job.progress(format!(
        "Querying certificate transparency logs for {domain}"
    ));
    context.job.command("GET", &[url.to_string()]);
    let options = FetchOptions {
        max_body_bytes: context.net.settings.max_body_bytes,
        accept: Some("application/json"),
    };
    let response = fetch(&context.net, &url, &options, &context.job.cancel)
        .await
        .map_err(|error| format!("certificate transparency lookup failed: {error}"))?;
    if response.body_truncated {
        return Err(
            "certificate transparency response exceeded network.max_body_bytes; raise the limit or collect a narrower name"
                .into(),
        );
    }
    if !response.is_success() {
        return Err(format!(
            "certificate transparency lookup failed: HTTP {}",
            response.status
        ));
    }
    let body = String::from_utf8_lossy(&response.body);
    let observation = parse_certificates(&domain, &body)?;
    let snapshot = context
        .files
        .store_snapshot(&response.body, "json")
        .map_err(|error| format!("could not store certificate transparency snapshot: {error}"))?;
    Ok(CollectionOutput::new(build_batch(
        target,
        &observation,
        url.as_str(),
        &snapshot,
    )))
}

/// Parses a crt.sh JSON array. Names outside the queried domain are ignored.
pub fn parse_certificates(domain: &str, body: &str) -> Result<CtObservation, String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Ok(CtObservation {
            domain: domain.to_string(),
            certificates: Vec::new(),
            hostnames_truncated: false,
        });
    }
    let rows: Vec<CtRow> = serde_json::from_str(trimmed).map_err(|_| {
        "certificate transparency response was not a JSON array; crt.sh may be unavailable"
            .to_string()
    })?;
    let mut certificates = Vec::new();
    let mut seen = BTreeSet::new();
    for row in rows {
        let Some(certificate) = certificate_from(domain, row) else {
            continue;
        };
        let identity = format!(
            "{}|{}|{}",
            certificate.serial,
            certificate.names.join(","),
            certificate.wildcard_names.join(",")
        );
        if seen.insert(identity) {
            certificates.push(certificate);
        }
    }
    certificates.sort_by_key(|certificate| std::cmp::Reverse(certificate.not_before));
    let mut hostnames = BTreeSet::new();
    let mut truncated = false;
    for certificate in &certificates {
        for name in &certificate.names {
            if hostnames.len() >= MAX_HOSTNAMES {
                truncated = true;
                break;
            }
            hostnames.insert(name.clone());
        }
    }
    if truncated {
        for certificate in &mut certificates {
            certificate.names.retain(|name| hostnames.contains(name));
        }
    }
    if certificates.len() > MAX_CERTIFICATES {
        certificates.truncate(MAX_CERTIFICATES);
    }
    Ok(CtObservation {
        domain: domain.to_string(),
        certificates,
        hostnames_truncated: truncated,
    })
}

fn certificate_from(domain: &str, row: CtRow) -> Option<CtCertificate> {
    let mut names = Vec::new();
    let mut wildcard_names = Vec::new();
    for raw in row.name_value.unwrap_or_default().split(['\n', ',']) {
        let cleaned = sanitize_line(raw)
            .trim()
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if cleaned.is_empty() {
            continue;
        }
        if let Some(suffix) = cleaned.strip_prefix("*.") {
            if name_in_domain(suffix, domain) {
                wildcard_names.push(cleaned);
            }
            continue;
        }
        if name_in_domain(&cleaned, domain)
            && normalize_hostname(&cleaned).is_ok()
            && !names.iter().any(|existing| existing == &cleaned)
        {
            names.push(cleaned);
        }
    }
    if names.is_empty() && wildcard_names.is_empty() {
        return None;
    }
    let serial = row
        .serial_number
        .map(|value| sanitize_line(&value).into_owned())
        .filter(|value| !value.is_empty())
        .or_else(|| row.id.map(|id| format!("ct-{id}")))?;
    Some(CtCertificate {
        serial,
        names,
        wildcard_names,
        issuer_organization: row.issuer_name.as_deref().and_then(issuer_organization),
        not_before: row.not_before.as_deref().and_then(parse_ct_time),
        not_after: row.not_after.as_deref().and_then(parse_ct_time),
    })
}

fn name_in_domain(name: &str, domain: &str) -> bool {
    name == domain || name.ends_with(&format!(".{domain}"))
}

fn issuer_organization(issuer: &str) -> Option<String> {
    let organization = issuer.split(',').find_map(|part| {
        let part = part.trim();
        part.strip_prefix("O=")
            .or_else(|| part.strip_prefix("o="))
            .map(str::trim)
    })?;
    let cleaned = sanitize_line(organization).into_owned();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn parse_ct_time(value: &str) -> Option<DateTime<Utc>> {
    let trimmed = value.trim();
    if let Ok(parsed) = DateTime::parse_from_rfc3339(trimmed) {
        return Some(parsed.with_timezone(&Utc));
    }
    let stamped = if trimmed.ends_with('Z') {
        trimmed.to_string()
    } else {
        format!("{trimmed}Z")
    };
    DateTime::parse_from_rfc3339(&stamped)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

/// Builds the evidence batch for a certificate transparency observation.
pub fn build_batch(
    target: &Target,
    observation: &CtObservation,
    query_url: &str,
    snapshot: &StoredBlob,
) -> EvidenceBatch {
    let mut batch = EvidenceBatch::new();
    let (content, fields) = describe(observation);
    let mut source = NewSource::collected(
        SourceKind::Ct,
        "ct",
        format!("Certificate transparency for {}", observation.domain),
    );
    source.url = Some(query_url.to_string());
    source.content_hash = Some(stable_hash(observation));
    source.snapshot_path = Some(snapshot.relative_path.clone());
    source.content = content;
    source.fields = fields;
    source
        .fields
        .push("snapshot_sha256", snapshot.sha256.clone());
    let key = batch.add_source(source);
    let domain_key = batch.entity(EntityKind::Domain, &observation.domain, Some(key));
    if let Target::Email(email) = target {
        let email_key = batch.entity(EntityKind::Email, email, Some(key));
        batch.relate(&email_key, relation::HOSTED_ON, &domain_key, Some(key));
    }
    let mut events = 0;
    for certificate in &observation.certificates {
        let certificate_key = batch.entity(EntityKind::Certificate, &certificate.serial, Some(key));
        batch.label(
            &certificate_key,
            certificate
                .names
                .first()
                .cloned()
                .unwrap_or_else(|| certificate.serial.clone()),
        );
        if let Some(organization) = &certificate.issuer_organization {
            let issuer = batch.entity(EntityKind::Organization, organization, Some(key));
            batch.relate(&certificate_key, relation::ISSUED_BY, &issuer, Some(key));
        }
        for name in &certificate.names {
            let name_key = if name == &observation.domain {
                domain_key.clone()
            } else {
                let hostname = EntityKey::new(EntityKind::Hostname, name);
                batch.relate(&hostname, relation::SUBDOMAIN_OF, &domain_key, Some(key));
                hostname
            };
            batch.relate(&certificate_key, relation::COVERS, &name_key, Some(key));
        }
        if events < MAX_TIMELINE_EVENTS
            && let Some(issued) = certificate.not_before
        {
            let label = certificate
                .names
                .first()
                .map(String::as_str)
                .unwrap_or(&observation.domain);
            batch.event(
                issued,
                TimelineKind::CertificateIssued,
                format!("Certificate logged for {label}"),
                Some(key),
                Some(certificate_key),
            );
            events += 1;
        }
    }
    let name_count = observation
        .certificates
        .iter()
        .flat_map(|certificate| certificate.names.iter())
        .collect::<BTreeSet<_>>()
        .len();
    batch.summary = format!(
        "{} logged certificate(s), {} name(s){}",
        observation.certificates.len(),
        name_count,
        if observation.hostnames_truncated {
            " (name list truncated)"
        } else {
            ""
        }
    );
    batch
}

fn describe(observation: &CtObservation) -> (String, Fields) {
    let mut lines = vec![format!(
        "Certificate transparency search for %.{} via https://crt.sh/",
        observation.domain
    )];
    if observation.hostnames_truncated {
        lines.push(format!(
            "Name list truncated to the first {MAX_HOSTNAMES} in-scope hostnames."
        ));
    }
    let mut fields = Fields::new();
    fields.push("certificates", observation.certificates.len().to_string());
    let mut wildcard_count = 0;
    for certificate in &observation.certificates {
        wildcard_count += certificate.wildcard_names.len();
        lines.push(String::new());
        lines.push(format!("Serial: {}", certificate.serial));
        if let Some(organization) = &certificate.issuer_organization {
            lines.push(format!("Issuer organization: {organization}"));
        }
        lines.push(format!(
            "Validity: {} → {}",
            format_time(&certificate.not_before),
            format_time(&certificate.not_after)
        ));
        if !certificate.names.is_empty() {
            lines.push(format!("Names: {}", certificate.names.join(", ")));
        }
        if !certificate.wildcard_names.is_empty() {
            lines.push(format!(
                "Wildcard names: {}",
                certificate.wildcard_names.join(", ")
            ));
        }
    }
    fields.push("wildcard_names", wildcard_count.to_string());
    (lines.join("\n"), fields)
}

fn format_time(value: &Option<DateTime<Utc>>) -> String {
    value
        .map(|time| time.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn stable_hash(observation: &CtObservation) -> String {
    let mut lines = Vec::new();
    for certificate in &observation.certificates {
        lines.push(format!(
            "{} {} {} {}",
            certificate.serial,
            certificate.names.join(","),
            certificate.wildcard_names.join(","),
            format_time(&certificate.not_before)
        ));
    }
    lines.sort();
    sha256_hex(lines.join("\n").as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[
      {"id": 1, "issuer_name": "C=US, O=Example CA, CN=R3", "name_value": "example.com\nwww.example.com\n*.example.com\nother.example.net", "not_before": "2024-01-01T00:00:00", "not_after": "2025-01-01T00:00:00", "serial_number": "AA:BB"},
      {"id": 2, "issuer_name": "C=US, O=Example CA, CN=R3", "name_value": "api.example.com", "not_before": "2024-06-01T00:00:00", "not_after": "2025-06-01T00:00:00", "serial_number": "CC:DD"},
      {"id": 1, "issuer_name": "C=US, O=Example CA, CN=R3", "name_value": "example.com\nwww.example.com\n*.example.com\nother.example.net", "not_before": "2024-01-01T00:00:00", "not_after": "2025-01-01T00:00:00", "serial_number": "AA:BB"}
    ]"#;

    fn snapshot() -> StoredBlob {
        StoredBlob {
            relative_path: "snapshots/ab/abc.json".into(),
            sha256: "abc".into(),
            size_bytes: 3,
        }
    }

    #[test]
    fn names_outside_the_domain_are_dropped_and_duplicates_collapse() {
        let actual = parse_certificates("example.com", SAMPLE).expect("parses");
        assert_eq!(actual.certificates.len(), 2);
        assert_eq!(actual.certificates[0].serial, "CC:DD");
        assert!(
            actual.certificates[1]
                .names
                .contains(&"www.example.com".to_string())
        );
        assert!(
            actual.certificates[1]
                .wildcard_names
                .contains(&"*.example.com".to_string())
        );
        assert!(
            !actual.certificates[1]
                .names
                .iter()
                .any(|name| name.contains("example.net"))
        );
    }

    #[test]
    fn batch_links_names_issuers_and_issue_dates() {
        let observation = parse_certificates("example.com", SAMPLE).expect("parses");
        let actual = build_batch(
            &Target::Domain("example.com".into()),
            &observation,
            "https://crt.sh/?q=%.example.com&output=json",
            &snapshot(),
        );
        assert!(
            actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "subdomain_of"
                    && relationship.from.value == "api.example.com")
        );
        assert!(
            actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "covers"
                    && relationship.to.value == "www.example.com")
        );
        assert!(
            actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "issued_by"
                    && relationship.to.value == "Example CA")
        );
        assert_eq!(actual.timeline.len(), 2);
        assert!(actual.sources[0].content.contains("Wildcard names"));
        assert!(!actual.summary.is_empty());
    }

    #[test]
    fn html_error_pages_are_rejected() {
        let actual = parse_certificates("example.com", "<html>unavailable</html>");
        assert!(actual.is_err());
    }

    #[test]
    fn an_empty_log_is_a_successful_observation() {
        let actual = parse_certificates("example.com", "[]").expect("parses");
        assert!(actual.certificates.is_empty());
    }
}
