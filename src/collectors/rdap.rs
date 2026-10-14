//! RDAP (registration data) for domains, IP networks, and autonomous systems.
//!
//! Servers are located through the IANA bootstrap registry (RFC 9224), cached for a week in the
//! data directory, so queries go straight to the authoritative registry.

use std::net::IpAddr;
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use futures::future::BoxFuture;
use serde_json::Value;
use url::Url;

use super::net::{FetchOptions, describe_status, fetch};
use super::{CollectResult, CollectionOutput, Collector, CollectorContext};
use crate::files::sha256_hex;
use crate::model::{
    EntityKey, EntityKind, EvidenceBatch, Fields, NewSource, SourceKind, Target, TargetKind,
    TimelineKind, Timestamp, relation,
};
use crate::sanitize::sanitize_line;

const BOOTSTRAP_BASE: &str = "https://data.iana.org/rdap/";
const BOOTSTRAP_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const BOOTSTRAP_MAX_BYTES: usize = 2 * 1024 * 1024;
const RESPONSE_MAX_BYTES: usize = 2 * 1024 * 1024;
const RDAP_ACCEPT: &str = "application/rdap+json, application/json;q=0.9";
const MAX_ENTITY_DEPTH: usize = 4;

pub struct RdapCollector;

impl Collector for RdapCollector {
    fn id(&self) -> &'static str {
        "rdap"
    }

    fn label(&self) -> &'static str {
        "RDAP"
    }

    fn description(&self) -> &'static str {
        "Registration data (registrar, dates, nameservers, network owner) via RDAP"
    }

    fn accepts(&self, kind: TargetKind) -> bool {
        matches!(kind, TargetKind::Domain | TargetKind::Ip | TargetKind::Asn)
    }

    fn collect<'a>(
        &'a self,
        target: &'a Target,
        context: &'a CollectorContext,
    ) -> BoxFuture<'a, CollectResult> {
        Box::pin(collect(target, context))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Registry {
    Dns,
    Ipv4,
    Ipv6,
    Asn,
}

impl Registry {
    fn file_name(self) -> &'static str {
        match self {
            Registry::Dns => "dns.json",
            Registry::Ipv4 => "ipv4.json",
            Registry::Ipv6 => "ipv6.json",
            Registry::Asn => "asn.json",
        }
    }
}

/// `(entries, service URLs)` pairs from an IANA bootstrap file.
pub type BootstrapServices = Vec<(Vec<String>, Vec<String>)>;

async fn collect(target: &Target, context: &CollectorContext) -> CollectResult {
    match collect_rdap(target, context).await {
        Ok(mut output) => {
            if super::whois::is_thin_rdap_batch(&output.batch) {
                match super::whois::collect_whois(target, &context.tools, &context.job).await {
                    Ok(whois) => {
                        output.batch.merge(whois.batch);
                        output.warnings.extend(whois.warnings);
                        output
                            .warnings
                            .push("RDAP was thin; merged structured WHOIS".into());
                    }
                    Err(error) => output
                        .warnings
                        .push(format!("WHOIS fallback skipped: {error}")),
                }
            }
            Ok(output)
        }
        Err(error) => match super::whois::collect_whois(target, &context.tools, &context.job).await
        {
            Ok(mut output) => {
                output
                    .warnings
                    .push(format!("RDAP failed ({error}); used structured WHOIS"));
                Ok(output)
            }
            Err(whois_error) => Err(format!("{error}; WHOIS fallback: {whois_error}")),
        },
    }
}

