//! DNS records: A, AAAA, CNAME, MX, NS, TXT, SOA, CAA (plus `www.` and `_dmarc.` names).

use std::collections::BTreeSet;

use futures::future::{BoxFuture, join_all};
use hickory_resolver::proto::rr::{RData, Record, RecordType};

use super::{CollectResult, CollectionOutput, Collector, CollectorContext};
use crate::files::sha256_hex;
use crate::model::{
    EntityKey, EntityKind, EvidenceBatch, Fields, NewSource, SourceKind, Target, TargetKind,
    relation,
};
use crate::sanitize::sanitize_line;

const RECORD_TYPES: [RecordType; 8] = [
    RecordType::A,
    RecordType::AAAA,
    RecordType::CNAME,
    RecordType::MX,
    RecordType::NS,
    RecordType::TXT,
    RecordType::SOA,
    RecordType::CAA,
];
const SUBNAME_TYPES: [RecordType; 3] = [RecordType::A, RecordType::AAAA, RecordType::CNAME];

pub struct DnsCollector;

impl Collector for DnsCollector {
    fn id(&self) -> &'static str {
        "dns"
    }

    fn label(&self) -> &'static str {
        "DNS"
    }

    fn description(&self) -> &'static str {
        "A, AAAA, CNAME, MX, NS, TXT, SOA, and CAA records; SPF and DMARC policy"
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

/// A DNS answer in presentation form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DnsRecord {
    pub name: String,
    pub ttl: u32,
    pub record_type: String,
    pub value: String,
    /// The host or address the record points to (A, AAAA, CNAME, MX, NS, PTR).
    pub points_to: Option<String>,
}

impl DnsRecord {
    pub fn zone_line(&self) -> String {
        format!(
            "{}.\t{}\tIN\t{}\t{}",
            self.name, self.ttl, self.record_type, self.value
        )
    }
}

/// Everything learned from one DNS collection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DnsAnswers {
    pub domain: String,
    pub records: Vec<DnsRecord>,
    pub nxdomain: bool,
    pub warnings: Vec<String>,
}

async fn collect(target: &Target, context: &CollectorContext) -> CollectResult {
    let domain = match target {
        Target::Domain(domain) => domain.clone(),
        Target::Email(_) => target.hostname().ok_or("the email address has no domain")?,
        _ => {
            return Err(format!(
                "the DNS collector does not accept {}",
                target.kind()
            ));
        }
    };
    context.job.progress(format!("Querying DNS for {domain}"));
    let mut queries: Vec<(String, RecordType)> = RECORD_TYPES
        .iter()
        .map(|kind| (domain.clone(), *kind))
        .collect();
    if context.settings.check_www && !domain.starts_with("www.") {
        queries.extend(
            SUBNAME_TYPES
                .iter()
                .map(|kind| (format!("www.{domain}"), *kind)),
        );
    }
    queries.push((format!("_dmarc.{domain}"), RecordType::TXT));
    let lookups = queries
        .iter()
        .map(|(name, kind)| lookup(context, name, *kind));
    let results = join_all(lookups).await;
    let mut answers = DnsAnswers {
        domain: domain.clone(),
        ..DnsAnswers::default()
    };
    let mut failures = 0;
    for ((name, kind), result) in queries.iter().zip(results) {
        match result {
            Ok(records) => answers.records.extend(records),
            Err(LookupFailure::NxDomain) if name == &domain => answers.nxdomain = true,
            Err(LookupFailure::NxDomain) | Err(LookupFailure::NoRecords) => {}
            Err(LookupFailure::Other(message)) => {
                failures += 1;
                answers.warnings.push(format!("{kind} {name}: {message}"));
            }
        }
    }
    if failures == queries.len() {
        return Err(format!(
            "DNS collection failed: {}",
            answers.warnings.first().cloned().unwrap_or_default()
        ));
    }
    dedupe_records(&mut answers.records);
    for warning in &answers.warnings {
        context.job.log(format!("warning: {warning}"));
    }
    let batch = build_batch(target, &answers);
    Ok(CollectionOutput {
        batch,
        warnings: answers.warnings,
    })
}

