//! Dated events on a case.
//!
//! Collectors write registration dates, certificate expirations, commits, and the moment an
//! observation's content hash changes (`TimelineKind::Changed`). Analyst-entered events use
//! `TimelineKind::UserEvent`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::common::{Provenance, Timestamp, open_enum};
use super::ids::{RefKind, ShortRef};

open_enum! {
    /// The category of a timeline event.
    pub enum TimelineKind {
        Registration => ("registration", "Registration"),
        Expiration => ("expiration", "Expiration"),
        Updated => ("updated", "Record updated", "last_changed"),
        CertificateIssued => ("certificate_issued", "Certificate issued"),
        CertificateExpires => ("certificate_expires", "Certificate expires"),
        Commit => ("commit", "Commit"),
        Release => ("release", "Release"),
        Published => ("published", "Published"),
        Observed => ("observed", "Observed", "collection"),
        Changed => ("changed", "Observation changed"),
        UserEvent => ("user", "Analyst event", "manual"),
        Finding => ("finding", "Finding recorded"),
        Investigation => ("investigation", "Investigation activity"),
        EntitiesResolved => ("entities_resolved", "Entities resolved"),
        EntitiesUnresolved => ("entities_unresolved", "Entities unresolved"),
        PlaybookRun => ("playbook_run", "Playbook run"),
    }
}

/// Something that happened at a point in time, with optional links to evidence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub id: Uuid,
    pub investigation_id: Uuid,
    pub seq: i64,
    pub occurred_at: Timestamp,
    pub kind: TimelineKind,
    pub title: String,
    pub description: String,
    pub provenance: Provenance,
    pub source_id: Option<Uuid>,
    pub entity_id: Option<Uuid>,
    pub page_id: Option<Uuid>,
    pub created_at: Timestamp,
}

impl TimelineEvent {
    pub fn short_ref(&self) -> ShortRef {
        ShortRef::new(RefKind::Timeline, self.seq)
    }
}

/// One row on the Timeline screen: a log event, a first-class Event, or both collapsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimelineRowOrigin {
    Log,
    Event,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimelineRow {
    pub id: uuid::Uuid,
    pub origin: TimelineRowOrigin,
    pub started_at: Timestamp,
    pub ended_at: Option<Timestamp>,
    pub kind: TimelineKind,
    pub title: String,
    pub description: String,
    pub provenance: Provenance,
    pub entity_ids: Vec<uuid::Uuid>,
    pub source_id: Option<uuid::Uuid>,
    pub short_ref: ShortRef,
}

/// Overlay of `timeline` log rows plus first-class Event objects, oldest first.
/// When both exist for the same kind/time/title, the Event wins so intervals stay visible.
pub fn overlay_timeline(
    data: &super::workspace::InvestigationData,
    entity_filter: Option<uuid::Uuid>,
) -> Vec<TimelineRow> {
    let members = entity_filter.map(|id| data.member_ids_of(id));
    let mut rows: Vec<TimelineRow> = Vec::new();
    for event in &data.events {
        if let Some(members) = members.as_ref()
            && !event
                .entity_ids
                .iter()
                .any(|id| members.contains(id) || members.contains(&data.canonical_id(*id)))
        {
            continue;
        }
        rows.push(TimelineRow {
            id: event.id,
            origin: TimelineRowOrigin::Event,
            started_at: event.started_at,
            ended_at: event.ended_at,
            kind: event.kind.clone(),
            title: event.title.clone(),
            description: event.description.clone(),
            provenance: event.provenance,
            entity_ids: event.entity_ids.clone(),
            source_id: event.source_id,
            short_ref: event.short_ref(),
        });
    }
    for event in &data.timeline {
        if let Some(members) = members.as_ref() {
            let linked = event
                .entity_id
                .map(|id| members.contains(&id) || members.contains(&data.canonical_id(id)))
                .unwrap_or(false);
            if !linked {
                continue;
            }
        }
        if rows.iter().any(|row| {
            row.kind == event.kind
                && row.started_at == event.occurred_at
                && row.title == event.title
        }) {
            continue;
        }
        rows.push(TimelineRow {
            id: event.id,
            origin: TimelineRowOrigin::Log,
            started_at: event.occurred_at,
            ended_at: None,
            kind: event.kind.clone(),
            title: event.title.clone(),
            description: event.description.clone(),
            provenance: event.provenance,
            entity_ids: event.entity_id.into_iter().collect(),
            source_id: event.source_id,
            short_ref: event.short_ref(),
        });
    }
    rows.sort_by(|left, right| {
        left.started_at
            .cmp(&right.started_at)
            .then(left.short_ref.seq.cmp(&right.short_ref.seq))
            .then(left.title.cmp(&right.title))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        Entity, EntityKind, Event, Fields, Investigation, InvestigationData, Provenance, now,
    };
    use uuid::Uuid;

    #[test]
    fn overlay_includes_collection_and_imported_events_and_filters() {
        let investigation_id = Uuid::new_v4();
        let alpha = Uuid::new_v4();
        let beta = Uuid::new_v4();
        let stamp = now();
        let mut data = InvestigationData::empty(Investigation {
            id: investigation_id,
            name: "t".into(),
            description: String::new(),
            tags: Vec::new(),
            marking: crate::model::Marking::Unmarked,
            created_at: stamp,
            updated_at: stamp,
        });
        data.entities = vec![
            entity(investigation_id, alpha, 1, "192.0.2.1"),
            entity(investigation_id, beta, 2, "192.0.2.2"),
        ];
        data.timeline.push(TimelineEvent {
            id: Uuid::new_v4(),
            investigation_id,
            seq: 1,
            occurred_at: stamp,
            kind: TimelineKind::Registration,
            title: "collected".into(),
            description: String::new(),
            provenance: Provenance::Collected,
            source_id: None,
            entity_id: Some(alpha),
            page_id: None,
            created_at: stamp,
        });
        data.events.push(Event {
            id: Uuid::new_v4(),
            investigation_id,
            seq: 1,
            kind: TimelineKind::Observed,
            title: "imported".into(),
            description: String::new(),
            started_at: stamp,
            ended_at: Some(stamp),
            provenance: Provenance::External,
            source_id: None,
            page_id: None,
            fields: Fields::new(),
            entity_ids: vec![beta],
            created_at: stamp,
        });
        let all = overlay_timeline(&data, None);
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|row| row.title == "collected"));
        assert!(
            all.iter()
                .any(|row| row.title == "imported" && row.ended_at.is_some())
        );
        let filtered = overlay_timeline(&data, Some(alpha));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].title, "collected");
        assert!(overlay_timeline(&data, Some(Uuid::new_v4())).is_empty());
    }

    fn entity(investigation_id: Uuid, id: Uuid, seq: i64, value: &str) -> Entity {
        Entity {
            id,
            investigation_id,
            seq,
            kind: EntityKind::Ip,
            value: value.into(),
            label: String::new(),
            notes: String::new(),
            provenance: Provenance::Collected,
            fields: Fields::new(),
            source_ids: Vec::new(),
            created_at: now(),
            updated_at: now(),
        }
    }
}