async fn collect_rdap(target: &Target, context: &CollectorContext) -> CollectResult {
    let (registry, path) = match target {
        Target::Domain(domain) => (Registry::Dns, format!("domain/{domain}")),
        Target::Ip(IpAddr::V4(ip)) => (Registry::Ipv4, format!("ip/{ip}")),
        Target::Ip(IpAddr::V6(ip)) => (Registry::Ipv6, format!("ip/{ip}")),
        Target::Asn(number) => (Registry::Asn, format!("autnum/{number}")),
        _ => {
            return Err(format!(
                "the RDAP collector does not accept {}",
                target.kind()
            ));
        }
    };
    context
        .job
        .progress("Locating the RDAP server (IANA bootstrap)");
    let services = load_bootstrap(context, registry).await?;
    let base = match_service(&services, target).ok_or_else(|| no_service_message(target))?;
    let base = if base.ends_with('/') {
        base
    } else {
        format!("{base}/")
    };
    let url = Url::parse(&format!("{base}{path}"))
        .map_err(|error| format!("invalid RDAP URL: {error}"))?;
    context.job.progress(format!("Querying {url}"));
    context.job.command("GET", &[url.to_string()]);
    let options = FetchOptions {
        max_body_bytes: RESPONSE_MAX_BYTES,
        accept: Some(RDAP_ACCEPT),
    };
    let response = fetch(&context.net, &url, &options, &context.job.cancel).await?;
    if response.status == 404 {
        return Err(format!(
            "RDAP collection failed: HTTP 404: no registration record for {}",
            target.value()
        ));
    }
    if !response.is_success() {
        return Err(describe_status("RDAP collection", response.status));
    }
    let json: Value = serde_json::from_slice(&response.body).map_err(|error| {
        format!("RDAP collection failed: the server returned invalid JSON ({error})")
    })?;
    let summary = parse_rdap(&json);
    let snapshot = context
        .files
        .store_snapshot(&response.body, "json")
        .map_err(|error| format!("could not store RDAP snapshot: {error}"))?;
    let batch = build_batch(
        target,
        &summary,
        response.final_url.as_str(),
        &snapshot.relative_path,
        &snapshot.sha256,
    );
    Ok(CollectionOutput::new(batch))
}

fn no_service_message(target: &Target) -> String {
    match target {
        Target::Domain(domain) => {
            let tld = domain.rsplit('.').next().unwrap_or(domain);
            format!("no RDAP service is registered for .{tld}; try the `whois` tool instead")
        }
        _ => format!("no RDAP service is registered for {}", target.value()),
    }
}

async fn load_bootstrap(
    context: &CollectorContext,
    registry: Registry,
) -> Result<BootstrapServices, String> {
    let cache_path = context.cache_dir.join("rdap").join(registry.file_name());
    if let Some(services) = read_fresh_cache(&cache_path) {
        return Ok(services);
    }
    let url = Url::parse(&format!("{BOOTSTRAP_BASE}{}", registry.file_name()))
        .map_err(|error| format!("invalid bootstrap URL: {error}"))?;
    context.job.command("GET", &[url.to_string()]);
    let options = FetchOptions {
        max_body_bytes: BOOTSTRAP_MAX_BYTES,
        accept: Some("application/json"),
    };
    let response = match fetch(&context.net, &url, &options, &context.job.cancel).await {
        Ok(response) if response.is_success() => response,
        Ok(response) => {
            return stale_cache_or(
                &cache_path,
                describe_status("RDAP bootstrap download", response.status),
            );
        }
        Err(error) => {
            return stale_cache_or(
                &cache_path,
                format!("RDAP bootstrap download failed: {error}"),
            );
        }
    };
    let services = parse_bootstrap(&response.body)?;
    if let Some(parent) = cache_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(error) = crate::files::write_atomically(&cache_path, &response.body) {
        tracing::warn!(%error, "could not cache RDAP bootstrap file");
    }
    Ok(services)
}

fn read_fresh_cache(path: &Path) -> Option<BootstrapServices> {
    let metadata = std::fs::metadata(path).ok()?;
    let age = metadata.modified().ok()?.elapsed().ok()?;
    if age > BOOTSTRAP_MAX_AGE {
        return None;
    }
    parse_bootstrap(&std::fs::read(path).ok()?).ok()
}

fn stale_cache_or(path: &Path, error: String) -> Result<BootstrapServices, String> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| parse_bootstrap(&bytes).ok())
        .ok_or(error)
}

