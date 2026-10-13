use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::common::{Confidence, Provenance, Timestamp};
use super::ids::{RefKind, ShortRef};

/// A conclusion drawn from evidence. Findings cite the sources that support them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub id: Uuid,
    pub investigation_id: Uuid,
    pub seq: i64,
    pub page_id: Option<Uuid>,
    /// The claim itself, as one sentence.
    pub title: String,
    pub details: String,
    pub confidence: Confidence,
    /// `Analyst` or `Agent`. AI findings stay labelled as AI even after acceptance.
    pub provenance: Provenance,
    pub source_ids: Vec<Uuid>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Finding {
    pub fn short_ref(&self) -> ShortRef {
        ShortRef::new(RefKind::Finding, self.seq)
    }

    /// A finding without cited sources is a hypothesis, not evidence.
    pub fn is_unsupported(&self) -> bool {
        self.source_ids.is_empty()
    }
}