enum LookupFailure {
    NxDomain,
    NoRecords,
    Other(String),
}

async fn lookup(
    context: &CollectorContext,
    name: &str,
    kind: RecordType,
) -> Result<Vec<DnsRecord>, LookupFailure> {
    let fqdn = format!("{name}.");
    let result = tokio::select! {
        _ = context.job.cancel.cancelled() => return Err(LookupFailure::Other("cancelled".into())),
        result = context.net.dns.lookup(fqdn.as_str(), kind) => result,
    };
    match result {
        Ok(lookup) => Ok(lookup.answers().iter().filter_map(record_from).collect()),
        Err(error) if error.is_nx_domain() => Err(LookupFailure::NxDomain),
        Err(error) if error.is_no_records_found() => Err(LookupFailure::NoRecords),
        Err(error) => Err(LookupFailure::Other(error.to_string())),
    }
}

fn trim_name(name: &impl std::fmt::Display) -> String {
    name.to_string().trim_end_matches('.').to_ascii_lowercase()
}

/// Converts a resolver record to presentation form. Values are sanitized: TXT records are
/// arbitrary attacker-controlled text.
pub fn record_from(record: &Record) -> Option<DnsRecord> {
    let name = trim_name(&record.name);
    let record_type = record.record_type().to_string();
    let (value, points_to) = match &record.data {
        RData::A(address) => (address.0.to_string(), Some(address.0.to_string())),
        RData::AAAA(address) => (address.0.to_string(), Some(address.0.to_string())),
        RData::CNAME(cname) => (
            format!("{}.", trim_name(&cname.0)),
            Some(trim_name(&cname.0)),
        ),
        RData::NS(ns) => (format!("{}.", trim_name(&ns.0)), Some(trim_name(&ns.0))),
        RData::PTR(ptr) => (format!("{}.", trim_name(&ptr.0)), Some(trim_name(&ptr.0))),
        RData::MX(mx) => (
            format!("{} {}.", mx.preference, trim_name(&mx.exchange)),
            Some(trim_name(&mx.exchange)),
        ),
        RData::TXT(txt) => {
            let joined: String = txt
                .txt_data
                .iter()
                .map(|part| String::from_utf8_lossy(part).into_owned())
                .collect();
            (format!("\"{}\"", sanitize_line(&joined)), None)
        }
        other => (sanitize_line(&other.to_string()).into_owned(), None),
    };
    Some(DnsRecord {
        name,
        ttl: record.ttl,
        record_type,
        value,
        points_to,
    })
}

fn dedupe_records(records: &mut Vec<DnsRecord>) {
    let mut seen = BTreeSet::new();
    records.retain(|record| {
        seen.insert((
            record.name.clone(),
            record.record_type.clone(),
            record.value.clone(),
        ))
    });
}

/// Parsed SPF policy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpfPolicy {
    pub includes: Vec<String>,
    pub redirect: Option<String>,
    pub networks: Vec<String>,
    pub all: Option<String>,
}

/// Parses `v=spf1 ...` TXT content. Returns `None` for non-SPF records.
pub fn parse_spf(txt: &str) -> Option<SpfPolicy> {
    let text = txt.trim().trim_matches('"');
    let mut terms = text.split_whitespace();
    if !terms.next()?.eq_ignore_ascii_case("v=spf1") {
        return None;
    }
    let mut policy = SpfPolicy::default();
    for term in terms {
        let lower = term.to_ascii_lowercase();
        let without_qualifier = lower.trim_start_matches(['+', '-', '~', '?']);
        if let Some(domain) = without_qualifier.strip_prefix("include:") {
            policy.includes.push(domain.to_string());
        } else if let Some(domain) = without_qualifier.strip_prefix("redirect=") {
            policy.redirect = Some(domain.to_string());
        } else if let Some(network) = without_qualifier
            .strip_prefix("ip4:")
            .or_else(|| without_qualifier.strip_prefix("ip6:"))
        {
            policy.networks.push(network.to_string());
        } else if without_qualifier == "all" {
            policy.all = Some(term.to_string());
        }
    }
    Some(policy)
}

