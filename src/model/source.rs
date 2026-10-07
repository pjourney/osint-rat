use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::common::{Fields, Provenance, Timestamp, open_enum};
use super::ids::{RefKind, ShortRef};

open_enum! {
    /// What kind of material a source is.
    pub enum SourceKind {
        Dns => ("dns", "DNS"),
        Rdap => ("rdap", "RDAP", "whois"),
        Http => ("http", "HTTP"),
        Tls => ("tls", "TLS certificate", "certificate"),
        IpIntel => ("ip_intel", "IP intelligence", "asn"),
        Repository => ("repository", "Repository", "git"),
        File => ("file", "File"),
        Command => ("command", "Tool output", "tool"),
        Web => ("web", "Web page", "page"),
        Document => ("document", "Document"),
        Manual => ("manual", "Manual entry", "note"),
    }
}

/// A unit of provenance: where a piece of intelligence came from and when.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub id: Uuid,
    pub investigation_id: Uuid,
    pub seq: i64,
    pub kind: SourceKind,
    pub title: String,
    pub url: Option<String>,
    pub retrieved_at: Timestamp,
    /// Last time a collector observed byte-identical content (re-collection does not duplicate).
    pub last_seen_at: Option<Timestamp>,
    pub published_at: Option<Timestamp>,
    pub author: Option<String>,
    /// The collector or tool that produced this source (`dns`, `rdap`, `tool:whois`, ...).
    pub collector: Option<String>,
    /// SHA-256 of the raw snapshot, hex encoded.
    pub content_hash: Option<String>,
    /// Path of the raw snapshot, relative to the investigation directory.
    pub snapshot_path: Option<String>,
    /// Normalized, sanitized text extracted from the material (bounded in size).
    pub content: String,
    /// Structured metadata (headers, record values, certificate fields, ...).
    pub fields: Fields,
    pub notes: String,
    pub provenance: Provenance,
    pub page_id: Option<Uuid>,
    pub job_id: Option<Uuid>,
}

impl Source {
    pub fn short_ref(&self) -> ShortRef {
        ShortRef::new(RefKind::Source, self.seq)
    }
}
