//! Reverse DNS and origin ASN/network data (Team Cymru's public DNS interface) for an IP.

use std::net::{IpAddr, Ipv6Addr};

use futures::future::BoxFuture;
use hickory_resolver::proto::rr::{RData, RecordType};

use super::{CollectResult, CollectionOutput, Collector, CollectorContext};
use crate::files::sha256_hex;
use crate::model::{
    EntityKey, EntityKind, EvidenceBatch, Fields, NewSource, SourceKind, Target, TargetKind,
    TimelineKind, relation,
};
use crate::sanitize::sanitize_line;

pub struct IpCollector;

impl Collector for IpCollector {
    fn id(&self) -> &'static str {
        "ip"
    }

    fn label(&self) -> &'static str {
        "IP"
    }

    fn description(&self) -> &'static str {
        "Reverse DNS, origin ASN, prefix, and registry country for an IP address"
    }

    fn accepts(&self, kind: TargetKind) -> bool {
        matches!(kind, TargetKind::Ip)
    }

    fn collect<'a>(
        &'a self,
        target: &'a Target,
        context: &'a CollectorContext,
    ) -> BoxFuture<'a, CollectResult> {
        Box::pin(collect(target, context))
    }
}

/// Parsed Team Cymru origin record: `ASN | prefix | CC | registry | allocated`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CymruOrigin {
    pub asn: u32,
    pub prefix: String,
    pub country: Option<String>,
    pub registry: Option<String>,
    pub allocated: Option<String>,
}

/// Parsed Team Cymru ASN description: `ASN | name | CC | registry | allocated`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CymruAsn {
    pub asn: u32,
    pub name: String,
    pub country: Option<String>,
    pub registry: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IpObservation {
    pub ip: String,
    pub ptr: Vec<String>,
    pub origin: Option<CymruOrigin>,
    pub asn: Option<CymruAsn>,
    pub warnings: Vec<String>,
}

async fn collect(target: &Target, context: &CollectorContext) -> CollectResult {
    let Target::Ip(ip) = target else {
        return Err(format!(
            "the IP collector does not accept {}",
            target.kind()
        ));
    };
    let mut observation = IpObservation {
        ip: ip.to_string(),
        ..IpObservation::default()
    };
    context.job.progress(format!("Reverse DNS for {ip}"));
    match reverse_lookup(context, *ip).await {
        Ok(names) => observation.ptr = names,
        Err(message) => observation.warnings.push(message),
    }
    context.job.progress(format!("ASN origin lookup for {ip}"));
    match origin_lookup(context, *ip).await {
        Ok(Some(origin)) => {
            let asn = origin.asn;
            observation.origin = Some(origin);
            match asn_lookup(context, asn).await {
                Ok(Some(description)) => observation.asn = Some(description),
                Ok(None) => {}
                Err(message) => observation.warnings.push(message),
            }
        }
        Ok(None) => {}
        Err(message) => observation.warnings.push(message),
    }
    if observation.ptr.is_empty() && observation.origin.is_none() && observation.warnings.is_empty()
    {
        observation
            .warnings
            .push("no reverse DNS or origin ASN records found".into());
    }
    for warning in &observation.warnings {
        context.job.log(format!("warning: {warning}"));
    }
    let geo = crate::geo::lookup_ip(context.settings.geo_db_path.as_deref(), *ip);
    let mut warnings = observation.warnings.clone();
    if let Some(warning) = geo.warning {
        warnings.push(warning);
    }
    Ok(CollectionOutput {
        warnings,
        batch: finish_batch(&observation, geo.location.as_ref()),
    })
}

async fn reverse_lookup(context: &CollectorContext, ip: IpAddr) -> Result<Vec<String>, String> {
    let result = tokio::select! {
        _ = context.job.cancel.cancelled() => return Err("cancelled".into()),
        result = context.net.dns.reverse_lookup(ip) => result,
    };
    match result {
        Ok(lookup) => Ok(lookup
            .answers()
            .iter()
            .filter_map(|record| match &record.data {
                RData::PTR(ptr) => {
                    Some(ptr.0.to_string().trim_end_matches('.').to_ascii_lowercase())
                }
                _ => None,
            })
            .collect()),
        Err(error) if error.is_nx_domain() || error.is_no_records_found() => Ok(Vec::new()),
        Err(error) => Err(format!("reverse DNS failed: {error}")),
    }
}