/// Recognizes common domain-verification TXT tokens, which reveal third-party services in use.
pub fn txt_service(txt: &str) -> Option<&'static str> {
    const PATTERNS: [(&str, &str); 16] = [
        ("google-site-verification=", "Google"),
        ("ms=", "Microsoft 365"),
        ("facebook-domain-verification=", "Meta"),
        ("apple-domain-verification=", "Apple"),
        ("atlassian-domain-verification=", "Atlassian"),
        ("docusign=", "DocuSign"),
        ("stripe-verification=", "Stripe"),
        ("globalsign-domain-verification=", "GlobalSign"),
        ("adobe-idp-site-verification=", "Adobe"),
        ("zoom_verify_", "Zoom"),
        ("slack-domain-verification=", "Slack"),
        ("hubspot-developer-verification=", "HubSpot"),
        ("onetrust-domain-verification=", "OneTrust"),
        ("cisco-ci-domain-verification=", "Cisco"),
        ("openai-domain-verification=", "OpenAI"),
        ("_github-challenge", "GitHub"),
    ];
    let lower = txt.trim().trim_matches('"').to_ascii_lowercase();
    PATTERNS
        .iter()
        .find(|(prefix, _)| lower.starts_with(prefix))
        .map(|(_, service)| *service)
}

fn dmarc_policy(txt: &str) -> Option<String> {
    let text = txt.trim().trim_matches('"');
    if !text.to_ascii_lowercase().starts_with("v=dmarc1") {
        return None;
    }
    text.split(';')
        .map(str::trim)
        .find_map(|tag| tag.strip_prefix("p=").map(str::to_string))
}

fn entity_for_name(name: &str, domain: &str) -> EntityKey {
    if name == domain {
        EntityKey::new(EntityKind::Domain, name)
    } else {
        EntityKey::new(EntityKind::Hostname, name)
    }
}

/// Builds the evidence batch for a DNS collection. Pure: unit-testable with synthetic answers.
pub fn build_batch(target: &Target, answers: &DnsAnswers) -> EvidenceBatch {
    let domain = &answers.domain;
    let mut batch = EvidenceBatch::new();
    let (content, observations, fields) = describe(answers);
    let mut source =
        NewSource::collected(SourceKind::Dns, "dns", format!("DNS records for {domain}"));
    source.content_hash = Some(stable_hash(answers));
    source.content = content;
    source.fields = fields;
    let key = batch.add_source(source);
    let domain_key = batch.entity(EntityKind::Domain, domain, Some(key));
    if let Target::Email(email) = target {
        let email_key = batch.entity(EntityKind::Email, email, Some(key));
        batch.relate(&email_key, relation::HOSTED_ON, &domain_key, Some(key));
    }
    for record in &answers.records {
        let owner = entity_for_name(&record.name, domain);
        if owner.kind == EntityKind::Hostname {
            if record.name.starts_with('_') {
                continue;
            }
            if record.name.ends_with(&format!(".{domain}")) {
                batch.relate(&owner, relation::SUBDOMAIN_OF, &domain_key, Some(key));
            }
        }
        relate_record(&mut batch, &owner, record, key);
    }
    for include in &observations.spf_includes {
        let include_key = EntityKey::new(EntityKind::Hostname, include);
        batch.relate(&domain_key, relation::SPF_INCLUDES, &include_key, Some(key));
    }
    for service in &observations.services {
        batch.annotate(&domain_key, "txt_verification", *service);
    }
    batch.summary = summarize(answers);
    batch
}

fn relate_record(batch: &mut EvidenceBatch, owner: &EntityKey, record: &DnsRecord, key: usize) {
    let Some(points_to) = record.points_to.as_deref() else {
        return;
    };
    let (kind, relationship) = match record.record_type.as_str() {
        "A" | "AAAA" => (EntityKind::Ip, relation::RESOLVES_TO),
        "CNAME" => (EntityKind::Hostname, relation::ALIAS_OF),
        "NS" => (EntityKind::Hostname, relation::USES_NAMESERVER),
        "MX" => (EntityKind::Hostname, relation::MAIL_HANDLED_BY),
        _ => return,
    };
    let to = EntityKey::new(kind, points_to);
    batch.relate(owner, relationship, &to, Some(key));
}

