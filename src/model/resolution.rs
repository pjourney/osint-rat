//! Entity resolution bags: a canonical object plus the constituents it absorbed.
//!
//! Merges are reversible. Constituent rows stay in SQLite; the bag records property and
//! source writes applied to the canonical so unresolve can restore them. Suggestions are
//! scored only — nothing auto-merges.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::common::{Timestamp, now};
use super::entity::{Entity, EntityKind};
use super::ids::{RefKind, ShortRef};
use super::target::{normalize_hostname, normalize_url};
use super::workspace::InvestigationData;

/// Special property-write keys for entity columns that are not in `fields`.
pub const LABEL_WRITE_KEY: &str = "__label";
pub const NOTES_WRITE_KEY: &str = "__notes";

/// A scored, human-reviewed merge candidate. Never applied automatically.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolutionSuggestion {
    pub entity_ids: Vec<Uuid>,
    pub score: u32,
    pub rule: SuggestionRule,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestionRule {
    SameCanonicalValue,
    UsernameProfileUrl,
    CertSanCluster,
}

impl SuggestionRule {
    pub fn as_str(self) -> &'static str {
        match self {
            SuggestionRule::SameCanonicalValue => "same_canonical_value",
            SuggestionRule::UsernameProfileUrl => "username_profile_url",
            SuggestionRule::CertSanCluster => "cert_san_cluster",
        }
    }
}

/// Whether a resolution bag is still in effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionStatus {
    Active,
    Unresolved,
}

impl ResolutionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ResolutionStatus::Active => "active",
            ResolutionStatus::Unresolved => "unresolved",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "unresolved" => ResolutionStatus::Unresolved,
            _ => ResolutionStatus::Active,
        }
    }
}

/// One property (or label/notes) write applied to the canonical during merge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropertyWrite {
    pub key: String,
    #[serde(default)]
    pub previous: Option<String>,
    pub new_value: String,
}

/// Canonical object + constituents. Survives export/import as `data.resolutions`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Resolution {
    pub id: Uuid,
    pub investigation_id: Uuid,
    pub seq: i64,
    pub canonical_id: Uuid,
    /// Every member, including the canonical. Order is canonical first, then others.
    pub constituent_ids: Vec<Uuid>,
    pub status: ResolutionStatus,
    pub reason: String,
    pub created_by: String,
    pub created_at: Timestamp,
    #[serde(default)]
    pub unresolved_by: Option<String>,
    #[serde(default)]
    pub unresolved_at: Option<Timestamp>,
    #[serde(default)]
    pub property_writes: Vec<PropertyWrite>,
    #[serde(default)]
    pub source_writes: Vec<Uuid>,
}

impl Resolution {
    pub fn short_ref(&self) -> ShortRef {
        ShortRef::new(RefKind::Resolution, self.seq)
    }

    pub fn is_active(&self) -> bool {
        self.status == ResolutionStatus::Active
    }

    pub fn others(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.constituent_ids
            .iter()
            .copied()
            .filter(|id| *id != self.canonical_id)
    }

    pub fn contains(&self, entity_id: Uuid) -> bool {
        self.constituent_ids.contains(&entity_id)
    }
}

impl InvestigationData {
    pub fn active_resolutions(&self) -> impl Iterator<Item = &Resolution> {
        self.resolutions
            .iter()
            .filter(|resolution| resolution.is_active())
    }

    /// Walks active bags so stacked merges still resolve to one winner.
    pub fn canonical_id(&self, entity_id: Uuid) -> Uuid {
        let mut current = entity_id;
        let mut seen = HashSet::new();
        while seen.insert(current) {
            let Some(next) = self.active_resolutions().find_map(|resolution| {
                if resolution.contains(current) && resolution.canonical_id != current {
                    Some(resolution.canonical_id)
                } else {
                    None
                }
            }) else {
                break;
            };
            current = next;
        }
        current
    }

    pub fn is_absorbed(&self, entity_id: Uuid) -> bool {
        self.canonical_id(entity_id) != entity_id
    }

    pub fn constituent_ids_of(&self, canonical_id: Uuid) -> Vec<Uuid> {
        let mut ids = Vec::new();
        let mut seen = HashSet::new();
        for resolution in self.active_resolutions() {
            if self.canonical_id(resolution.canonical_id) != canonical_id {
                continue;
            }
            for id in resolution.others() {
                if seen.insert(id) {
                    ids.push(id);
                }
            }
        }
        ids
    }
}

