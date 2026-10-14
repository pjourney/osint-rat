//! Investigation members, roles, and write-guard checks.

use color_eyre::eyre::{Result, bail, eyre};
use rusqlite::{Connection, params};
use uuid::Uuid;

use super::audit::append_audit_on;
use super::rows::{RowExt, id, ts};
use super::{Store, touch_investigation};
use crate::model::{AuditKind, Fields, Identity, Member, Role, now, same_user};

pub(crate) fn list_members(
    connection: &Connection,
    investigation_id: Uuid,
) -> rusqlite::Result<Vec<Member>> {
    let mut statement = connection.prepare(
        "SELECT investigation_id, user_id, role, added_by, added_at
         FROM investigation_members WHERE investigation_id = ?1 ORDER BY added_at, user_id",
    )?;
    let rows = statement.query_map(params![id(&investigation_id)], member_from_row)?;
    rows.collect()
}

fn member_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Member> {
    let role_key: String = row.get(2)?;
    let role = Role::parse(&role_key).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(error)),
        )
    })?;
    Ok(Member {
        investigation_id: row.uuid(0)?,
        user_id: row.get(1)?,
        role,
        added_by: row.get(3)?,
        added_at: row.timestamp(4)?,
    })
}

pub(crate) fn insert_member_record(
    connection: &Connection,
    member: &Member,
) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO investigation_members (investigation_id, user_id, role, added_by, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            id(&member.investigation_id),
            member.user_id,
            member.role.as_str(),
            member.added_by,
            ts(&member.added_at),
        ],
    )?;
    Ok(())
}

fn role_from_members(members: &[Member], user_id: &str) -> Role {
    if members.is_empty() {
        return Role::Owner;
    }
    members
        .iter()
        .find(|member| same_user(&member.user_id, user_id))
        .map(|member| member.role)
        .unwrap_or(Role::Reader)
}

fn stored_user_id<'a>(members: &'a [Member], user_id: &str) -> Option<&'a str> {
    members
        .iter()
        .find(|member| same_user(&member.user_id, user_id))
        .map(|member| member.user_id.as_str())
}

impl Store {
    pub fn list_members(&self, investigation_id: Uuid) -> Result<Vec<Member>> {
        Ok(list_members(self.connection(), investigation_id)?)
    }

    pub fn role_for(&self, investigation_id: Uuid) -> Result<Role> {
        if self.get_investigation(investigation_id)?.is_none() {
            bail!("investigation {investigation_id} does not exist");
        }
        let members = list_members(self.connection(), investigation_id)?;
        Ok(role_from_members(&members, &self.identity.user_id))
    }

    pub fn assert_can_mutate(&self, investigation_id: Uuid) -> Result<()> {
        let role = self.role_for(investigation_id)?;
        if !role.can_write() {
            bail!("role {} cannot modify this investigation", role.as_str());
        }
        Ok(())
    }