#[derive(Default)]
struct Observations {
    spf_includes: Vec<String>,
    services: Vec<&'static str>,
}

fn describe(answers: &DnsAnswers) -> (String, Observations, Fields) {
    let mut lines = vec![format!(
        "; DNS records for {} (system resolver)",
        answers.domain
    )];
    let mut observations = Observations::default();
    let mut fields = Fields::new();
    if answers.nxdomain {
        lines.push(format!("; {} does not exist (NXDOMAIN)", answers.domain));
        fields.push("status", "NXDOMAIN");
    }
    let mut current_name = String::new();
    for record in &answers.records {
        if record.name != current_name {
            current_name = record.name.clone();
            lines.push(format!(";; {current_name}"));
        }
        lines.push(record.zone_line());
    }
    let mut notes = Vec::new();
    for record in answers
        .records
        .iter()
        .filter(|record| record.record_type == "TXT")
    {
        if let Some(policy) = parse_spf(&record.value) {
            observations
                .spf_includes
                .extend(policy.includes.iter().cloned());
            notes.push(format!(
                "SPF: includes [{}] networks [{}] all={}",
                policy.includes.join(", "),
                policy.networks.join(", "),
                policy.all.as_deref().unwrap_or("none")
            ));
            fields.push("spf", record.value.trim_matches('"'));
        }
        if let Some(policy) = dmarc_policy(&record.value) {
            notes.push(format!("DMARC policy: p={policy}"));
            fields.push("dmarc_policy", policy);
        }
        if let Some(service) = txt_service(&record.value)
            && !observations.services.contains(&service)
        {
            observations.services.push(service);
        }
    }
    if !observations.services.is_empty() {
        notes.push(format!(
            "Domain verification tokens for: {}",
            observations.services.join(", ")
        ));
    }
    let mail: Vec<String> = answers
        .records
        .iter()
        .filter(|record| record.record_type == "MX")
        .map(|record| record.value.clone())
        .collect();
    if !mail.is_empty() {
        notes.push(format!("Mail exchangers: {}", mail.join(", ")));
    }
    if !notes.is_empty() {
        lines.push(";; observations".to_string());
        lines.extend(notes.into_iter().map(|note| format!("; {note}")));
    }
    for record_type in ["A", "AAAA", "CNAME", "MX", "NS", "TXT", "CAA"] {
        let count = answers
            .records
            .iter()
            .filter(|record| record.record_type == record_type)
            .count();
        if count > 0 {
            fields.push(format!("{record_type} records"), count.to_string());
        }
    }
    (lines.join("\n"), observations, fields)
}

/// A hash over the answers without TTLs, so identical re-collections deduplicate even though
/// resolvers count TTLs down.
fn stable_hash(answers: &DnsAnswers) -> String {
    let mut lines: Vec<String> = answers
        .records
        .iter()
        .map(|record| format!("{} {} {}", record.name, record.record_type, record.value))
        .collect();
    lines.sort();
    if answers.nxdomain {
        lines.push("NXDOMAIN".into());
    }
    sha256_hex(lines.join("\n").as_bytes())
}