async fn origin_lookup(
    context: &CollectorContext,
    ip: IpAddr,
) -> Result<Option<CymruOrigin>, String> {
    let name = origin_name(ip);
    let txt = lookup_txt(context, &name).await?;
    Ok(txt
        .into_iter()
        .find_map(|record| parse_cymru_origin(&record)))
}

async fn asn_lookup(context: &CollectorContext, asn: u32) -> Result<Option<CymruAsn>, String> {
    let name = format!("AS{asn}.asn.cymru.com.");
    let txt = lookup_txt(context, &name).await?;
    Ok(txt.into_iter().find_map(|record| parse_cymru_asn(&record)))
}

async fn lookup_txt(context: &CollectorContext, name: &str) -> Result<Vec<String>, String> {
    let result = tokio::select! {
        _ = context.job.cancel.cancelled() => return Err("cancelled".into()),
        result = context.net.dns.lookup(name, RecordType::TXT) => result,
    };
    match result {
        Ok(lookup) => Ok(lookup
            .answers()
            .iter()
            .filter_map(|record| match &record.data {
                RData::TXT(txt) => Some(
                    txt.txt_data
                        .iter()
                        .map(|part| String::from_utf8_lossy(part).into_owned())
                        .collect::<String>(),
                ),
                _ => None,
            })
            .collect()),
        Err(error) if error.is_nx_domain() || error.is_no_records_found() => Ok(Vec::new()),
        Err(error) => Err(format!("Team Cymru lookup of {name} failed: {error}")),
    }
}

/// Team Cymru origin zone name for an IP.
pub fn origin_name(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            format!(
                "{}.{}.{}.{}.origin.asn.cymru.com.",
                octets[3], octets[2], octets[1], octets[0]
            )
        }
        IpAddr::V6(v6) => format!("{}.origin6.asn.cymru.com.", nibble_reverse(v6)),
    }
}

fn nibble_reverse(ip: Ipv6Addr) -> String {
    let mut nibbles = Vec::with_capacity(32);
    for byte in ip.octets() {
        nibbles.push(format!("{:x}", (byte >> 4) & 0x0f));
        nibbles.push(format!("{:x}", byte & 0x0f));
    }
    nibbles.reverse();
    nibbles.join(".")
}

fn fields(record: &str) -> Vec<String> {
    record
        .split('|')
        .map(|part| sanitize_line(part.trim()).into_owned())
        .collect()
}

/// Parses `13335 | 1.1.1.0/24 | AU | apnic | 2011-08-11`.
pub fn parse_cymru_origin(record: &str) -> Option<CymruOrigin> {
    let parts = fields(record);
    let asn: u32 = parts.first()?.parse().ok()?;
    let prefix = parts.get(1).filter(|value| !value.is_empty())?.clone();
    Some(CymruOrigin {
        asn,
        prefix,
        country: parts.get(2).filter(|value| !value.is_empty()).cloned(),
        registry: parts.get(3).filter(|value| !value.is_empty()).cloned(),
        allocated: parts.get(4).filter(|value| !value.is_empty()).cloned(),
    })
}

/// Parses `13335 | CLOUDFLARENET, US | US | arin |`.
pub fn parse_cymru_asn(record: &str) -> Option<CymruAsn> {
    let parts = fields(record);
    let asn: u32 = parts.first()?.parse().ok()?;
    let name = parts.get(1).filter(|value| !value.is_empty())?.clone();
    Some(CymruAsn {
        asn,
        name,
        country: parts.get(2).filter(|value| !value.is_empty()).cloned(),
        registry: parts.get(3).filter(|value| !value.is_empty()).cloned(),
    })
}

/// Builds the evidence batch. Pure and unit-tested.
pub fn build_batch(observation: &IpObservation) -> EvidenceBatch {
    finish_batch(observation, None)
}

