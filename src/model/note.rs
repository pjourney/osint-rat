//! Standalone annotations.
//!
//! The notes you edit with `e` on Overview are stored on the page (`Page::notes`). A [`Note`]
//! row is a separate record with its own short id, used for notes you add from the palette and
//! for notes accepted from an agent.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::common::{Provenance, Timestamp};
use super::ids::{RefKind, ShortRef};

/// A discrete annotation. Page notes live on the page itself; `Note` records are standalone
/// entries with their own ID and provenance (for example an accepted agent suggestion).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: Uuid,
    pub investigation_id: Uuid,
    pub seq: i64,
    pub page_id: Option<Uuid>,
    pub body: String,
    pub provenance: Provenance,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Note {
    pub fn short_ref(&self) -> ShortRef {
        ShortRef::new(RefKind::Note, self.seq)
    }

    /// The first non-empty line, used as a title in lists.
    pub fn headline(&self) -> &str {
        self.body
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("(empty note)")
    }
}
