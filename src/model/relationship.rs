//! Directed links between entities.
//!
//! Built-in verbs live in [`kinds`]. Other snake_case phrases are accepted from the analyst
//! and from agent `LINK_ENTITY` proposals. A relationship can cite the sources that support it.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::common::{Confidence, Provenance, Timestamp};
use super::ids::{RefKind, ShortRef};

/// Relationship kinds produced by built-in collectors.
///
/// Any other snake_case verb phrase is stored as-is when an analyst or an agent links two
/// entities. The strings here are the ones collectors write, so reports and the entity pane
/// stay stable.
pub mod kinds {
    /// A name resolved to an address (`A` / `AAAA`, or a host seen over HTTP).
    pub const RESOLVES_TO: &str = "resolves_to";
    /// A CNAME, or another name for the same subject.
    pub const ALIAS_OF: &str = "alias_of";
    /// A domain's nameserver.
    pub const USES_NAMESERVER: &str = "uses_nameserver";
    /// The target of an MX record.
    pub const MAIL_HANDLED_BY: &str = "mail_handled_by";
    /// A domain named by an SPF include or redirect.
    pub const SPF_INCLUDES: &str = "spf_includes";
    /// The hostname in a pointer record for an address.
    pub const REVERSE_DNS: &str = "reverse_dns";
    /// The origin ASN for an address.
    pub const ANNOUNCED_BY: &str = "announced_by";
    /// A network operator, or the organization that publishes a repository.
    pub const OPERATED_BY: &str = "operated_by";
    /// The registrar named by RDAP.
    pub const REGISTERED_WITH: &str = "registered_with";
    /// The registrant named by RDAP, when the value is a domain.
    pub const REGISTERED_TO: &str = "registered_to";
    /// One HTTP hop to the next URL.
    pub const REDIRECTS_TO: &str = "redirects_to";
    /// The final URL a collected page was served from.
    pub const SERVES: &str = "serves";
    /// A hostname linked from a collected page.
    pub const LINKS_TO: &str = "links_to";
    /// An email address or other identifier mentioned in collected text.
    pub const MENTIONS: &str = "mentions";
    /// The certificate a TLS handshake presented.
    pub const PRESENTS_CERTIFICATE: &str = "presents_certificate";
    /// The issuer of a certificate.
    pub const ISSUED_BY: &str = "issued_by";
    /// A name listed on a certificate.
    pub const COVERS: &str = "covers";
    /// A discovered hostname beneath the name that was collected.
    pub const SUBDOMAIN_OF: &str = "subdomain_of";
    /// A URL or host a page, security.txt policy, or repository points at.
    pub const REFERENCES: &str = "references";
    /// An email address that committed to a repository.
    pub const CONTRIBUTED_TO: &str = "contributed_to";
    /// An email address at a domain.
    pub const HOSTED_ON: &str = "hosted_on";
    /// Fallback when a relationship name normalizes to an empty string.
    pub const ASSOCIATED_WITH: &str = "associated_with";
    /// Reserved verb for a container relationship. Built-in collectors do not write it.
    pub const CONTAINS: &str = "contains";
}

/// A directed, typed edge between two entities, optionally supported by sources.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Relationship {
    pub id: Uuid,
    pub investigation_id: Uuid,
    pub seq: i64,
    pub from_id: Uuid,
    pub to_id: Uuid,
    pub kind: String,
    pub provenance: Provenance,
    pub confidence: Confidence,
    pub notes: String,
    pub source_ids: Vec<Uuid>,
    pub created_at: Timestamp,
}

impl Relationship {
    pub fn short_ref(&self) -> ShortRef {
        ShortRef::new(RefKind::Relationship, self.seq)
    }
}

/// Normalizes free-form relationship names to `snake_case` (`Resolves To` -> `resolves_to`).
pub fn normalize_relationship_kind(raw: &str) -> String {
    let mut output = String::with_capacity(raw.len());
    for c in raw.trim().chars() {
        if c.is_ascii_alphanumeric() {
            output.push(c.to_ascii_lowercase());
        } else if !output.ends_with('_') && !output.is_empty() {
            output.push('_');
        }
    }
    let trimmed = output.trim_end_matches('_').to_string();
    if trimmed.is_empty() {
        kinds::ASSOCIATED_WITH.to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relationship_kinds_are_snake_cased() {
        assert_eq!(normalize_relationship_kind("Resolves To"), "resolves_to");
        assert_eq!(
            normalize_relationship_kind("--uses-nameserver-->"),
            "uses_nameserver"
        );
        assert_eq!(normalize_relationship_kind("  "), "associated_with");
    }
}
