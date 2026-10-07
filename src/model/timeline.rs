use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::common::{Provenance, Timestamp, open_enum};
use super::ids::{RefKind, ShortRef};

open_enum! {
    /// The category of a timeline event.
    pub enum TimelineKind {
        Registration => ("registration", "Registration"),
        Expiration => ("expiration", "Expiration"),
        Updated => ("updated", "Record updated", "last_changed"),
        CertificateIssued => ("certificate_issued", "Certificate issued"),
        CertificateExpires => ("certificate_expires", "Certificate expires"),
        Commit => ("commit", "Commit"),
        Release => ("release", "Release"),
        Published => ("published", "Published"),
        Observed => ("observed", "Observed", "collection"),
        UserEvent => ("user", "Analyst event", "manual"),
        Finding => ("finding", "Finding recorded"),
        Investigation => ("investigation", "Investigation activity"),
    }
}

/// Something that happened at a point in time, with optional links to evidence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub id: Uuid,
    pub investigation_id: Uuid,
    pub seq: i64,
    pub occurred_at: Timestamp,
    pub kind: TimelineKind,
    pub title: String,
    pub description: String,
    pub provenance: Provenance,
    pub source_id: Option<Uuid>,
    pub entity_id: Option<Uuid>,
    pub page_id: Option<Uuid>,
    pub created_at: Timestamp,
}

impl TimelineEvent {
    pub fn short_ref(&self) -> ShortRef {
        ShortRef::new(RefKind::Timeline, self.seq)
    }
}
