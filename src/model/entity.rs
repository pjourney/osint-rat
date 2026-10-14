//! Normalized identifiers observed in a case.
//!
//! Entities are upserted by kind plus canonical value, so `Example.COM.` and `example.com` share
//! a row. A page's subject is usually also an entity. Nameservers, addresses, and certificate
//! names become entities even when you never created a page for them.

use std::net::IpAddr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::common::{Fields, Provenance, Timestamp, open_enum};
use super::ids::{RefKind, ShortRef};
use super::marking::{Marking, OBJECT_MARKING_FIELD};
use super::target::{normalize_email, normalize_hostname, normalize_url, parse_asn};

open_enum! {
    /// The type of a real-world thing observed during an investigation.
    pub enum EntityKind {
        Person => ("person", "Person"),
        Organization => ("organization", "Organization", "org", "company"),
        Domain => ("domain", "Domain"),
        Hostname => ("hostname", "Hostname", "host", "nameserver"),
        Ip => ("ip", "IP", "ip_address", "ipv4", "ipv6"),
        Email => ("email", "Email"),
        Username => ("username", "Username", "handle"),
        Url => ("url", "URL"),
        Repository => ("repository", "Repository", "repo"),
        File => ("file", "File"),
        Location => ("location", "Location", "place"),
        Phone => ("phone", "Phone", "telephone"),
        Asn => ("asn", "ASN", "as"),
        Certificate => ("certificate", "Certificate", "cert"),
        Unknown => ("unknown", "Unknown"),
    }
}

impl EntityKind {
    /// Converts an identifier into its canonical form so duplicates merge.
    /// Values that fail strict validation are kept trimmed rather than rejected.
    pub fn normalize_value(&self, raw: &str) -> String {
        let trimmed = raw.trim();
        let normalized = match self {
            EntityKind::Domain | EntityKind::Hostname => normalize_hostname(trimmed).ok(),
            EntityKind::Email => normalize_email(trimmed).ok(),
            EntityKind::Ip => trimmed
                .trim_matches(['[', ']'])
                .parse::<IpAddr>()
                .ok()
                .map(|ip| ip.to_string()),
            EntityKind::Url => normalize_url(trimmed).ok().map(|url| url.to_string()),
            EntityKind::Asn => parse_asn(trimmed).ok().map(|number| format!("AS{number}")),
            EntityKind::Username => Some(trimmed.trim_start_matches('@').to_string()),
            EntityKind::Certificate => Some(trimmed.to_ascii_lowercase().replace(':', "")),
            _ => None,
        };
        normalized.unwrap_or_else(|| trimmed.to_string())
    }
}

/// A person, organization, domain, IP, ... observed in the investigation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub id: Uuid,
    pub investigation_id: Uuid,
    pub seq: i64,
    pub kind: EntityKind,
    /// Canonical value (unique per investigation and kind).
    pub value: String,
    /// Optional friendlier display name (an organization's legal name, a person's handle).
    pub label: String,
    pub notes: String,
    pub provenance: Provenance,
    pub fields: Fields,
    pub source_ids: Vec<Uuid>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Entity {
    pub fn short_ref(&self) -> ShortRef {
        ShortRef::new(RefKind::Entity, self.seq)
    }

    pub fn display_name(&self) -> &str {
        if self.label.trim().is_empty() {
            &self.value
        } else {
            &self.label
        }
    }

    /// Optional object-level TLP stored in the property bag. Unmarked when absent.
    pub fn marking(&self) -> Marking {
        self.fields
            .get(OBJECT_MARKING_FIELD)
            .and_then(|value| Marking::parse(value).ok())
            .unwrap_or(Marking::Unmarked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_identifiers_per_kind() {
        assert_eq!(
            EntityKind::Domain.normalize_value("WWW.Example.com."),
            "www.example.com"
        );
        assert_eq!(
            EntityKind::Ip.normalize_value("2001:DB8::0001"),
            "2001:db8::1"
        );
        assert_eq!(EntityKind::Asn.normalize_value("13335"), "AS13335");
        assert_eq!(EntityKind::Username.normalize_value("@octocat"), "octocat");
        assert_eq!(EntityKind::Certificate.normalize_value("AB:CD"), "abcd");
        assert_eq!(
            EntityKind::Person.normalize_value("  Alice Smith "),
            "Alice Smith"
        );
    }
}