fn summarize(answers: &DnsAnswers) -> String {
    if answers.nxdomain {
        return format!("{} does not exist (NXDOMAIN)", answers.domain);
    }
    let count = |record_type: &str| {
        answers
            .records
            .iter()
            .filter(|record| record.record_type == record_type)
            .count()
    };
    format!(
        "{} records: {} A, {} AAAA, {} MX, {} NS, {} TXT",
        answers.records.len(),
        count("A"),
        count("AAAA"),
        count("MX"),
        count("NS"),
        count("TXT")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(name: &str, record_type: &str, value: &str, points_to: Option<&str>) -> DnsRecord {
        DnsRecord {
            name: name.into(),
            ttl: 300,
            record_type: record_type.into(),
            value: value.into(),
            points_to: points_to.map(str::to_string),
        }
    }

    fn sample_answers() -> DnsAnswers {
        DnsAnswers {
            domain: "example.com".into(),
            records: vec![
                record("example.com", "A", "192.0.2.10", Some("192.0.2.10")),
                record(
                    "example.com",
                    "NS",
                    "ns1.cloudflare.com.",
                    Some("ns1.cloudflare.com"),
                ),
                record(
                    "example.com",
                    "MX",
                    "10 mx.example.net.",
                    Some("mx.example.net"),
                ),
                record(
                    "example.com",
                    "TXT",
                    "\"v=spf1 include:_spf.google.com ip4:192.0.2.0/24 -all\"",
                    None,
                ),
                record(
                    "example.com",
                    "TXT",
                    "\"google-site-verification=abc\"",
                    None,
                ),
                record(
                    "www.example.com",
                    "CNAME",
                    "example.cdn.net.",
                    Some("example.cdn.net"),
                ),
                record("example.cdn.net", "A", "198.51.100.7", Some("198.51.100.7")),
                record("_dmarc.example.com", "TXT", "\"v=DMARC1; p=reject\"", None),
            ],
            nxdomain: false,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn spf_is_parsed() {
        let actual = parse_spf(
            "\"v=spf1 include:_spf.google.com ~include:mail.example.net ip4:192.0.2.0/24 ~all\"",
        )
        .expect("spf");
        assert_eq!(actual.includes, vec!["_spf.google.com", "mail.example.net"]);
        assert_eq!(actual.networks, vec!["192.0.2.0/24"]);
        assert_eq!(actual.all.as_deref(), Some("~all"));
        assert!(parse_spf("hello").is_none());
    }

    #[test]
    fn verification_tokens_are_recognized() {
        assert_eq!(txt_service("\"MS=ms12345\""), Some("Microsoft 365"));
        assert_eq!(txt_service("random"), None);
    }

    #[test]
    fn batch_contains_relationships_for_each_record_kind() {
        let input_target = Target::Domain("example.com".into());
        let actual = build_batch(&input_target, &sample_answers());
        let kinds: BTreeSet<&str> = actual
            .relationships
            .iter()
            .map(|relationship| relationship.kind.as_str())
            .collect();
        let expected: BTreeSet<&str> = [
            "resolves_to",
            "uses_nameserver",
            "mail_handled_by",
            "spf_includes",
            "alias_of",
            "subdomain_of",
        ]
        .into_iter()
        .collect();
        assert_eq!(kinds, expected);
        assert_eq!(actual.sources.len(), 1);
        assert!(actual.sources[0].content.contains("DMARC policy: p=reject"));
        assert!(actual.sources[0].content.contains("Google"));
    }

    #[test]
    fn dmarc_names_do_not_become_entities() {
        let input_target = Target::Domain("example.com".into());
        let actual = build_batch(&input_target, &sample_answers());
        assert!(
            !actual
                .entities
                .iter()
                .any(|entity| entity.key.value.starts_with("_dmarc"))
        );
    }

    #[test]
    fn stable_hash_ignores_ttl() {
        let mut first = sample_answers();
        let second = sample_answers();
        first.records[0].ttl = 12;
        assert_eq!(stable_hash(&first), stable_hash(&second));
    }

    #[test]
    fn nxdomain_is_recorded() {
        let input = DnsAnswers {
            domain: "nonexistent.example".into(),
            nxdomain: true,
            ..DnsAnswers::default()
        };
        let actual = build_batch(&Target::Domain("nonexistent.example".into()), &input);
        assert!(actual.summary.contains("NXDOMAIN"));
        assert_eq!(actual.sources[0].fields.get("status"), Some("NXDOMAIN"));
    }

    #[test]
    fn email_targets_link_to_their_domain() {
        let input_target = Target::Email("alice@example.com".into());
        let actual = build_batch(&input_target, &sample_answers());
        assert!(
            actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "hosted_on"
                    && relationship.from.value == "alice@example.com")
        );
    }
}
