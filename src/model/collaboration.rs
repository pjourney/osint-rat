//! Local-first team roles for a shared data directory. Not a cloud ACL.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::common::Timestamp;

/// How concurrent writers are reconciled. Documented in `docs/collaboration.md`.
pub const CONFLICT_POLICY: &str = "last-write-wins-with-audit";

/// ASCII case-insensitive `user_id` match (`Pat` and `pat` are the same person).
pub fn same_user(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

/// Role on one investigation. Empty membership means single-user owner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    #[default]
    Owner,
    Analyst,
    Reader,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Analyst => "analyst",
            Role::Reader => "reader",
        }
    }

    pub fn parse(value: &str) -> Result<Role, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "owner" => Ok(Role::Owner),
            "analyst" | "writer" => Ok(Role::Analyst),
            "reader" | "read" | "readonly" | "read-only" => Ok(Role::Reader),
            other => Err(format!(
                "unknown role {other:?}; use owner, analyst, or reader"
            )),
        }
    }

    pub fn can_write(self) -> bool {
        matches!(self, Role::Owner | Role::Analyst)
    }

    pub fn can_manage_team(self) -> bool {
        matches!(self, Role::Owner)
    }

    pub fn can_set_marking(self) -> bool {
        matches!(self, Role::Owner)
    }
}

/// A person listed on an investigation. `user_id` matches `[identity] user_id`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub investigation_id: Uuid,
    pub user_id: String,
    pub role: Role,
    pub added_by: String,
    pub added_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_cannot_write() {
        assert!(Role::Owner.can_write());
        assert!(Role::Analyst.can_write());
        assert!(!Role::Reader.can_write());
        assert!(Role::Owner.can_manage_team());
        assert!(!Role::Analyst.can_manage_team());
        assert!(!Role::Reader.can_set_marking());
    }

    #[test]
    fn parses_roles() {
        assert_eq!(Role::parse("Analyst").expect("role"), Role::Analyst);
        assert_eq!(Role::parse("read-only").expect("role"), Role::Reader);
        assert!(Role::parse("admin").is_err());
    }

    #[test]
    fn user_ids_match_case_insensitively() {
        assert!(same_user("Pat", "pat"));
        assert!(!same_user("pat", "patricia"));
    }
}