fn finish_batch(
    observation: &IpObservation,
    geo: Option<&crate::model::GeoLocation>,
) -> EvidenceBatch {
    let mut batch = EvidenceBatch::new();
    let (content, fields) = describe(observation);
    let mut source = NewSource::collected(
        SourceKind::IpIntel,
        "ip",
        format!("IP intelligence for {}", observation.ip),
    );
    source.content_hash = Some(sha256_hex(content.as_bytes()));
    source.content = content;
    source.fields = fields;
    let key = batch.add_source(source);
    let ip = batch.entity(EntityKind::Ip, &observation.ip, Some(key));
    for name in &observation.ptr {
        let host = EntityKey::new(EntityKind::Hostname, name);
        batch.relate(&ip, relation::REVERSE_DNS, &host, Some(key));
    }
    if let Some(origin) = &observation.origin {
        let asn = EntityKey::new(EntityKind::Asn, &format!("AS{}", origin.asn));
        batch.relate(&ip, relation::ANNOUNCED_BY, &asn, Some(key));
        if let Some(description) = &observation.asn {
            batch.label(&asn, description.name.clone());
            batch.annotate(
                &asn,
                "country",
                description.country.clone().unwrap_or_default(),
            );
        }
        batch.annotate(&ip, "prefix", origin.prefix.clone());
        if let Some(country) = origin
            .country
            .as_deref()
            .and_then(crate::model::normalize_country)
        {
            batch.annotate(&ip, "country", country);
        }
        batch.annotate(&asn, "prefix", origin.prefix.clone());
        if let Some(allocated) = origin
            .allocated
            .as_deref()
            .and_then(crate::collectors::rdap::parse_date)
        {
            batch.event(
                allocated,
                TimelineKind::Registration,
                format!(
                    "{} allocated in {}",
                    origin.prefix,
                    origin.registry.as_deref().unwrap_or("unknown registry")
                ),
                Some(key),
                Some(ip.clone()),
            );
        }
    }
    apply_geo(&mut batch, &ip, observation, geo);
    batch.summary = summarize(observation);
    batch
}

fn apply_geo(
    batch: &mut EvidenceBatch,
    ip: &EntityKey,
    observation: &IpObservation,
    geo: Option<&crate::model::GeoLocation>,
) {
    if let Some(location) = geo
        && let Some(entity) = batch.entities.iter_mut().find(|entity| entity.key == *ip)
    {
        crate::model::annotate_geo_fields(
            &mut entity.fields,
            location,
            crate::model::Provenance::Collected,
        );
    }
    let country = batch
        .entities
        .iter()
        .find(|entity| entity.key == *ip)
        .and_then(|entity| entity.fields.get("country").map(ToString::to_string))
        .or_else(|| {
            observation
                .origin
                .as_ref()
                .and_then(|origin| origin.country.clone())
        });
    let has_point = batch
        .entities
        .iter()
        .find(|entity| entity.key == *ip)
        .and_then(|entity| crate::model::point_from_fields(&entity.fields))
        .is_some();
    if !has_point
        && let Some(country) = country.as_deref().and_then(crate::model::normalize_country)
        && let Some(point) = crate::model::country_centroid(&country)
    {
        batch.annotate(ip, "latitude", format!("{:.6}", point.latitude));
        batch.annotate(ip, "longitude", format!("{:.6}", point.longitude));
        batch.annotate(ip, "geo_source", "country_centroid");
    }
    if let Some(origin) = &observation.origin {
        let asn = crate::model::EntityKey::new(
            crate::model::EntityKind::Asn,
            &format!("AS{}", origin.asn),
        );
        let country = observation
            .asn
            .as_ref()
            .and_then(|description| description.country.clone())
            .or_else(|| origin.country.clone())
            .as_deref()
            .and_then(crate::model::normalize_country);
        if let Some(country) = country {
            batch.annotate(&asn, "country", country.clone());
            let has_point = batch
                .entities
                .iter()
                .find(|entity| entity.key == asn)
                .and_then(|entity| crate::model::point_from_fields(&entity.fields))
                .is_some();
            if !has_point && let Some(point) = crate::model::country_centroid(&country) {
                batch.annotate(&asn, "latitude", format!("{:.6}", point.latitude));
                batch.annotate(&asn, "longitude", format!("{:.6}", point.longitude));
                batch.annotate(&asn, "geo_source", "country_centroid");
            }
        }
    }
}