pub fn parse_bootstrap(bytes: &[u8]) -> Result<BootstrapServices, String> {
    let json: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid RDAP bootstrap file: {error}"))?;
    let services = json
        .get("services")
        .and_then(Value::as_array)
        .ok_or("invalid RDAP bootstrap file: no services")?;
    Ok(services
        .iter()
        .filter_map(|service| {
            let parts = service.as_array()?;
            let strings = |value: &Value| -> Vec<String> {
                value
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            Some((strings(parts.first()?), strings(parts.get(1)?)))
        })
        .collect())
}

/// Picks the most specific service for a target, preferring HTTPS URLs.
pub fn match_service(services: &BootstrapServices, target: &Target) -> Option<String> {
    let mut best: Option<(usize, &Vec<String>)> = None;
    for (entries, urls) in services {
        for entry in entries {
            let Some(specificity) = specificity(entry, target) else {
                continue;
            };
            if best
                .map(|(current, _)| specificity > current)
                .unwrap_or(true)
            {
                best = Some((specificity, urls));
            }
        }
    }
    let (_, urls) = best?;
    urls.iter()
        .find(|url| url.starts_with("https://"))
        .or_else(|| urls.first())
        .cloned()
}

fn specificity(entry: &str, target: &Target) -> Option<usize> {
    match target {
        Target::Domain(domain) => {
            let entry = entry.to_ascii_lowercase();
            let matches = domain == &entry || domain.ends_with(&format!(".{entry}"));
            matches.then_some(entry.len())
        }
        Target::Ip(ip) => cidr_prefix_if_contains(entry, *ip).map(usize::from),
        Target::Asn(number) => {
            let (start, end) = entry.split_once('-').unwrap_or((entry, entry));
            let start: u32 = start.trim().parse().ok()?;
            let end: u32 = end.trim().parse().ok()?;
            (start..=end)
                .contains(number)
                .then(|| usize::MAX - (end - start) as usize)
        }
        _ => None,
    }
}

/// Returns the prefix length when `cidr` contains `ip`.
pub fn cidr_prefix_if_contains(cidr: &str, ip: IpAddr) -> Option<u8> {
    let (network, length) = cidr.split_once('/')?;
    let length: u8 = length.parse().ok()?;
    match (network.parse::<IpAddr>().ok()?, ip) {
        (IpAddr::V4(network), IpAddr::V4(ip)) if length <= 32 => {
            let mask = if length == 0 {
                0
            } else {
                u32::MAX << (32 - u32::from(length))
            };
            (u32::from(network) & mask == u32::from(ip) & mask).then_some(length)
        }
        (IpAddr::V6(network), IpAddr::V6(ip)) if length <= 128 => {
            let mask = if length == 0 {
                0
            } else {
                u128::MAX << (128 - u32::from(length))
            };
            (u128::from(network) & mask == u128::from(ip) & mask).then_some(length)
        }
        _ => None,
    }
}

/// A contact or organization in an RDAP response.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RdapContact {
    pub roles: Vec<String>,
    pub handle: Option<String>,
    pub name: Option<String>,
    pub organization: Option<String>,
    pub email: Option<String>,
    pub country: Option<String>,
}

impl RdapContact {
    pub fn display_name(&self) -> Option<&str> {
        self.organization
            .as_deref()
            .or(self.name.as_deref())
            .filter(|name| !is_redacted(name))
    }

    pub fn has_role(&self, role: &str) -> bool {
        self.roles
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(role))
    }
}

/// The parts of an RDAP response os-int uses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RdapSummary {
    pub object_class: String,
    pub handle: Option<String>,
    pub name: Option<String>,
    pub status: Vec<String>,
    pub events: Vec<(String, Timestamp)>,
    pub nameservers: Vec<String>,
    pub contacts: Vec<RdapContact>,
    pub secure_dns: Option<bool>,
    pub port43: Option<String>,
    pub start_address: Option<String>,
    pub end_address: Option<String>,
    pub country: Option<String>,
    pub network_type: Option<String>,
    pub parent_handle: Option<String>,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(|text| sanitize_line(text.trim()).into_owned())
        .filter(|text| !text.is_empty())
}

