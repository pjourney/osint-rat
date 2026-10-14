//! Append-only governance log. Distinct from the investigative timeline.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::common::{Fields, Timestamp, now};
use super::identity::Identity;
use super::ids::{RefKind, ShortRef};

/// Kind of a governance event. Stored as a snake_case key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditKind {
    EntitiesMerged,
    EntitiesUnresolved,
    ActionAccepted,
    PlaybookRun,
    Export,
    MarkingChanged,
    MemberAdded,
    MemberRoleChanged,
    MemberRemoved,
}

impl AuditKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AuditKind::EntitiesMerged => "entities_merged",
            AuditKind::EntitiesUnresolved => "entities_unresolved",
            AuditKind::ActionAccepted => "action_accepted",
            AuditKind::PlaybookRun => "playbook_run",
            AuditKind::Export => "export",
            AuditKind::MarkingChanged => "marking_changed",
            AuditKind::MemberAdded => "member_added",
            AuditKind::MemberRoleChanged => "member_role_changed",
            AuditKind::MemberRemoved => "member_removed",
        }
    }

    pub fn parse(value: &str) -> Result<AuditKind, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "entities_merged" | "merged" | "merge" => Ok(AuditKind::EntitiesMerged),
            "entities_unresolved" | "unresolved" | "unresolve" => Ok(AuditKind::EntitiesUnresolved),
            "action_accepted" | "accept" | "accepted" => Ok(AuditKind::ActionAccepted),
            "playbook_run" | "playbook" => Ok(AuditKind::PlaybookRun),
            "export" => Ok(AuditKind::Export),
            "marking_changed" | "marking" => Ok(AuditKind::MarkingChanged),
            "member_added" => Ok(AuditKind::MemberAdded),
            "member_role_changed" | "role_changed" => Ok(AuditKind::MemberRoleChanged),
            "member_removed" => Ok(AuditKind::MemberRemoved),
            other => Err(format!("unknown audit kind {other:?}")),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AuditKind::EntitiesMerged => "merged",
            AuditKind::EntitiesUnresolved => "unresolved",
            AuditKind::ActionAccepted => "accepted Action",
            AuditKind::PlaybookRun => "playbook",
            AuditKind::Export => "export",
            AuditKind::MarkingChanged => "marking",
            AuditKind::MemberAdded => "member added",
            AuditKind::MemberRoleChanged => "role changed",
            AuditKind::MemberRemoved => "member removed",
        }
    }
}

/// One append-only audit row. Never updated after insert.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: Uuid,
    pub investigation_id: Uuid,
    pub seq: i64,
    pub kind: AuditKind,
    pub actor: String,
    pub machine: String,
    pub occurred_at: Timestamp,
    pub summary: String,
    #[serde(default)]
    pub details: Fields,
    #[serde(default)]
    pub target_ids: Vec<Uuid>,
}

impl AuditEntry {
    pub fn short_ref(&self) -> ShortRef {
        ShortRef::new(RefKind::Audit, self.seq)
    }

    pub fn new(
        investigation_id: Uuid,
        seq: i64,
        identity: &Identity,
        kind: AuditKind,
        summary: impl Into<String>,
        details: Fields,
        target_ids: Vec<Uuid>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            investigation_id,
            seq,
            kind,
            actor: identity.user_id.clone(),
            machine: identity.machine_name.clone(),
            occurred_at: now(),
            summary: summary.into(),
            details,
            target_ids,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_round_trips() {
        for kind in [
            AuditKind::EntitiesMerged,
            AuditKind::ActionAccepted,
            AuditKind::Export,
            AuditKind::MarkingChanged,
        ] {
            assert_eq!(AuditKind::parse(kind.as_str()).expect("parse"), kind);
        }
    }
}
