use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::common::{Confidence, Provenance, Timestamp};
use super::ids::{RefKind, ShortRef};

/// Relationship kinds produced by built-in collectors. Any snake_case verb phrase is allowed.
pub mod kinds {
    pub const RESOLVES_TO: &str = "resolves_to";
    pub const ALIAS_OF: &str = "alias_of";
    pub const USES_NAMESERVER: &str = "uses_nameserver";
    pub const MAIL_HANDLED_BY: &str = "mail_handled_by";
    pub const SPF_INCLUDES: &str = "spf_includes";
    pub const REVERSE_DNS: &str = "reverse_dns";
    pub const ANNOUNCED_BY: &str = "announced_by";
    pub const OPERATED_BY: &str = "operated_by";
    pub const REGISTERED_WITH: &str = "registered_with";
    pub const REGISTERED_TO: &str = "registered_to";
    pub const REDIRECTS_TO: &str = "redirects_to";
    pub const SERVES: &str = "serves";
    pub const LINKS_TO: &str = "links_to";
    pub const MENTIONS: &str = "mentions";
    pub const PRESENTS_CERTIFICATE: &str = "presents_certificate";
    pub const ISSUED_BY: &str = "issued_by";
    pub const COVERS: &str = "covers";
    pub const SUBDOMAIN_OF: &str = "subdomain_of";
    pub const REFERENCES: &str = "references";
    pub const CONTRIBUTED_TO: &str = "contributed_to";
    pub const HOSTED_ON: &str = "hosted_on";
    pub const ASSOCIATED_WITH: &str = "associated_with";
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