pub fn parse_rdap(json: &Value) -> RdapSummary {
    let mut summary = RdapSummary {
        object_class: text(json, "objectClassName").unwrap_or_else(|| "unknown".into()),
        handle: text(json, "handle"),
        name: text(json, "ldhName").or_else(|| text(json, "name")),
        port43: text(json, "port43"),
        start_address: text(json, "startAddress"),
        end_address: text(json, "endAddress"),
        country: text(json, "country"),
        network_type: text(json, "type"),
        parent_handle: text(json, "parentHandle"),
        ..RdapSummary::default()
    };
    if let Some(start) = json.get("startAutnum").and_then(Value::as_u64) {
        summary.start_address = Some(format!("AS{start}"));
    }
    if let Some(end) = json.get("endAutnum").and_then(Value::as_u64) {
        summary.end_address = Some(format!("AS{end}"));
    }
    summary.status = json
        .get("status")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|item| sanitize_line(item).into_owned())
                .collect()
        })
        .unwrap_or_default();
    summary.events = parse_events(json.get("events"));
    summary.nameservers = json
        .get("nameservers")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| text(item, "ldhName"))
                .map(|name| name.trim_end_matches('.').to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default();
    summary.secure_dns = json
        .get("secureDNS")
        .and_then(|secure| secure.get("delegationSigned"))
        .and_then(Value::as_bool);
    collect_contacts(json.get("entities"), &mut summary.contacts, 0);
    summary
}

