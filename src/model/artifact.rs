use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::common::{Fields, Timestamp, open_enum};
use super::ids::{RefKind, ShortRef};

open_enum! {
    /// What an artifact file is.
    pub enum ArtifactKind {
        Snapshot => ("snapshot", "Snapshot"),
        File => ("file", "Evidence file", "evidence"),
        Report => ("report", "Report"),
        AgentContext => ("agent_context", "Agent context pack", "context"),
        ToolOutput => ("tool_output", "Tool output"),
        Repository => ("repository", "Repository clone", "clone"),
    }
}

/// A file stored in the investigation directory and referenced from the database.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Artifact {
    pub id: Uuid,
    pub investigation_id: Uuid,
    pub seq: i64,
    pub page_id: Option<Uuid>,
    pub kind: ArtifactKind,
    pub name: String,
    /// Path relative to the investigation directory.
    pub path: String,
    pub mime: Option<String>,
    pub size_bytes: i64,
    pub sha256: Option<String>,
    pub fields: Fields,
    pub source_id: Option<Uuid>,
    pub created_at: Timestamp,
}

impl Artifact {
    pub fn short_ref(&self) -> ShortRef {
        ShortRef::new(RefKind::Artifact, self.seq)
    }
}