/// Rules v1: same canonical email/domain/IP/repo URL; username↔profile URL; cert SAN clusters.
pub fn suggest_resolutions(data: &InvestigationData) -> Vec<ResolutionSuggestion> {
    let mut suggestions = Vec::new();
    push_same_canonical_value(&mut suggestions, data);
    push_username_profile_urls(&mut suggestions, data);
    push_cert_san_clusters(&mut suggestions, data);
    suggestions.retain(|suggestion| suggestion.entity_ids.len() >= 2);
    dedupe_suggestions(&mut suggestions);
    suggestions.retain(|suggestion| !already_resolved(data, &suggestion.entity_ids));
    suggestions.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.entity_ids.cmp(&right.entity_ids))
    });
    suggestions
}

fn already_resolved(data: &InvestigationData, entity_ids: &[Uuid]) -> bool {
    let mut canonicals = HashSet::new();
    for id in entity_ids {
        canonicals.insert(data.canonical_id(*id));
    }
    canonicals.len() <= 1
}

fn push_same_canonical_value(out: &mut Vec<ResolutionSuggestion>, data: &InvestigationData) {
    let mut host_groups: HashMap<String, Vec<Uuid>> = HashMap::new();
    let mut url_groups: HashMap<String, Vec<Uuid>> = HashMap::new();
    let mut ip_groups: HashMap<String, Vec<Uuid>> = HashMap::new();
    let mut email_groups: HashMap<String, Vec<Uuid>> = HashMap::new();
    let mut repo_or_url: Vec<(&Entity, String)> = Vec::new();
    for entity in &data.entities {
        match entity.kind {
            EntityKind::Domain | EntityKind::Hostname => {
                host_groups
                    .entry(entity.value.clone())
                    .or_default()
                    .push(entity.id);
            }
            EntityKind::Ip => {
                ip_groups
                    .entry(entity.value.clone())
                    .or_default()
                    .push(entity.id);
            }
            EntityKind::Email => {
                email_groups
                    .entry(entity.value.clone())
                    .or_default()
                    .push(entity.id);
            }
            EntityKind::Url | EntityKind::Repository => {
                url_groups
                    .entry(entity.value.clone())
                    .or_default()
                    .push(entity.id);
                if let Some(host) = url_host(&entity.value) {
                    repo_or_url.push((entity, host));
                }
            }
            _ => {}
        }
        if let Some(email) = entity.fields.get("email") {
            let normalized = EntityKind::Email.normalize_value(email);
            if !normalized.is_empty() {
                email_groups.entry(normalized).or_default().push(entity.id);
            }
        }
    }
    emit_groups(
        out,
        host_groups,
        95,
        SuggestionRule::SameCanonicalValue,
        "same canonical domain or hostname",
    );
    emit_groups(
        out,
        url_groups,
        90,
        SuggestionRule::SameCanonicalValue,
        "same canonical URL or repository URL",
    );
    emit_groups(
        out,
        ip_groups,
        95,
        SuggestionRule::SameCanonicalValue,
        "same canonical IP",
    );
    emit_groups(
        out,
        email_groups,
        85,
        SuggestionRule::SameCanonicalValue,
        "same canonical email",
    );
    for entity in &data.entities {
        if !matches!(entity.kind, EntityKind::Domain | EntityKind::Hostname) {
            continue;
        }
        for (other, host) in &repo_or_url {
            if host == &entity.value && other.id != entity.id {
                out.push(ResolutionSuggestion {
                    entity_ids: ordered_ids(&[entity.id, other.id]),
                    score: 75,
                    rule: SuggestionRule::SameCanonicalValue,
                    reason: format!(
                        "URL host {} matches {}",
                        other.display_name(),
                        entity.display_name()
                    ),
                });
            }
        }
    }
}

fn emit_groups(
    out: &mut Vec<ResolutionSuggestion>,
    groups: HashMap<String, Vec<Uuid>>,
    score: u32,
    rule: SuggestionRule,
    reason: &str,
) {
    for (value, mut ids) in groups {
        ids.sort();
        ids.dedup();
        if ids.len() < 2 {
            continue;
        }
        out.push(ResolutionSuggestion {
            entity_ids: ids,
            score,
            rule,
            reason: format!("{reason} ({value})"),
        });
    }
}