    pub fn assert_can_mutate_record(&self, table: &'static str, row_id: Uuid) -> Result<()> {
        match super::investigation_id_of(self.connection(), table, row_id) {
            Ok(investigation_id) => self.assert_can_mutate(investigation_id),
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                bail!("unknown {table} record {row_id}")
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn assert_can_manage_team(&self, investigation_id: Uuid) -> Result<()> {
        let role = self.role_for(investigation_id)?;
        if !role.can_manage_team() {
            bail!("only the owner can change team membership");
        }
        Ok(())
    }

    pub fn assert_can_set_marking(&self, investigation_id: Uuid) -> Result<()> {
        let role = self.role_for(investigation_id)?;
        if !role.can_set_marking() {
            bail!("only the owner can change the investigation marking");
        }
        Ok(())
    }

    /// Adds a member. The first add seeds the current identity as owner.
    pub fn add_member(
        &mut self,
        investigation_id: Uuid,
        user_id: &str,
        role: Role,
    ) -> Result<Member> {
        self.assert_can_manage_team(investigation_id)?;
        let user_id = user_id.trim();
        if user_id.is_empty() {
            bail!("a member needs a user id");
        }
        let identity = self.identity.clone();
        let transaction = self.connection_mut().transaction()?;
        seed_owner_if_empty(&transaction, investigation_id, &identity.user_id)?;
        if stored_user_id(&list_members(&transaction, investigation_id)?, user_id).is_some() {
            bail!("{user_id} is already a member of this investigation");
        }
        let member = Member {
            investigation_id,
            user_id: user_id.to_string(),
            role,
            added_by: identity.user_id.clone(),
            added_at: now(),
        };
        insert_member_record(&transaction, &member)?;
        let mut details = Fields::new();
        details.push("user_id", user_id);
        details.push("role", role.as_str());
        append_audit_on(
            &transaction,
            investigation_id,
            &identity,
            AuditKind::MemberAdded,
            &format!("Added {user_id} as {}", role.as_str()),
            details,
            Vec::new(),
        )?;
        touch_investigation(&transaction, investigation_id)?;
        transaction.commit()?;
        Ok(member)
    }

    pub fn set_member_role(
        &mut self,
        investigation_id: Uuid,
        user_id: &str,
        role: Role,
    ) -> Result<Member> {
        self.assert_can_manage_team(investigation_id)?;
        let user_id = user_id.trim();
        let identity = self.identity.clone();
        let transaction = self.connection_mut().transaction()?;
        let mut members = list_members(&transaction, investigation_id)?;
        let Some(existing) = members
            .iter()
            .find(|member| same_user(&member.user_id, user_id))
        else {
            return Err(eyre!("{user_id} is not a member of this investigation"));
        };
        let stored_id = existing.user_id.clone();
        if existing.role == Role::Owner
            && role != Role::Owner
            && members
                .iter()
                .filter(|member| member.role == Role::Owner)
                .count()
                == 1
        {
            bail!("an investigation needs at least one owner");
        }
        transaction.execute(
            "UPDATE investigation_members SET role = ?3 WHERE investigation_id = ?1 AND user_id = ?2",
            params![id(&investigation_id), stored_id, role.as_str()],
        )?;
        let mut details = Fields::new();
        details.push("user_id", user_id);
        details.push("role", role.as_str());
        append_audit_on(
            &transaction,
            investigation_id,
            &identity,
            AuditKind::MemberRoleChanged,
            &format!("Set {user_id} to {}", role.as_str()),
            details,
            Vec::new(),
        )?;
        touch_investigation(&transaction, investigation_id)?;
        transaction.commit()?;
        members = list_members(self.connection(), investigation_id)?;
        members
            .into_iter()
            .find(|member| same_user(&member.user_id, user_id))
            .ok_or_else(|| eyre!("member vanished after role change"))
    }

    pub fn remove_member(&mut self, investigation_id: Uuid, user_id: &str) -> Result<()> {
        self.assert_can_manage_team(investigation_id)?;
        let user_id = user_id.trim();
        let identity = self.identity.clone();
        let transaction = self.connection_mut().transaction()?;
        let members = list_members(&transaction, investigation_id)?;
        let Some(existing) = members
            .iter()
            .find(|member| same_user(&member.user_id, user_id))
        else {
            return Err(eyre!("{user_id} is not a member of this investigation"));
        };
        let stored_id = existing.user_id.clone();
        if existing.role == Role::Owner
            && members
                .iter()
                .filter(|member| member.role == Role::Owner)
                .count()
                == 1
        {
            bail!("an investigation needs at least one owner");
        }
        transaction.execute(
            "DELETE FROM investigation_members WHERE investigation_id = ?1 AND user_id = ?2",
            params![id(&investigation_id), stored_id],
        )?;
        let mut details = Fields::new();
        details.push("user_id", user_id);
        append_audit_on(
            &transaction,
            investigation_id,
            &identity,
            AuditKind::MemberRemoved,
            &format!("Removed {user_id}"),
            details,
            Vec::new(),
        )?;
        touch_investigation(&transaction, investigation_id)?;
        transaction.commit()?;
        Ok(())
    }
}

pub(crate) fn ensure_importer_owner(
    connection: &Connection,
    investigation_id: Uuid,
    identity: &Identity,
) -> rusqlite::Result<()> {
    let user_id = identity.user_id.as_str();
    let members = list_members(connection, investigation_id)?;
    if members.is_empty() {
        return Ok(());
    }
    if let Some(existing) = members
        .iter()
        .find(|member| same_user(&member.user_id, user_id))
    {
        if existing.role == Role::Owner {
            return Ok(());
        }
        let previous = existing.role;
        connection.execute(
            "UPDATE investigation_members SET role = 'owner' WHERE investigation_id = ?1 AND user_id = ?2",
            params![id(&investigation_id), existing.user_id],
        )?;
        let mut details = Fields::new();
        details.push("user_id", existing.user_id.as_str());
        details.push("from", previous.as_str());
        details.push("to", Role::Owner.as_str());
        append_audit_on(
            connection,
            investigation_id,
            identity,
            AuditKind::MemberRoleChanged,
            &format!("Importer {} promoted to owner", existing.user_id),
            details,
            Vec::new(),
        )?;
        return Ok(());
    }
    insert_member_record(
        connection,
        &Member {
            investigation_id,
            user_id: user_id.to_string(),
            role: Role::Owner,
            added_by: user_id.to_string(),
            added_at: now(),
        },
    )?;
    let mut details = Fields::new();
    details.push("user_id", user_id);
    details.push("role", Role::Owner.as_str());
    append_audit_on(
        connection,
        investigation_id,
        identity,
        AuditKind::MemberAdded,
        &format!("Importer {user_id} added as owner"),
        details,
        Vec::new(),
    )?;
    Ok(())
}

fn seed_owner_if_empty(
    connection: &Connection,
    investigation_id: Uuid,
    user_id: &str,
) -> rusqlite::Result<()> {
    let members = list_members(connection, investigation_id)?;
    if !members.is_empty() {
        return Ok(());
    }
    insert_member_record(
        connection,
        &Member {
            investigation_id,
            user_id: user_id.to_string(),
            role: Role::Owner,
            added_by: user_id.to_string(),
            added_at: now(),
        },
    )
}
