use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::common::{Timestamp, open_enum};
use super::entity::EntityKind;
use super::ids::{RefKind, ShortRef};
use super::target::{Target, TargetKind};

open_enum! {
    /// The type of an investigation page. Open-ended: unknown kinds are preserved as `Other`.
    pub enum PageKind {
        Generic => ("generic", "Generic", "note", "notes"),
        Person => ("person", "Person", "people"),
        Organization => ("organization", "Organization", "org", "company", "organisation"),
        Domain => ("domain", "Domain", "hostname"),
        Ip => ("ip", "IP address", "ip_address", "ipv4", "ipv6"),
        Url => ("url", "URL", "link"),
        Email => ("email", "Email", "mail"),
        Username => ("username", "Username", "handle", "account"),
        Repository => ("repository", "Repository", "repo", "git"),
        File => ("file", "File", "document"),
        Event => ("event", "Event", "incident"),
    }
}

impl PageKind {
    /// The kind of the page's primary entity, if the page is about a concrete identifier.
    pub fn entity_kind(&self) -> Option<EntityKind> {
        match self {
            PageKind::Person => Some(EntityKind::Person),
            PageKind::Organization => Some(EntityKind::Organization),
            PageKind::Domain => Some(EntityKind::Domain),
            PageKind::Ip => Some(EntityKind::Ip),
            PageKind::Url => Some(EntityKind::Url),
            PageKind::Email => Some(EntityKind::Email),
            PageKind::Username => Some(EntityKind::Username),
            PageKind::Repository => Some(EntityKind::Repository),
            PageKind::File => Some(EntityKind::File),
            PageKind::Generic | PageKind::Event | PageKind::Other(_) => None,
        }
    }

    /// The collector target kind, when collectors can run against pages of this kind.
    pub fn target_kind(&self) -> Option<TargetKind> {
        match self {
            PageKind::Domain => Some(TargetKind::Domain),
            PageKind::Ip => Some(TargetKind::Ip),
            PageKind::Url => Some(TargetKind::Url),
            PageKind::Email => Some(TargetKind::Email),
            PageKind::Username => Some(TargetKind::Username),
            PageKind::Repository => Some(TargetKind::Repository),
            PageKind::File => Some(TargetKind::File),
            _ => None,
        }
    }

    /// Example input shown in the "new page" form.
    pub fn target_hint(&self) -> &'static str {
        match self {
            PageKind::Domain => "example.com",
            PageKind::Ip => "192.0.2.15",
            PageKind::Url => "https://example.com/about",
            PageKind::Email => "alice@example.com",
            PageKind::Username => "octocat",
            PageKind::Repository => "https://github.com/owner/project",
            PageKind::File => "/path/to/evidence.pdf",
            PageKind::Person => "Full name",
            PageKind::Organization => "Organization name",
            _ => "(optional)",
        }
    }

    pub fn from_target_kind(kind: TargetKind) -> PageKind {
        match kind {
            TargetKind::Domain => PageKind::Domain,
            TargetKind::Ip => PageKind::Ip,
            TargetKind::Url => PageKind::Url,
            TargetKind::Email => PageKind::Email,
            TargetKind::Repository => PageKind::Repository,
            TargetKind::File => PageKind::File,
            TargetKind::Username => PageKind::Username,
            TargetKind::Asn => PageKind::Organization,
        }
    }
}

/// A page is the primary investigation workspace: a subject plus its notes and evidence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub id: Uuid,
    pub investigation_id: Uuid,
    pub seq: i64,
    pub title: String,
    pub kind: PageKind,
    /// The page subject in canonical form (`example.com`, `192.0.2.15`, ...).
    pub target: Option<String>,
    pub description: String,
    /// Markdown notes written by the analyst.
    pub notes: String,
    /// The primary entity for this page's subject.
    pub entity_id: Option<Uuid>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Page {
    pub fn short_ref(&self) -> ShortRef {
        ShortRef::new(RefKind::Page, self.seq)
    }

    /// The collection target for this page, when it has one.
    pub fn collection_target(&self) -> Option<Target> {
        let kind = self.kind.target_kind()?;
        let value = self.target.as_deref()?;
        Target::parse(kind, value).ok()
    }
}
