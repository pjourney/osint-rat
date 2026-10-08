//! A batch of new evidence produced by a collector, tool, or import, applied atomically.

use serde::{Deserialize, Serialize};

use super::artifact::ArtifactKind;
use super::common::{Confidence, Fields, Provenance, Timestamp, now};
use super::entity::EntityKind;
use super::source::SourceKind;
use super::timeline::TimelineKind;

/// Index of a source within `EvidenceBatch::sources`.
pub type SourceKey = usize;

/// Natural key of an entity: kind plus canonical value.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntityKey {
    pub kind: EntityKind,
    pub value: String,
}

impl EntityKey {
    pub fn new(kind: EntityKind, raw_value: &str) -> Self {
        let value = kind.normalize_value(raw_value);
        Self { kind, value }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewSource {
    pub kind: SourceKind,
    pub title: String,
    pub url: Option<String>,
    pub retrieved_at: Timestamp,
    pub published_at: Option<Timestamp>,
    pub author: Option<String>,
    pub collector: Option<String>,
    pub content_hash: Option<String>,
    pub snapshot_path: Option<String>,
    pub content: String,
    pub fields: Fields,
    pub provenance: Provenance,
}

impl NewSource {
    /// A collected source with the given kind and title, retrieved now.
    pub fn collected(kind: SourceKind, collector: &str, title: impl Into<String>) -> Self {
        Self {
            kind,
            title: title.into(),
            url: None,
            retrieved_at: now(),
            published_at: None,
            author: None,
            collector: Some(collector.to_string()),
            content_hash: None,
            snapshot_path: None,
            content: String::new(),
            fields: Fields::new(),
            provenance: Provenance::Collected,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewEntity {
    pub key: EntityKey,
    pub label: Option<String>,
    pub fields: Fields,
    pub sources: Vec<SourceKey>,
    /// Link the entity to the page the collection ran from.
    pub link_to_page: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewRelationship {
    pub from: EntityKey,
    pub kind: String,
    pub to: EntityKey,
    pub sources: Vec<SourceKey>,
    pub confidence: Confidence,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewTimelineEvent {
    pub occurred_at: Timestamp,
    pub kind: TimelineKind,
    pub title: String,
    pub description: String,
    pub source: Option<SourceKey>,
    pub entity: Option<EntityKey>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewArtifact {
    pub kind: ArtifactKind,
    pub name: String,
    /// Relative to the investigation directory.
    pub path: String,
    pub mime: Option<String>,
    pub size_bytes: i64,
    pub sha256: Option<String>,
    pub fields: Fields,
    pub source: Option<SourceKey>,
}

/// Everything a single collection produced.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EvidenceBatch {
    pub sources: Vec<NewSource>,
    pub entities: Vec<NewEntity>,
    pub relationships: Vec<NewRelationship>,
    pub timeline: Vec<NewTimelineEvent>,
    pub artifacts: Vec<NewArtifact>,
    /// Diagnostic gaps to store as collected findings. Present checks are listed in
    /// `resolved_findings` so an earlier gap can be cleared.
    #[serde(default)]
    pub findings: Vec<NewCollectedFinding>,
    /// Titles of collected findings that no longer apply.
    #[serde(default)]
    pub resolved_findings: Vec<String>,
    /// One-line human summary for the job result.
    pub summary: String,
}

/// A collected diagnostic finding. Provenance is always `Collected`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewCollectedFinding {
    pub title: String,
    pub details: String,
    pub confidence: Confidence,
    pub source: Option<SourceKey>,
}

impl EvidenceBatch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_source(&mut self, source: NewSource) -> SourceKey {
        self.sources.push(source);
        self.sources.len() - 1
    }

    /// Adds (or merges into) an entity and returns its key.
    pub fn entity(
        &mut self,
        kind: EntityKind,
        raw_value: &str,
        source: Option<SourceKey>,
    ) -> EntityKey {
        let key = EntityKey::new(kind, raw_value);
        if let Some(existing) = self.entities.iter_mut().find(|entity| entity.key == key) {
            if let Some(source) = source
                && !existing.sources.contains(&source)
            {
                existing.sources.push(source);
            }
            return key;
        }
        self.entities.push(NewEntity {
            key: key.clone(),
            label: None,
            fields: Fields::new(),
            sources: source.into_iter().collect(),
            link_to_page: true,
        });
        key
    }

    /// Adds a field to an entity already in the batch.
    pub fn annotate(&mut self, key: &EntityKey, field: &str, value: impl Into<String>) {
        if let Some(entity) = self.entities.iter_mut().find(|entity| &entity.key == key) {
            entity.fields.push(field, value);
        }
    }

    pub fn label(&mut self, key: &EntityKey, label: impl Into<String>) {
        if let Some(entity) = self.entities.iter_mut().find(|entity| &entity.key == key) {
            entity.label = Some(label.into());
        }
    }

    /// Adds a relationship (and both endpoint entities) supported by `source`.
    pub fn relate(
        &mut self,
        from: &EntityKey,
        kind: &str,
        to: &EntityKey,
        source: Option<SourceKey>,
    ) {
        if from == to {
            return;
        }
        self.entity(from.kind.clone(), &from.value, source);
        self.entity(to.kind.clone(), &to.value, source);
        if let Some(existing) = self.relationships.iter_mut().find(|relationship| {
            &relationship.from == from && relationship.kind == kind && &relationship.to == to
        }) {
            if let Some(source) = source
                && !existing.sources.contains(&source)
            {
                existing.sources.push(source);
            }
            return;
        }
        self.relationships.push(NewRelationship {
            from: from.clone(),
            kind: kind.to_string(),
            to: to.clone(),
            sources: source.into_iter().collect(),
            confidence: Confidence::High,
        });
    }

    pub fn add_finding(
        &mut self,
        title: impl Into<String>,
        details: impl Into<String>,
        confidence: Confidence,
        source: Option<SourceKey>,
    ) {
        let title = title.into();
        if title.trim().is_empty() || self.findings.iter().any(|finding| finding.title == title) {
            return;
        }
        self.resolved_findings.retain(|existing| existing != &title);
        self.findings.push(NewCollectedFinding {
            title,
            details: details.into(),
            confidence,
            source,
        });
    }

    pub fn resolve_finding(&mut self, title: impl Into<String>) {
        let title = title.into();
        if title.trim().is_empty() {
            return;
        }
        self.findings.retain(|finding| finding.title != title);
        if !self
            .resolved_findings
            .iter()
            .any(|existing| existing == &title)
        {
            self.resolved_findings.push(title);
        }
    }

    pub fn event(
        &mut self,
        occurred_at: Timestamp,
        kind: TimelineKind,
        title: impl Into<String>,
        source: Option<SourceKey>,
        entity: Option<EntityKey>,
    ) {
        self.timeline.push(NewTimelineEvent {
            occurred_at,
            kind,
            title: title.into(),
            description: String::new(),
            source,
            entity,
        });
    }

    /// Merges another batch into this one, re-indexing its source keys.
    pub fn merge(&mut self, other: EvidenceBatch) {
        let offset = self.sources.len();
        let shift =
            |keys: Vec<SourceKey>| keys.into_iter().map(|key| key + offset).collect::<Vec<_>>();
        self.sources.extend(other.sources);
        for entity in other.entities {
            for source in shift(entity.sources.clone()) {
                self.entity(entity.key.kind.clone(), &entity.key.value, Some(source));
            }
            if entity.sources.is_empty() {
                self.entity(entity.key.kind.clone(), &entity.key.value, None);
            }
            for (field, value) in entity.fields.0 {
                self.annotate(&entity.key, &field, value);
            }
            if let Some(label) = entity.label {
                self.label(&entity.key, label);
            }
        }
        for relationship in other.relationships {
            for source in shift(relationship.sources.clone()) {
                self.relate(
                    &relationship.from,
                    &relationship.kind,
                    &relationship.to,
                    Some(source),
                );
            }
        }
        for mut event in other.timeline {
            event.source = event.source.map(|key| key + offset);
            self.timeline.push(event);
        }
        for mut artifact in other.artifacts {
            artifact.source = artifact.source.map(|key| key + offset);
            self.artifacts.push(artifact);
        }
        for mut finding in other.findings {
            finding.source = finding.source.map(|key| key + offset);
            self.add_finding(
                finding.title,
                finding.details,
                finding.confidence,
                finding.source,
            );
        }
        for title in other.resolved_findings {
            if !self.findings.iter().any(|finding| finding.title == title) {
                self.resolve_finding(title);
            }
        }
        if !other.summary.is_empty() {
            if self.summary.is_empty() {
                self.summary = other.summary;
            } else {
                self.summary = format!("{}; {}", self.summary, other.summary);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty() && self.entities.is_empty() && self.timeline.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities_are_deduplicated_and_sources_merged() {
        let mut batch = EvidenceBatch::new();
        let first = batch.add_source(NewSource::collected(SourceKind::Dns, "dns", "a"));
        let second = batch.add_source(NewSource::collected(SourceKind::Http, "http", "b"));
        batch.entity(EntityKind::Domain, "Example.com", Some(first));
        batch.entity(EntityKind::Domain, "example.com.", Some(second));
        assert_eq!(batch.entities.len(), 1);
        assert_eq!(batch.entities[0].sources, vec![first, second]);
    }

    #[test]
    fn relate_adds_endpoints_and_ignores_self_loops() {
        let mut batch = EvidenceBatch::new();
        let source = batch.add_source(NewSource::collected(SourceKind::Dns, "dns", "a"));
        let domain = EntityKey::new(EntityKind::Domain, "example.com");
        let ip = EntityKey::new(EntityKind::Ip, "192.0.2.10");
        batch.relate(&domain, "resolves_to", &ip, Some(source));
        batch.relate(&domain, "resolves_to", &ip, Some(source));
        batch.relate(&domain, "alias_of", &domain, Some(source));
        assert_eq!(batch.entities.len(), 2);
        assert_eq!(batch.relationships.len(), 1);
    }

    #[test]
    fn merge_reindexes_source_keys() {
        let mut first = EvidenceBatch::new();
        first.add_source(NewSource::collected(SourceKind::Dns, "dns", "a"));
        let mut second = EvidenceBatch::new();
        let key = second.add_source(NewSource::collected(SourceKind::Http, "http", "b"));
        let domain = EntityKey::new(EntityKind::Domain, "example.com");
        let url = EntityKey::new(EntityKind::Url, "https://example.com/");
        second.relate(&domain, "serves", &url, Some(key));
        first.merge(second);
        assert_eq!(first.sources.len(), 2);
        assert_eq!(first.relationships[0].sources, vec![1]);
    }
}
