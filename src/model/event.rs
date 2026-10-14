//! First-class Event objects: instants or intervals, linked to entities.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::batch::{EntityKey, SourceKey};
use super::common::{Fields, Provenance, Timestamp};
use super::ids::{RefKind, ShortRef};
use super::timeline::TimelineKind;

/// An investigative event: an instant when `ended_at` is absent, otherwise an interval.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: Uuid,
    pub investigation_id: Uuid,
    pub seq: i64,
    pub kind: TimelineKind,
    pub title: String,
    pub description: String,
    pub started_at: Timestamp,
    #[serde(default)]
    pub ended_at: Option<Timestamp>,
    pub provenance: Provenance,
    #[serde(default)]
    pub source_id: Option<Uuid>,
    #[serde(default)]
    pub page_id: Option<Uuid>,
    #[serde(default)]
    pub fields: Fields,
    #[serde(default)]
    pub entity_ids: Vec<Uuid>,
    pub created_at: Timestamp,
}

impl Event {
    pub fn short_ref(&self) -> ShortRef {
        ShortRef::new(RefKind::Event, self.seq)
    }

    pub fn is_interval(&self) -> bool {
        self.ended_at.is_some()
    }
}

/// Collector / ingest payload for an [`Event`]. Entity keys resolve at apply time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewEvent {
    pub started_at: Timestamp,
    #[serde(default)]
    pub ended_at: Option<Timestamp>,
    pub kind: TimelineKind,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub source: Option<SourceKey>,
    #[serde(default)]
    pub entities: Vec<EntityKey>,
    #[serde(default)]
    pub fields: Fields,
}

impl NewEvent {
    pub fn instant(
        started_at: Timestamp,
        kind: TimelineKind,
        title: impl Into<String>,
        source: Option<SourceKey>,
        entity: Option<EntityKey>,
    ) -> Self {
        Self {
            started_at,
            ended_at: None,
            kind,
            title: title.into(),
            description: String::new(),
            source,
            entities: entity.into_iter().collect(),
            fields: Fields::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::now;
    use chrono::Duration;

    #[test]
    fn interval_is_the_presence_of_ended_at() {
        let started = now();
        let mut event = Event {
            id: Uuid::new_v4(),
            investigation_id: Uuid::new_v4(),
            seq: 1,
            kind: TimelineKind::Observed,
            title: "window".into(),
            description: String::new(),
            started_at: started,
            ended_at: None,
            provenance: Provenance::External,
            source_id: None,
            page_id: None,
            fields: Fields::new(),
            entity_ids: Vec::new(),
            created_at: started,
        };
        assert!(!event.is_interval());
        event.ended_at = Some(started + Duration::hours(2));
        assert!(event.is_interval());
        assert_eq!(event.short_ref().to_string(), "C-1");
    }
}
