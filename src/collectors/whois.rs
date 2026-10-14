//! Structured WHOIS fallback when RDAP is missing or thin.
//!
//! Runs the `whois` program through `exec/` (no shell) and parses registrar, nameservers,
//! dates, and registrant fields into the same graph verbs RDAP uses.

use std::time::Duration;

use crate::exec::{CommandSpec, ExecError, run};
use crate::files::sha256_hex;
use crate::jobs::JobContext;
use crate::model::{
    EntityKind, EvidenceBatch, Fields, NewSource, SourceKind, Target, TimelineKind, relation,
};
use crate::sanitize::{sanitize_line, sanitize_text};
use crate::tools::ToolRegistry;

use super::rdap::{RdapContact, RdapSummary, is_redacted, parse_date};
use super::{CollectResult, CollectionOutput};

const WHOIS_TIMEOUT: Duration = Duration::from_secs(30);

/// Parsed WHOIS text, mapped onto the RDAP summary shape so relationship verbs stay shared.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WhoisRecord {
    pub summary: RdapSummary,
    pub raw: String,
}

/// True when RDAP produced no nameservers, no usable contacts, and no events.
pub fn is_thin_rdap(summary: &RdapSummary) -> bool {
    summary.nameservers.is_empty()
        && summary.events.is_empty()
        && summary
            .contacts
            .iter()
            .all(|contact| contact.display_name().is_none())
}

/// True when an RDAP batch has no registration graph edges.
pub fn is_thin_rdap_batch(batch: &EvidenceBatch) -> bool {
    !batch.relationships.iter().any(|relationship| {
        matches!(
            relationship.kind.as_str(),
            relation::USES_NAMESERVER
                | relation::REGISTERED_WITH
                | relation::REGISTERED_TO
                | relation::OPERATED_BY
        )
    })
}

/// Runs `whois` when it is installed and returns a collection output.
pub async fn collect_whois(
    target: &Target,
    tools: &ToolRegistry,
    job: &JobContext,
) -> CollectResult {
    let program = tools.path_of("whois").ok_or_else(|| {
        "whois is not installed; RDAP was thin and there is no WHOIS fallback".to_string()
    })?;
    job.progress(format!("WHOIS fallback for {}", target.value()));
    let mut spec = CommandSpec::new(
        program.display().to_string(),
        vec![target.value()],
        WHOIS_TIMEOUT,
    );
    spec.max_output_bytes = crate::exec::DEFAULT_MAX_OUTPUT_BYTES;
    job.command(&spec.program, &spec.args);
    let outcome = match run(&spec, &job.cancel, |line| {
        job.log(sanitize_line(&line.text).into_owned());
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(ExecError::Cancelled) => return Err("cancelled".into()),
        Err(error) => return Err(error.to_string()),
    };
    if outcome.stdout.trim().is_empty() {
        return Err(format!(
            "whois produced no output (exit {})",
            outcome.exit_code.unwrap_or(-1)
        ));
    }
    let stdout = sanitize_text(&outcome.stdout).into_owned();
    let record = parse_whois(&stdout);
    if record.summary.nameservers.is_empty()
        && record.summary.contacts.is_empty()
        && record.summary.events.is_empty()
    {
        return Err("whois output did not contain registrar, nameserver, or date fields".into());
    }
    Ok(CollectionOutput::new(build_batch(target, &record)))
}

/// Builds an evidence batch from structured WHOIS fields. Pure and unit-tested.
pub fn build_batch(target: &Target, record: &WhoisRecord) -> EvidenceBatch {
    let mut batch = EvidenceBatch::new();
    let mut source = NewSource::collected(
        SourceKind::Rdap,
        "whois",
        format!("WHOIS for {}", target.value()),
    );
    source.content = record.raw.clone();
    source.content_hash = Some(sha256_hex(record.raw.as_bytes()));
    source.fields = whois_fields(&record.summary);
    source.fields.push("fallback", "rdap-thin");
    let key = batch.add_source(source);
    let subject = batch.entity(target.entity_kind(), &target.value(), Some(key));
    for nameserver in &record.summary.nameservers {
        let nameserver_key = crate::model::EntityKey::new(EntityKind::Hostname, nameserver);
        batch.relate(
            &subject,
            relation::USES_NAMESERVER,
            &nameserver_key,
            Some(key),
        );
    }
    for contact in &record.summary.contacts {
        let Some(name) = contact.display_name() else {
            continue;
        };
        let verb = if contact.has_role("registrar") {
            relation::REGISTERED_WITH
        } else if contact.has_role("registrant") {
            match target {
                Target::Domain(_) => relation::REGISTERED_TO,
                _ => relation::OPERATED_BY,
            }
        } else if matches!(target, Target::Ip(_) | Target::Asn(_)) {
            relation::OPERATED_BY
        } else {
            continue;
        };
        let organization = batch.entity(EntityKind::Organization, name, Some(key));
        batch.relate(&subject, verb, &organization, Some(key));
    }
    for (action, date) in &record.summary.events {
        let kind = match action.to_ascii_lowercase().as_str() {
            "registration" => TimelineKind::Registration,
            "expiration" => TimelineKind::Expiration,
            _ => TimelineKind::Updated,
        };
        batch.event(
            *date,
            kind,
            format!("{} {} (WHOIS)", target.value(), action),
            Some(key),
            Some(subject.clone()),
        );
    }
    batch.summary = format!(
        "WHOIS fallback: {} nameserver(s), {} contact(s)",
        record.summary.nameservers.len(),
        record.summary.contacts.len()
    );
    batch
}

fn whois_fields(summary: &RdapSummary) -> Fields {
    let mut fields = Fields::new();
    fields.push_opt("handle", summary.handle.clone());
    fields.push_opt("name", summary.name.clone());
    fields.push_opt("country", summary.country.clone());
    if !summary.status.is_empty() {
        fields.push("status", summary.status.join(", "));
    }
    if !summary.nameservers.is_empty() {
        fields.push("nameservers", summary.nameservers.join(", "));
    }
    fields
}

/// Parses common registrar WHOIS and IANA-style `key: value` records.
pub fn parse_whois(text: &str) -> WhoisRecord {
    let raw = sanitize_text(text).into_owned();
    let mut summary = RdapSummary {
        object_class: "whois".into(),
        ..RdapSummary::default()
    };
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with('%')
            || line.starts_with('#')
            || line.starts_with('>')
        {
            continue;
        }
        let Some((key, value)) = split_whois_line(line) else {
            continue;
        };
        let key = key.to_ascii_lowercase();
        let value = sanitize_line(value.trim()).into_owned();
        if value.is_empty() || is_redacted(&value) {
            continue;
        }
        match key.as_str() {
            "nserver" | "name server" | "nameserver" => {
                let host = value.split_whitespace().next().unwrap_or(&value);
                let host = host.trim_end_matches('.').to_ascii_lowercase();
                if !host.is_empty() && !summary.nameservers.iter().any(|existing| existing == &host)
                {
                    summary.nameservers.push(host);
                }
            }
            "registrar" => push_contact(&mut summary, "registrar", &value),
            "registrant"
            | "registrant name"
            | "registrant organization"
            | "orgname"
            | "organisation"
            | "organization"
            | "org" => {
                push_contact(&mut summary, "registrant", &value);
            }
            "country" | "registrant country" => {
                if summary.country.is_none() {
                    summary.country = Some(value);
                }
            }
            "creation date" | "created" | "created on" | "registration time" | "registered" => {
                if let Some(date) = parse_date(&value) {
                    summary.events.push(("registration".into(), date));
                }
            }
            "registry expiry date"
            | "expiry date"
            | "expires"
            | "expiration date"
            | "paid-till" => {
                if let Some(date) = parse_date(&value) {
                    summary.events.push(("expiration".into(), date));
                }
            }
            "updated date" | "last updated" | "changed" => {
                if let Some(date) = parse_date(&value) {
                    summary.events.push(("last changed".into(), date));
                }
            }
            "domain name" | "ldhname" => summary.name = Some(value.to_ascii_uppercase()),
            "domain status" | "status" => {
                if !summary.status.iter().any(|existing| existing == &value) {
                    summary.status.push(value);
                }
            }
            "whois" | "registrar whois server" => summary.port43 = Some(value),
            "dnssec" => {
                summary.secure_dns = Some(value.to_ascii_lowercase().contains("signed"));
            }
            _ => {}
        }
    }
    WhoisRecord { summary, raw }
}

