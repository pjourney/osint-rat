//! One case: the unit of listing, export, import, and the agent context pack.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::common::Timestamp;

/// The top-level container. Everything in os-int belongs to exactly one investigation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Investigation {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// An investigation plus record counts, for listings.
#[derive(Clone, Debug, PartialEq)]
pub struct InvestigationSummary {
    pub investigation: Investigation,
    pub page_count: i64,
    pub source_count: i64,
    pub finding_count: i64,
    pub entity_count: i64,
}