fn describe(observation: &IpObservation) -> (String, Fields) {
    let mut lines = vec![format!("Address: {}", observation.ip)];
    let mut fields = Fields::new();
    if observation.ptr.is_empty() {
        lines.push("Reverse DNS: (none)".into());
    } else {
        lines.push(format!("Reverse DNS: {}", observation.ptr.join(", ")));
        fields.push("ptr", observation.ptr.join(", "));
    }
    if let Some(origin) = &observation.origin {
        lines.push(format!("Origin ASN: AS{}", origin.asn));
        lines.push(format!("Prefix: {}", origin.prefix));
        lines.push(format!(
            "Country: {}",
            origin.country.as_deref().unwrap_or("unknown")
        ));
        lines.push(format!(
            "Registry: {}",
            origin.registry.as_deref().unwrap_or("unknown")
        ));
        lines.push(format!(
            "Allocated: {}",
            origin.allocated.as_deref().unwrap_or("unknown")
        ));
        fields.push("asn", format!("AS{}", origin.asn));
        fields.push("prefix", origin.prefix.clone());
        fields.push_opt("country", origin.country.clone());
        fields.push_opt("registry", origin.registry.clone());
        fields.push_opt("allocated", origin.allocated.clone());
    }
    if let Some(asn) = &observation.asn {
        lines.push(format!("AS name: {}", asn.name));
        fields.push("as_name", asn.name.clone());
    }
    lines.push(String::new());
    lines.push("Sources: system reverse DNS; Team Cymru DNS (origin.asn.cymru.com).".into());
    (lines.join("\n"), fields)
}

fn summarize(observation: &IpObservation) -> String {
    match (
        &observation.origin,
        &observation.asn,
        observation.ptr.first(),
    ) {
        (Some(origin), Some(asn), Some(ptr)) => {
            format!(
                "AS{} {} announcing {}; PTR {ptr}",
                origin.asn, asn.name, origin.prefix
            )
        }
        (Some(origin), Some(asn), None) => {
            format!("AS{} {} announcing {}", origin.asn, asn.name, origin.prefix)
        }
        (Some(origin), None, _) => format!("AS{} announcing {}", origin.asn, origin.prefix),
        (None, _, Some(ptr)) => format!("PTR {ptr}"),
        _ => "no origin ASN or reverse DNS".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_names_are_reversed() {
        let v4: IpAddr = "192.0.2.77".parse().expect("ip");
        let v6: IpAddr = "2001:db8::1".parse().expect("ip");
        assert_eq!(origin_name(v4), "77.2.0.192.origin.asn.cymru.com.");
        assert!(origin_name(v6).starts_with(
            "1.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.8.b.d.0.1.0.0.2.origin6"
        ));
    }

    #[test]
    fn parses_cymru_records() {
        let origin =
            parse_cymru_origin("13335 | 1.1.1.0/24 | AU | apnic | 2011-08-11").expect("origin");
        assert_eq!(origin.asn, 13335);
        assert_eq!(origin.prefix, "1.1.1.0/24");
        assert_eq!(origin.country.as_deref(), Some("AU"));
        let asn = parse_cymru_asn("13335 | CLOUDFLARENET, US | US | arin |").expect("asn");
        assert_eq!(asn.name, "CLOUDFLARENET, US");
        assert!(parse_cymru_origin("not a record").is_none());
    }

    #[test]
    fn batch_links_ptr_and_asn() {
        let input = IpObservation {
            ip: "1.1.1.1".into(),
            ptr: vec!["one.one.one.one".into()],
            origin: Some(CymruOrigin {
                asn: 13335,
                prefix: "1.1.1.0/24".into(),
                country: Some("AU".into()),
                registry: Some("apnic".into()),
                allocated: Some("2011-08-11".into()),
            }),
            asn: Some(CymruAsn {
                asn: 13335,
                name: "CLOUDFLARENET".into(),
                country: Some("US".into()),
                registry: Some("arin".into()),
            }),
            warnings: Vec::new(),
        };
        let actual = build_batch(&input);
        assert!(actual.relationships.iter().any(|relationship| {
            relationship.kind == "reverse_dns" && relationship.to.value == "one.one.one.one"
        }));
        assert!(actual.relationships.iter().any(|relationship| {
            relationship.kind == "announced_by" && relationship.to.value == "AS13335"
        }));
        assert_eq!(actual.timeline.len(), 1);
        assert!(actual.summary.contains("CLOUDFLARENET"));
        let ip = actual
            .entities
            .iter()
            .find(|entity| entity.key.kind == EntityKind::Ip)
            .expect("ip");
        assert_eq!(ip.fields.get("country"), Some("AU"));
        assert_eq!(ip.fields.get("geo_source"), Some("country_centroid"));
        assert!(ip.fields.get("latitude").is_some());
        let asn = actual
            .entities
            .iter()
            .find(|entity| entity.key.kind == EntityKind::Asn)
            .expect("asn");
        assert_eq!(asn.fields.get("country"), Some("US"));
        assert_eq!(asn.fields.get("prefix"), Some("1.1.1.0/24"));
    }
}
