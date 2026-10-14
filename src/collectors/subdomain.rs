//! Subdomain expansion: catalogued `subdomain_of` / `resolves_to` under a configured cap.
//!
//! Names come from optional passive tools (`subfinder`, `amass -passive`) plus a small set of
//! common prefixes. Each name is DNS-resolved (A/AAAA) until the cap is reached.

use std::collections::BTreeSet;
use std::time::Duration;

use futures::future::BoxFuture;
use hickory_resolver::proto::rr::{RData, RecordType};

use super::tool::run_tool;
use super::{CollectResult, CollectionOutput, Collector, CollectorContext};
use crate::files::sha256_hex;
use crate::model::{
    EntityKind, EvidenceBatch, NewSource, SourceKind, Target, TargetKind, is_subdomain_of, relation,
};
use crate::sanitize::sanitize_text;
use crate::tools::{Activity, OutputParser};

const COMMON_PREFIXES: [&str; 10] = [
    "www",
    "mail",
    "ns1",
    "ns2",
    "ftp",
    "api",
    "dev",
    "staging",
    "vpn",
    "autodiscover",
];

pub struct SubdomainCollector;

impl Collector for SubdomainCollector {
    fn id(&self) -> &str {
        "subdomain"
    }

    fn label(&self) -> &str {
        "Subdomains"
    }

    fn description(&self) -> &str {
        "Subdomain expansion (passive tools + DNS) with subdomain_of / resolves_to"
    }

    fn accepts(&self, kind: TargetKind) -> bool {
        kind == TargetKind::Domain
    }

    fn collect<'a>(
        &'a self,
        target: &'a Target,
        context: &'a CollectorContext,
    ) -> BoxFuture<'a, CollectResult> {
        Box::pin(collect(target, context))
    }
}

/// One expanded name and any addresses it resolved to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubdomainHit {
    pub hostname: String,
    pub addresses: Vec<String>,
}

async fn collect(target: &Target, context: &CollectorContext) -> CollectResult {
    let Target::Domain(apex) = target else {
        return Err(format!(
            "the subdomain collector does not accept {}",
            target.kind()
        ));
    };
    let cap = context.settings.max_subdomains.max(1) as usize;
    let mut names: BTreeSet<String> = BTreeSet::new();
    let mut warnings = Vec::new();
    for prefix in COMMON_PREFIXES {
        names.insert(format!("{prefix}.{apex}"));
    }
    for (tool_name, args) in [
        ("subfinder", vec!["-silent", "-d", apex.as_str()]),
        (
            "amass",
            vec!["enum", "-passive", "-silent", "-d", apex.as_str()],
        ),
    ] {
        if context.job.cancel.is_cancelled() {
            return Err("cancelled".into());
        }
        match run_named_tool(context, target, tool_name, &args).await {
            Ok(Some(output)) => {
                for (kind, value) in
                    crate::collectors::tool::extract_values(OutputParser::Hostnames, &output)
                {
                    if matches!(kind, EntityKind::Hostname | EntityKind::Domain)
                        && is_subdomain_of(&value, apex)
                    {
                        names.insert(value);
                    }
                }
            }
            Ok(None) => {}
            Err(error) => warnings.push(error),
        }
    }
    let mut hits = Vec::new();
    for hostname in names.into_iter().take(cap) {
        if context.job.cancel.is_cancelled() {
            return Err("cancelled".into());
        }
        context.job.progress(format!("Resolving {hostname}"));
        let mut addresses = Vec::new();
        for record_type in [RecordType::A, RecordType::AAAA] {
            match lookup_addresses(context, &hostname, record_type).await {
                Ok(found) => addresses.extend(found),
                Err(error) => warnings.push(format!("{hostname} {record_type}: {error}")),
            }
        }
        hits.push(SubdomainHit {
            hostname,
            addresses,
        });
    }
    let mut output = CollectionOutput::new(build_batch(target, &hits, cap));
    output.warnings = warnings;
    Ok(output)
}

async fn run_named_tool(
    context: &CollectorContext,
    target: &Target,
    name: &str,
    args: &[&str],
) -> Result<Option<String>, String> {
    let Some(entry) = context.tools.find(name).cloned() else {
        return Ok(None);
    };
    if !entry.is_available() {
        return Ok(None);
    }
    let program = entry
        .path
        .clone()
        .ok_or_else(|| format!("{name} has no path"))?;
    let mut definition = entry.definition.clone();
    definition.args = args.iter().map(|arg| (*arg).to_string()).collect();
    definition.parser = OutputParser::Hostnames;
    definition.activity = Activity::Passive;
    definition.timeout = Duration::from_secs(120);
    let output = run_tool(&definition, program, target, &context.job).await?;
    let content = output
        .batch
        .sources
        .first()
        .map(|source| source.content.clone())
        .unwrap_or_default();
    // extract_values works on stdout; recover it from the stored source body.
    let stdout = content
        .split_once("--- stdout ---\n")
        .map(|(_, rest)| {
            rest.split_once("\n\n--- stderr ---")
                .map(|(head, _)| head)
                .unwrap_or(rest)
        })
        .unwrap_or(&content)
        .to_string();
    Ok(Some(sanitize_text(&stdout).into_owned()))
}