fn push_username_profile_urls(out: &mut Vec<ResolutionSuggestion>, data: &InvestigationData) {
    let usernames: Vec<&Entity> = data
        .entities
        .iter()
        .filter(|entity| entity.kind == EntityKind::Username)
        .collect();
    let urls: Vec<&Entity> = data
        .entities
        .iter()
        .filter(|entity| matches!(entity.kind, EntityKind::Url | EntityKind::Repository))
        .collect();
    for username in usernames {
        let handle = username.value.trim_start_matches('@').to_ascii_lowercase();
        if handle.is_empty() {
            continue;
        }
        for url in &urls {
            if let Some(extracted) = profile_username(&url.value)
                && extracted == handle
            {
                out.push(ResolutionSuggestion {
                    entity_ids: ordered_ids(&[username.id, url.id]),
                    score: 80,
                    rule: SuggestionRule::UsernameProfileUrl,
                    reason: format!(
                        "username {} matches profile URL {}",
                        username.display_name(),
                        url.display_name()
                    ),
                });
            }
        }
    }
}

fn push_cert_san_clusters(out: &mut Vec<ResolutionSuggestion>, data: &InvestigationData) {
    let hosts: Vec<&Entity> = data
        .entities
        .iter()
        .filter(|entity| matches!(entity.kind, EntityKind::Domain | EntityKind::Hostname))
        .collect();
    for cert in data
        .entities
        .iter()
        .filter(|entity| entity.kind == EntityKind::Certificate)
    {
        let names = certificate_sans(cert);
        if names.is_empty() {
            continue;
        }
        let mut ids = vec![cert.id];
        let mut matched = Vec::new();
        for host in &hosts {
            if names.contains(&host.value) {
                ids.push(host.id);
                matched.push(host.display_name().to_string());
            }
        }
        ids = ordered_ids(&ids);
        if ids.len() < 2 {
            continue;
        }
        out.push(ResolutionSuggestion {
            entity_ids: ids,
            score: 70,
            rule: SuggestionRule::CertSanCluster,
            reason: format!("certificate SAN cluster ({})", matched.join(", ")),
        });
    }
}

fn certificate_sans(entity: &Entity) -> HashSet<String> {
    const KEYS: &[&str] = &[
        "san",
        "sans",
        "dns_names",
        "dns_name",
        "subject_alt_name",
        "subject_alt_names",
        "covers",
        "names",
        "alt_names",
    ];
    let mut names = HashSet::new();
    for key in KEYS {
        if let Some(value) = entity.fields.get(key) {
            for part in value.split(|ch: char| ch == ',' || ch == ';' || ch.is_whitespace()) {
                if let Ok(host) = normalize_hostname(part) {
                    names.insert(host);
                }
            }
        }
    }
    names
}

fn url_host(value: &str) -> Option<String> {
    let url = normalize_url(value).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    Some(host.trim_end_matches('.').to_string())
}

fn profile_username(value: &str) -> Option<String> {
    let url = normalize_url(value).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host.trim_start_matches("www.");
    if !is_profile_host(host) {
        return None;
    }
    let mut segments: Vec<&str> = url
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.is_empty() {
        return None;
    }
    if host.ends_with("reddit.com")
        && matches!(segments.first().copied(), Some("user" | "u"))
        && segments.len() >= 2
    {
        segments.remove(0);
    }
    if host.ends_with("linkedin.com")
        && segments.first().copied() == Some("in")
        && segments.len() >= 2
    {
        segments.remove(0);
    }
    let handle = segments
        .first()?
        .trim_start_matches('@')
        .trim()
        .to_ascii_lowercase();
    if handle.is_empty() {
        None
    } else {
        Some(handle)
    }
}

