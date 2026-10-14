//! Email enrichment pack: hosted domain, local-part username, and cited graph edges.
//!
//! Pure from the address itself so `os-int collect email` yields useful edges without custom
//! tools and without a network call. DNS and CT remain the other default collectors for email.

use futures::future::BoxFuture;

use super::{CollectResult, CollectionOutput, Collector, CollectorContext};
use crate::files::sha256_hex;
use crate::model::{
    EntityKind, EvidenceBatch, NewSource, SourceKind, Target, TargetKind, relation,
};

pub struct EmailCollector;

impl Collector for EmailCollector {
    fn id(&self) -> &str {
        "email"
    }

    fn label(&self) -> &str {
        "Email"
    }

    fn description(&self) -> &str {
        "Email identity pack: hosted domain, local-part username, and mailbox graph edges"
    }

    fn accepts(&self, kind: TargetKind) -> bool {
        kind == TargetKind::Email
    }

    fn needs_network(&self) -> bool {
        false
    }

    fn collect<'a>(
        &'a self,
        target: &'a Target,
        _context: &'a CollectorContext,
    ) -> BoxFuture<'a, CollectResult> {
        Box::pin(async move { Ok(CollectionOutput::new(build_batch(target))) })
    }
}

/// Builds the email identity graph from the address. Pure and unit-tested.
pub fn build_batch(target: &Target) -> EvidenceBatch {
    let mut batch = EvidenceBatch::new();
    let email = target.value();
    let mut source = NewSource::collected(
        SourceKind::Manual,
        "email",
        format!("Email identity for {email}"),
    );
    let (local, domain) = email
        .rsplit_once('@')
        .map(|(local, domain)| (local.to_string(), domain.to_string()))
        .unwrap_or_else(|| (email.clone(), String::new()));
    source.content = format!("email: {email}\nlocal: {local}\ndomain: {domain}");
    source.content_hash = Some(sha256_hex(source.content.as_bytes()));
    source.fields.push("email", email.clone());
    source.fields.push("local_part", local.clone());
    source
        .fields
        .push_opt("domain", Some(domain.clone()).filter(|d| !d.is_empty()));
    let key = batch.add_source(source);
    let subject = batch.entity(EntityKind::Email, &email, Some(key));
    if !domain.is_empty() {
        let domain_key = batch.entity(EntityKind::Domain, &domain, Some(key));
        batch.relate(&subject, relation::HOSTED_ON, &domain_key, Some(key));
    }
    if crate::model::Target::parse(TargetKind::Username, &local).is_ok() {
        let username = batch.entity(EntityKind::Username, &local, Some(key));
        batch.relate(&subject, relation::ASSOCIATED_WITH, &username, Some(key));
    }
    batch.summary = format!("{email} hosted on {domain}");
    batch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_pack_links_domain_and_username() {
        let target = Target::Email("alice.j@example.com".into());
        let actual = build_batch(&target);
        assert!(
            actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "hosted_on"
                    && relationship.to.value == "example.com")
        );
        assert!(
            actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "associated_with"
                    && relationship.to.value == "alice.j")
        );
        assert_eq!(actual.sources[0].collector.as_deref(), Some("email"));
        assert!(
            !actual.sources[0]
                .content_hash
                .as_deref()
                .unwrap_or("")
                .is_empty()
        );
        assert!(
            !actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == relation::SERVES)
        );
        assert!(
            !actual
                .entities
                .iter()
                .any(|entity| entity.key.kind == EntityKind::Url)
        );
    }
}