async fn lookup_addresses(
    context: &CollectorContext,
    name: &str,
    kind: RecordType,
) -> Result<Vec<String>, String> {
    let fqdn = format!("{name}.");
    let result = tokio::select! {
        _ = context.job.cancel.cancelled() => return Err("cancelled".into()),
        result = context.net.dns.lookup(fqdn.as_str(), kind) => result,
    };
    match result {
        Ok(lookup) => Ok(lookup
            .answers()
            .iter()
            .filter_map(|record| match &record.data {
                RData::A(address) => Some(address.0.to_string()),
                RData::AAAA(address) => Some(address.0.to_string()),
                _ => None,
            })
            .collect()),
        Err(error) if error.is_nx_domain() || error.is_no_records_found() => Ok(Vec::new()),
        Err(error) => Err(error.to_string()),
    }
}

/// Builds `subdomain_of` / `resolves_to` edges. Pure and unit-tested.
pub fn build_batch(target: &Target, hits: &[SubdomainHit], cap: usize) -> EvidenceBatch {
    let mut batch = EvidenceBatch::new();
    let apex = target.value();
    let mut source = NewSource::collected(
        SourceKind::Dns,
        "subdomain",
        format!("Subdomain expansion for {apex}"),
    );
    let mut lines = vec![format!("apex: {apex}"), format!("cap: {cap}")];
    for hit in hits {
        lines.push(format!(
            "{} {}",
            hit.hostname,
            if hit.addresses.is_empty() {
                "unresolved".into()
            } else {
                hit.addresses.join(",")
            }
        ));
    }
    source.content = lines.join("\n");
    source.content_hash = Some(sha256_hex(source.content.as_bytes()));
    source.fields.push("cap", cap.to_string());
    source.fields.push("count", hits.len().to_string());
    let key = batch.add_source(source);
    let apex_key = batch.entity(EntityKind::Domain, &apex, Some(key));
    for hit in hits.iter().take(cap) {
        if !is_subdomain_of(&hit.hostname, &apex) {
            continue;
        }
        let host = batch.entity(EntityKind::Hostname, &hit.hostname, Some(key));
        batch.relate(&host, relation::SUBDOMAIN_OF, &apex_key, Some(key));
        for address in &hit.addresses {
            let ip = batch.entity(EntityKind::Ip, address, Some(key));
            batch.relate(&host, relation::RESOLVES_TO, &ip, Some(key));
        }
    }
    batch.summary = format!(
        "expanded {} subdomain(s) under cap {cap}",
        batch
            .relationships
            .iter()
            .filter(|relationship| relationship.kind == relation::SUBDOMAIN_OF)
            .count()
    );
    batch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expansion_uses_subdomain_of_and_resolves_to() {
        let target = Target::Domain("example.com".into());
        let hits = vec![
            SubdomainHit {
                hostname: "www.example.com".into(),
                addresses: vec!["192.0.2.10".into()],
            },
            SubdomainHit {
                hostname: "not-related.net".into(),
                addresses: vec!["192.0.2.11".into()],
            },
        ];
        let actual = build_batch(&target, &hits, 80);
        assert!(
            actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "subdomain_of"
                    && relationship.from.value == "www.example.com")
        );
        assert!(
            actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "resolves_to"
                    && relationship.to.value == "192.0.2.10")
        );
        assert!(
            !actual
                .relationships
                .iter()
                .any(|relationship| relationship.from.value == "not-related.net")
        );
        assert!(
            !actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "mentions")
        );
    }

    #[test]
    fn cap_truncates_hits() {
        let target = Target::Domain("example.com".into());
        let hits: Vec<SubdomainHit> = (0..5)
            .map(|i| SubdomainHit {
                hostname: format!("s{i}.example.com"),
                addresses: Vec::new(),
            })
            .collect();
        let actual = build_batch(&target, &hits, 2);
        let subdomains = actual
            .relationships
            .iter()
            .filter(|relationship| relationship.kind == "subdomain_of")
            .count();
        assert_eq!(subdomains, 2);
    }
}