fn split_whois_line(line: &str) -> Option<(&str, &str)> {
    line.split_once(':').filter(|(key, _)| {
        let key = key.trim();
        (!key.is_empty() && key.len() < 80 && !key.contains(' '))
            || key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || " -_".contains(c))
    })
}

fn push_contact(summary: &mut RdapSummary, role: &str, name: &str) {
    if is_redacted(name) {
        return;
    }
    if summary
        .contacts
        .iter()
        .any(|contact| contact.has_role(role) && contact.display_name() == Some(name))
    {
        return;
    }
    summary.contacts.push(RdapContact {
        roles: vec![role.to_string()],
        name: Some(name.to_string()),
        organization: Some(name.to_string()),
        ..RdapContact::default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
Domain Name: EXAMPLE.COM
Registrar: RESERVED-Internet Assigned Numbers Authority
Creation Date: 1995-08-14T04:00:00Z
Registry Expiry Date: 2026-08-13T04:00:00Z
Name Server: A.IANA-SERVERS.NET
Name Server: B.IANA-SERVERS.NET
Registrant Organization: Internet Assigned Numbers Authority
DNSSEC: signedDelegation
"#;

    const IANA: &str = r#"
domain:       EXAMPLE.COM
nserver:      a.iana-servers.net.
nserver:      b.iana-servers.net.
organisation: Internet Assigned Numbers Authority
created:      1992-01-01
"#;

    #[test]
    fn parses_registrar_style_whois() {
        let actual = parse_whois(SAMPLE);
        assert_eq!(
            actual.summary.nameservers,
            vec!["a.iana-servers.net", "b.iana-servers.net"]
        );
        assert!(
            actual
                .summary
                .contacts
                .iter()
                .any(|contact| contact.has_role("registrar"))
        );
        assert_eq!(actual.summary.events.len(), 2);
        assert_eq!(actual.summary.secure_dns, Some(true));
    }

    #[test]
    fn parses_iana_key_value_whois() {
        let actual = parse_whois(IANA);
        assert_eq!(actual.summary.nameservers.len(), 2);
        assert!(
            actual.summary.contacts.iter().any(
                |contact| contact.display_name() == Some("Internet Assigned Numbers Authority")
            )
        );
    }

    #[test]
    fn batch_emits_catalog_verbs() {
        let target = Target::Domain("example.com".into());
        let actual = build_batch(&target, &parse_whois(SAMPLE));
        assert!(
            actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "uses_nameserver")
        );
        assert!(
            actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "registered_with")
        );
        assert_eq!(actual.sources[0].collector.as_deref(), Some("whois"));
    }

    #[test]
    fn thin_rdap_detects_empty_summaries() {
        assert!(is_thin_rdap(&RdapSummary::default()));
        let mut filled = RdapSummary::default();
        filled.nameservers.push("ns1.example.net".into());
        assert!(!is_thin_rdap(&filled));
    }
}