fn parse_events(value: Option<&Value>) -> Vec<(String, Timestamp)> {
    value
        .and_then(Value::as_array)
        .map(|events| {
            events
                .iter()
                .filter_map(|event| {
                    let action = text(event, "eventAction")?;
                    let date = parse_date(event.get("eventDate")?.as_str()?)?;
                    Some((action, date))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_date(value: &str) -> Option<Timestamp> {
    let value = value.trim();
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Some(parsed.with_timezone(&Utc));
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return date.and_hms_opt(0, 0, 0).map(|naive| naive.and_utc());
    }
    [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
    ]
    .iter()
    .find_map(|format| NaiveDateTime::parse_from_str(value, format).ok())
    .map(|naive| naive.and_utc())
}

fn collect_contacts(value: Option<&Value>, contacts: &mut Vec<RdapContact>, depth: usize) {
    let Some(entities) = value.and_then(Value::as_array) else {
        return;
    };
    if depth > MAX_ENTITY_DEPTH {
        return;
    }
    for entity in entities {
        let mut contact = RdapContact {
            roles: entity
                .get("roles")
                .and_then(Value::as_array)
                .map(|roles| {
                    roles
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            handle: text(entity, "handle"),
            ..RdapContact::default()
        };
        apply_vcard(entity.get("vcardArray"), &mut contact);
        contacts.push(contact);
        collect_contacts(entity.get("entities"), contacts, depth + 1);
    }
}

fn apply_vcard(value: Option<&Value>, contact: &mut RdapContact) {
    let Some(properties) = value
        .and_then(|vcard| vcard.get(1))
        .and_then(Value::as_array)
    else {
        return;
    };
    for property in properties {
        let Some(parts) = property.as_array() else {
            continue;
        };
        let Some(name) = parts.first().and_then(Value::as_str) else {
            continue;
        };
        let value = parts.get(3).map(vcard_value).unwrap_or_default();
        if value.is_empty() {
            continue;
        }
        match name {
            "fn" => contact.name = Some(value),
            "org" => contact.organization = Some(value),
            "email" => contact.email = Some(value.to_ascii_lowercase()),
            "adr" => contact.country = vcard_country(property).or(contact.country.take()),
            _ => {}
        }
    }
}

fn vcard_value(value: &Value) -> String {
    let raw = match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    };
    sanitize_line(raw.trim()).into_owned()
}

fn vcard_country(property: &Value) -> Option<String> {
    let parts = property.as_array()?;
    parts
        .get(1)
        .and_then(|parameters| parameters.get("cc"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            parts
                .get(3)
                .and_then(Value::as_array)
                .and_then(|address| address.last())
                .and_then(Value::as_str)
                .filter(|country| !country.is_empty())
                .map(str::to_string)
        })
}

/// Recognizes privacy-redacted registration fields so they do not become entities.
pub fn is_redacted(value: &str) -> bool {
    const MARKERS: [&str; 9] = [
        "redacted",
        "privacy",
        "data protected",
        "not disclosed",
        "withheld",
        "masked",
        "gdpr",
        "proxy",
        "statutory masking",
    ];
    let lower = value.to_ascii_lowercase();
    MARKERS.iter().any(|marker| lower.contains(marker))
}

fn timeline_kind(action: &str) -> Option<TimelineKind> {
    match action.to_ascii_lowercase().as_str() {
        "registration" => Some(TimelineKind::Registration),
        "expiration" => Some(TimelineKind::Expiration),
        "last changed" | "transfer" | "reregistration" => Some(TimelineKind::Updated),
        _ => None,
    }
}

fn describe(summary: &RdapSummary) -> (String, Fields) {
    let mut lines = Vec::new();
    let mut fields = Fields::new();
    let mut add = |label: &str, value: Option<&str>| {
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            lines.push(format!("{label}: {value}"));
            fields.push(label.to_ascii_lowercase().replace(' ', "_"), value);
        }
    };
    add("Object", Some(&summary.object_class));
    add("Handle", summary.handle.as_deref());
    add("Name", summary.name.as_deref());
    add("Type", summary.network_type.as_deref());
    add("Start", summary.start_address.as_deref());
    add("End", summary.end_address.as_deref());
    add("Country", summary.country.as_deref());
    add("Parent", summary.parent_handle.as_deref());
    add("WHOIS server", summary.port43.as_deref());
    let status = summary.status.join(", ");
    add("Status", Some(&status));
    let dnssec = summary
        .secure_dns
        .map(|signed| if signed { "signed" } else { "unsigned" });
    add("DNSSEC", dnssec);
    if !summary.events.is_empty() {
        lines.push("Events:".into());
        for (action, date) in &summary.events {
            lines.push(format!("  {action}: {}", date.to_rfc3339()));
            if let Some(kind) = timeline_kind(action) {
                fields.push(kind.as_str(), date.to_rfc3339());
            }
        }
    }
    if !summary.nameservers.is_empty() {
        lines.push(format!("Nameservers: {}", summary.nameservers.join(", ")));
    }
    for contact in &summary.contacts {
        let name = contact
            .display_name()
            .map(str::to_string)
            .or_else(|| {
                contact
                    .name
                    .as_ref()
                    .map(|name| format!("{name} (redacted)"))
            })
            .unwrap_or_else(|| "(no name)".into());
        let mut line = format!("Contact [{}]: {name}", contact.roles.join(", "));
        if let Some(email) = &contact.email {
            line.push_str(&format!(" <{email}>"));
        }
        if let Some(country) = &contact.country {
            line.push_str(&format!(" ({country})"));
        }
        lines.push(line);
        if contact.has_role("abuse")
            && let Some(email) = &contact.email
        {
            fields.push("abuse_email", email.clone());
        }
        if contact.has_role("registrar")
            && let Some(name) = contact.display_name()
        {
            fields.push("registrar", name);
        }
    }
    (lines.join("\n"), fields)
}

/// Builds the evidence batch from a parsed RDAP response. Pure and unit-tested.
pub fn build_batch(
    target: &Target,
    summary: &RdapSummary,
    url: &str,
    snapshot_path: &str,
    snapshot_hash: &str,
) -> EvidenceBatch {
    let mut batch = EvidenceBatch::new();
    let (content, fields) = describe(summary);
    let title = match target {
        Target::Domain(domain) => format!("RDAP registration for {domain}"),
        Target::Ip(ip) => format!("RDAP network for {ip}"),
        _ => format!("RDAP record for {}", target.value()),
    };
    let mut source = NewSource::collected(SourceKind::Rdap, "rdap", title);
    source.url = Some(url.to_string());
    source.content_hash = Some(sha256_hex(content.as_bytes()));
    source.snapshot_path = Some(snapshot_path.to_string());
    source.content = content;
    source.fields = fields;
    source.fields.push("snapshot_sha256", snapshot_hash);
    let key = batch.add_source(source);
    let subject = batch.entity(target.entity_kind(), &target.value(), Some(key));
    for nameserver in &summary.nameservers {
        let nameserver_key = EntityKey::new(EntityKind::Hostname, nameserver);
        batch.relate(
            &subject,
            relation::USES_NAMESERVER,
            &nameserver_key,
            Some(key),
        );
    }
    relate_contacts(&mut batch, target, summary, &subject, key);
    if !matches!(target, Target::Domain(_))
        && let Some(name) = &summary.name
    {
        batch.annotate(&subject, "network_name", name.clone());
    }
    for (action, date) in &summary.events {
        if let Some(kind) = timeline_kind(action) {
            let title = format!("{} {} ({})", target.value(), action, "RDAP");
            batch.event(*date, kind, title, Some(key), Some(subject.clone()));
        }
    }
    batch.summary = summarize(summary);
    batch
}

fn relate_contacts(
    batch: &mut EvidenceBatch,
    target: &Target,
    summary: &RdapSummary,
    subject: &EntityKey,
    key: usize,
) {
    for contact in &summary.contacts {
        let Some(name) = contact.display_name() else {
            continue;
        };
        let relationship = if contact.has_role("registrar") {
            relation::REGISTERED_WITH
        } else if contact.has_role("registrant") {
            match target {
                Target::Domain(_) => relation::REGISTERED_TO,
                _ => relation::OPERATED_BY,
            }
        } else if matches!(target, Target::Ip(_) | Target::Asn(_))
            && contact.has_role("administrative")
        {
            relation::OPERATED_BY
        } else {
            continue;
        };
        let organization = batch.entity(EntityKind::Organization, name, Some(key));
        if let Some(email) = &contact.email {
            batch.annotate(&organization, "email", email.clone());
        }
        batch.relate(subject, relationship, &organization, Some(key));
    }
}

fn summarize(summary: &RdapSummary) -> String {
    let registrar = summary
        .contacts
        .iter()
        .find(|contact| contact.has_role("registrar"))
        .and_then(|contact| contact.display_name());
    let registered = summary
        .events
        .iter()
        .find(|(action, _)| action.eq_ignore_ascii_case("registration"))
        .map(|(_, date)| date.format("%Y-%m-%d").to_string());
    match (registrar, registered) {
        (Some(registrar), Some(date)) => format!("registered {date} via {registrar}"),
        (Some(registrar), None) => format!("registrar {registrar}"),
        (None, Some(date)) => format!("registered {date}"),
        _ => summary
            .name
            .clone()
            .map(|name| format!("network {name}"))
            .unwrap_or_else(|| "registration record retrieved".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOMAIN_RESPONSE: &str = r#"{
      "objectClassName": "domain",
      "handle": "2336799_DOMAIN_COM-VRSN",
      "ldhName": "EXAMPLE.COM",
      "status": ["client delete prohibited", "client transfer prohibited"],
      "events": [
        {"eventAction": "registration", "eventDate": "1995-08-14T04:00:00Z"},
        {"eventAction": "expiration", "eventDate": "2026-08-13T04:00:00Z"},
        {"eventAction": "last update of RDAP database", "eventDate": "2026-10-01T00:00:00Z"}
      ],
      "nameservers": [{"ldhName": "A.IANA-SERVERS.NET"}, {"ldhName": "B.IANA-SERVERS.NET"}],
      "secureDNS": {"delegationSigned": true},
      "entities": [
        {"roles": ["registrar"], "handle": "376",
         "vcardArray": ["vcard", [["version", {}, "text", "4.0"], ["fn", {}, "text", "RESERVED-Internet Assigned Numbers Authority"]]],
         "entities": [{"roles": ["abuse"], "vcardArray": ["vcard", [["fn", {}, "text", "Abuse"], ["email", {}, "text", "Abuse@Example.net"]]]}]},
        {"roles": ["registrant"], "vcardArray": ["vcard", [["fn", {}, "text", "REDACTED FOR PRIVACY"]]]}
      ]
    }"#;

    fn sample_summary() -> RdapSummary {
        parse_rdap(&serde_json::from_str(DOMAIN_RESPONSE).expect("json"))
    }

    #[test]
    fn parses_domain_response() {
        let actual = sample_summary();
        assert_eq!(actual.name.as_deref(), Some("EXAMPLE.COM"));
        assert_eq!(
            actual.nameservers,
            vec!["a.iana-servers.net", "b.iana-servers.net"]
        );
        assert_eq!(actual.events.len(), 3);
        assert_eq!(actual.secure_dns, Some(true));
        assert_eq!(actual.contacts.len(), 3);
        assert_eq!(
            actual.contacts[1].email.as_deref(),
            Some("abuse@example.net")
        );
    }

    #[test]
    fn redacted_registrants_do_not_become_entities() {
        let target = Target::Domain("example.com".into());
        let actual = build_batch(
            &target,
            &sample_summary(),
            "https://rdap.example/domain/example.com",
            "snapshots/aa/x.json",
            "aa",
        );
        let organizations: Vec<&str> = actual
            .entities
            .iter()
            .filter(|entity| entity.key.kind == EntityKind::Organization)
            .map(|entity| entity.key.value.as_str())
            .collect();
        assert_eq!(
            organizations,
            vec!["RESERVED-Internet Assigned Numbers Authority"]
        );
        assert!(
            actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "registered_with")
        );
        assert!(
            !actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "registered_to")
        );
    }

    #[test]
    fn registration_dates_become_timeline_events() {
        let target = Target::Domain("example.com".into());
        let actual = build_batch(&target, &sample_summary(), "u", "snapshots/aa/x.json", "aa");
        let kinds: Vec<&TimelineKind> = actual.timeline.iter().map(|event| &event.kind).collect();
        assert_eq!(
            kinds,
            vec![&TimelineKind::Registration, &TimelineKind::Expiration]
        );
        assert!(actual.summary.contains("1995-08-14"));
        assert_eq!(
            actual.sources[0].fields.get("abuse_email"),
            Some("abuse@example.net")
        );
    }

    #[test]
    fn bootstrap_matching_prefers_most_specific_entry() {
        let input = br#"{"services": [
            [["com", "net"], ["http://rdap.verisign.com/com/v1/", "https://rdap.verisign.com/com/v1/"]],
            [["uk"], ["https://rdap.nominet.uk/uk/"]],
            [["co.uk"], ["https://rdap.example/co.uk/"]]
        ]}"#;
        let services = parse_bootstrap(input).expect("parses");
        let com = match_service(&services, &Target::Domain("www.example.com".into()));
        let couk = match_service(&services, &Target::Domain("example.co.uk".into()));
        let missing = match_service(&services, &Target::Domain("example.zz".into()));
        assert_eq!(com.as_deref(), Some("https://rdap.verisign.com/com/v1/"));
        assert_eq!(couk.as_deref(), Some("https://rdap.example/co.uk/"));
        assert!(missing.is_none());
    }

    #[test]
    fn cidr_matching_handles_v4_and_v6() {
        let v4: IpAddr = "192.0.2.77".parse().expect("ip");
        let v6: IpAddr = "2001:db8::1".parse().expect("ip");
        assert_eq!(cidr_prefix_if_contains("192.0.0.0/8", v4), Some(8));
        assert_eq!(cidr_prefix_if_contains("10.0.0.0/8", v4), None);
        assert_eq!(cidr_prefix_if_contains("2001:db8::/32", v6), Some(32));
        assert_eq!(cidr_prefix_if_contains("2001:db8::/32", v4), None);
    }

    #[test]
    fn asn_ranges_match() {
        let input =
            br#"{"services": [[["1-1876", "13335"], ["https://rdap.arin.net/registry/"]]]}"#;
        let services = parse_bootstrap(input).expect("parses");
        assert!(match_service(&services, &Target::Asn(13335)).is_some());
        assert!(match_service(&services, &Target::Asn(100)).is_some());
        assert!(match_service(&services, &Target::Asn(5000)).is_none());
    }

    #[test]
    fn dates_without_timezones_parse() {
        assert!(parse_date("2020-01-02T03:04:05").is_some());
        assert!(parse_date("2020-01-02T03:04:05+02:00").is_some());
        assert!(parse_date("2011-08-11").is_some());
        assert!(parse_date("yesterday").is_none());
    }
}