fn is_profile_host(host: &str) -> bool {
    const HOSTS: &[&str] = &[
        "github.com",
        "gitlab.com",
        "bitbucket.org",
        "twitter.com",
        "x.com",
        "reddit.com",
        "linkedin.com",
        "instagram.com",
        "facebook.com",
        "medium.com",
        "hackerone.com",
        "keybase.io",
    ];
    HOSTS.iter().any(|known| {
        host == *known
            || host
                .strip_suffix(known)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

fn ordered_ids(ids: &[Uuid]) -> Vec<Uuid> {
    let mut unique = ids.to_vec();
    unique.sort();
    unique.dedup();
    unique
}

fn dedupe_suggestions(suggestions: &mut Vec<ResolutionSuggestion>) {
    let mut seen: HashSet<Vec<Uuid>> = HashSet::new();
    suggestions.retain(|suggestion| seen.insert(suggestion.entity_ids.clone()));
}

/// Builds the writes that would be applied if `others` were merged into `canonical`.
pub fn plan_property_writes(canonical: &Entity, others: &[&Entity]) -> Vec<PropertyWrite> {
    let mut writes = Vec::new();
    let mut seen_keys = HashSet::new();
    for other in others {
        if canonical.label.trim().is_empty()
            && !other.label.trim().is_empty()
            && seen_keys.insert(LABEL_WRITE_KEY)
        {
            writes.push(PropertyWrite {
                key: LABEL_WRITE_KEY.into(),
                previous: None,
                new_value: other.label.clone(),
            });
        }
        if canonical.notes.trim().is_empty()
            && !other.notes.trim().is_empty()
            && seen_keys.insert(NOTES_WRITE_KEY)
        {
            writes.push(PropertyWrite {
                key: NOTES_WRITE_KEY.into(),
                previous: None,
                new_value: other.notes.clone(),
            });
        }
        for (key, value) in other.fields.iter() {
            if canonical.fields.get(key).is_some() || !seen_keys.insert(key) {
                continue;
            }
            writes.push(PropertyWrite {
                key: key.to_string(),
                previous: None,
                new_value: value.to_string(),
            });
        }
    }
    writes
}

pub fn plan_source_writes(canonical: &Entity, others: &[&Entity]) -> Vec<Uuid> {
    let mut existing: HashSet<Uuid> = canonical.source_ids.iter().copied().collect();
    let mut added = Vec::new();
    for other in others {
        for source_id in &other.source_ids {
            if existing.insert(*source_id) {
                added.push(*source_id);
            }
        }
    }
    added
}

#[allow(clippy::too_many_arguments)]
pub fn new_resolution(
    investigation_id: Uuid,
    seq: i64,
    canonical_id: Uuid,
    constituent_ids: Vec<Uuid>,
    reason: &str,
    created_by: &str,
    property_writes: Vec<PropertyWrite>,
    source_writes: Vec<Uuid>,
) -> Resolution {
    let mut members = constituent_ids;
    if !members.contains(&canonical_id) {
        members.insert(0, canonical_id);
    }
    members.sort_by_key(|id| if *id == canonical_id { 0 } else { 1 });
    members.dedup();
    Resolution {
        id: Uuid::new_v4(),
        investigation_id,
        seq,
        canonical_id,
        constituent_ids: members,
        status: ResolutionStatus::Active,
        reason: reason.trim().to_string(),
        created_by: created_by.trim().to_string(),
        created_at: now(),
        unresolved_by: None,
        unresolved_at: None,
        property_writes,
        source_writes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EntityKind, Fields, Investigation, InvestigationData, Provenance};

    fn entity(kind: EntityKind, value: &str) -> Entity {
        let timestamp = now();
        Entity {
            id: Uuid::new_v4(),
            investigation_id: Uuid::nil(),
            seq: 1,
            kind: kind.clone(),
            value: kind.normalize_value(value),
            label: String::new(),
            notes: String::new(),
            provenance: Provenance::Collected,
            fields: Fields::new(),
            source_ids: Vec::new(),
            created_at: timestamp,
            updated_at: timestamp,
        }
    }

    fn data_with(entities: Vec<Entity>) -> InvestigationData {
        let timestamp = now();
        let mut data = InvestigationData::empty(Investigation {
            id: Uuid::nil(),
            name: "case".into(),
            description: String::new(),
            tags: Vec::new(),
            created_at: timestamp,
            updated_at: timestamp,
        });
        data.entities = entities;
        data
    }

    #[test]
    fn suggests_same_host_domain_and_hostname() {
        let domain = entity(EntityKind::Domain, "Example.com");
        let host = entity(EntityKind::Hostname, "example.com");
        let data = data_with(vec![domain.clone(), host.clone()]);
        let actual = suggest_resolutions(&data);
        assert!(
            actual.iter().any(|item| {
                item.rule == SuggestionRule::SameCanonicalValue
                    && item.entity_ids.contains(&domain.id)
                    && item.entity_ids.contains(&host.id)
                    && item.score >= 90
            }),
            "{actual:?}"
        );
    }

    #[test]
    fn suggests_username_and_github_profile() {
        let user = entity(EntityKind::Username, "@octocat");
        let url = entity(EntityKind::Url, "https://github.com/octocat");
        let data = data_with(vec![user.clone(), url.clone()]);
        let actual = suggest_resolutions(&data);
        assert!(
            actual.iter().any(|item| {
                item.rule == SuggestionRule::UsernameProfileUrl
                    && item.entity_ids.contains(&user.id)
                    && item.entity_ids.contains(&url.id)
            }),
            "{actual:?}"
        );
    }

    #[test]
    fn suggests_cert_san_cluster() {
        let mut cert = entity(EntityKind::Certificate, "abcd");
        cert.fields.push("san", "www.example.com, example.com");
        let domain = entity(EntityKind::Domain, "example.com");
        let data = data_with(vec![cert.clone(), domain.clone()]);
        let actual = suggest_resolutions(&data);
        assert!(
            actual.iter().any(|item| {
                item.rule == SuggestionRule::CertSanCluster
                    && item.entity_ids.contains(&cert.id)
                    && item.entity_ids.contains(&domain.id)
            }),
            "{actual:?}"
        );
    }

    #[test]
    fn suggestions_skip_already_resolved_pairs() {
        let domain = entity(EntityKind::Domain, "example.com");
        let host = entity(EntityKind::Hostname, "example.com");
        let mut data = data_with(vec![domain.clone(), host.clone()]);
        data.resolutions.push(Resolution {
            id: Uuid::new_v4(),
            investigation_id: Uuid::nil(),
            seq: 1,
            canonical_id: domain.id,
            constituent_ids: vec![domain.id, host.id],
            status: ResolutionStatus::Active,
            reason: "test".into(),
            created_by: "local".into(),
            created_at: now(),
            unresolved_by: None,
            unresolved_at: None,
            property_writes: Vec::new(),
            source_writes: Vec::new(),
        });
        let actual = suggest_resolutions(&data);
        assert!(
            actual
                .iter()
                .all(|item| !item.entity_ids.contains(&domain.id)
                    || !item.entity_ids.contains(&host.id)),
            "{actual:?}"
        );
    }

    #[test]
    fn canonical_id_walks_stacked_bags() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();
        let mut data = data_with(Vec::new());
        data.resolutions.push(Resolution {
            id: Uuid::new_v4(),
            investigation_id: Uuid::nil(),
            seq: 1,
            canonical_id: a,
            constituent_ids: vec![a, b],
            status: ResolutionStatus::Active,
            reason: String::new(),
            created_by: "local".into(),
            created_at: now(),
            unresolved_by: None,
            unresolved_at: None,
            property_writes: Vec::new(),
            source_writes: Vec::new(),
        });
        data.resolutions.push(Resolution {
            id: Uuid::new_v4(),
            investigation_id: Uuid::nil(),
            seq: 2,
            canonical_id: a,
            constituent_ids: vec![a, c],
            status: ResolutionStatus::Active,
            reason: String::new(),
            created_by: "local".into(),
            created_at: now(),
            unresolved_by: None,
            unresolved_at: None,
            property_writes: Vec::new(),
            source_writes: Vec::new(),
        });
        assert_eq!(data.canonical_id(b), a);
        assert_eq!(data.canonical_id(c), a);
        assert!(!data.is_absorbed(a));
        assert!(data.is_absorbed(b));
    }

    #[test]
    fn plan_writes_copies_missing_fields_only() {
        let mut canonical = entity(EntityKind::Domain, "example.com");
        canonical.fields.push("registrar", "Keep Me");
        let mut other = entity(EntityKind::Hostname, "example.com");
        other.fields.push("registrar", "Ignore");
        other.fields.push("asn", "AS1");
        other.label = "Example".into();
        let writes = plan_property_writes(&canonical, &[&other]);
        assert!(writes.iter().any(|write| write.key == "asn"));
        assert!(writes.iter().all(|write| write.key != "registrar"));
        assert!(writes.iter().any(|write| write.key == LABEL_WRITE_KEY));
    }
}
