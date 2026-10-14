//! Co-occurrence: entities that share a time window, a source, or an Event.

use chrono::Duration;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::event::Event;
use super::workspace::InvestigationData;

/// Default window used when two instants are treated as co-occurring.
pub const DEFAULT_WINDOW: Duration = Duration::hours(24);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoOccurrenceKind {
    SharedEvent,
    SharedSource,
    SharedWindow,
}

impl CoOccurrenceKind {
    /// Serde / CLI key (`shared_event`, `shared_source`, `shared_window`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SharedEvent => "shared_event",
            Self::SharedSource => "shared_source",
            Self::SharedWindow => "shared_window",
        }
    }
}

/// A co-occurring pair of canonical entities.
///
/// `left` is the lesser UUID, `right` the greater (`left < right`). [`co_occur`]
/// returns these sorted by `(left, right, kind.as_str())`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoOccurrence {
    pub left: Uuid,
    pub right: Uuid,
    pub kind: CoOccurrenceKind,
    pub evidence: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoOccurrenceQuery {
    /// Restrict to these canonical entity ids. Empty means every visible entity.
    pub entity_ids: Vec<Uuid>,
    pub window: Option<Duration>,
}

/// Pairs of visible (canonical) entities that co-occur. Empty data returns no pairs.
///
/// Each pair has `left < right` by canonical UUID. The vec is sorted by `left`, then
/// `right`, then [`CoOccurrenceKind::as_str`] (the serde key), not by `Debug`.
pub fn co_occur(data: &InvestigationData, query: &CoOccurrenceQuery) -> Vec<CoOccurrence> {
    if data.entities.is_empty() {
        return Vec::new();
    }
    let window = query.window.unwrap_or(DEFAULT_WINDOW);
    let mut pairs = Vec::new();
    pairs.extend(shared_events(data, query));
    pairs.extend(shared_sources(data, query));
    pairs.extend(shared_windows(data, query, window));
    pairs.sort_by(cmp_pair);
    pairs.dedup_by(|left, right| {
        left.left == right.left && left.right == right.right && left.kind == right.kind
    });
    pairs
}

fn cmp_pair(left: &CoOccurrence, right: &CoOccurrence) -> std::cmp::Ordering {
    left.left
        .cmp(&right.left)
        .then(left.right.cmp(&right.right))
        .then(left.kind.as_str().cmp(right.kind.as_str()))
}

fn wanted(data: &InvestigationData, query: &CoOccurrenceQuery, entity_id: Uuid) -> bool {
    if data.is_absorbed(entity_id) {
        return false;
    }
    if query.entity_ids.is_empty() {
        return true;
    }
    let canonical = data.canonical_id(entity_id);
    query
        .entity_ids
        .iter()
        .any(|id| data.canonical_id(*id) == canonical)
}

pub(crate) fn ordered_pair(left: Uuid, right: Uuid) -> Option<(Uuid, Uuid)> {
    if left == right {
        return None;
    }
    if left < right {
        Some((left, right))
    } else {
        Some((right, left))
    }
}

fn shared_events(data: &InvestigationData, query: &CoOccurrenceQuery) -> Vec<CoOccurrence> {
    let mut pairs = Vec::new();
    for event in &data.events {
        let members: Vec<Uuid> = event
            .entity_ids
            .iter()
            .copied()
            .map(|id| data.canonical_id(id))
            .filter(|id| wanted(data, query, *id))
            .collect();
        push_clique(
            &mut pairs,
            &members,
            CoOccurrenceKind::SharedEvent,
            &event.title,
        );
    }
    pairs
}

fn shared_sources(data: &InvestigationData, query: &CoOccurrenceQuery) -> Vec<CoOccurrence> {
    let mut pairs = Vec::new();
    for source in &data.sources {
        let members: Vec<Uuid> = data
            .entities
            .iter()
            .filter(|entity| entity.source_ids.contains(&source.id))
            .map(|entity| data.canonical_id(entity.id))
            .filter(|id| wanted(data, query, *id))
            .collect();
        push_clique(
            &mut pairs,
            &members,
            CoOccurrenceKind::SharedSource,
            &source.title,
        );
    }
    pairs
}

fn shared_windows(
    data: &InvestigationData,
    query: &CoOccurrenceQuery,
    window: Duration,
) -> Vec<CoOccurrence> {
    let spans = entity_spans(data);
    let mut pairs = Vec::new();
    for (index, left) in spans.iter().enumerate() {
        if !wanted(data, query, left.entity_id) {
            continue;
        }
        for right in spans.iter().skip(index + 1) {
            if !wanted(data, query, right.entity_id) {
                continue;
            }
            let Some((a, b)) = ordered_pair(left.entity_id, right.entity_id) else {
                continue;
            };
            if overlaps(left, right, window) {
                pairs.push(CoOccurrence {
                    left: a,
                    right: b,
                    kind: CoOccurrenceKind::SharedWindow,
                    evidence: format!("{} overlaps {}", left.title, right.title),
                });
            }
        }
    }
    pairs
}

struct Span {
    entity_id: Uuid,
    start: chrono::DateTime<chrono::Utc>,
    end: chrono::DateTime<chrono::Utc>,
    title: String,
}

fn entity_spans(data: &InvestigationData) -> Vec<Span> {
    let mut spans = Vec::new();
    for event in &data.timeline {
        let Some(entity_id) = event.entity_id else {
            continue;
        };
        let canonical = data.canonical_id(entity_id);
        if data.is_absorbed(canonical) {
            continue;
        }
        spans.push(Span {
            entity_id: canonical,
            start: event.occurred_at,
            end: event.occurred_at,
            title: event.title.clone(),
        });
    }
    for event in &data.events {
        let end = event.ended_at.unwrap_or(event.started_at);
        for entity_id in &event.entity_ids {
            let canonical = data.canonical_id(*entity_id);
            if data.is_absorbed(canonical) {
                continue;
            }
            spans.push(Span {
                entity_id: canonical,
                start: event.started_at,
                end,
                title: event.title.clone(),
            });
        }
    }
    spans
}

fn overlaps(left: &Span, right: &Span, window: Duration) -> bool {
    let left_end = left.end + window;
    let right_end = right.end + window;
    left.start <= right_end && right.start <= left_end
}

fn push_clique(
    pairs: &mut Vec<CoOccurrence>,
    members: &[Uuid],
    kind: CoOccurrenceKind,
    evidence: &str,
) {
    let mut unique = members.to_vec();
    unique.sort();
    unique.dedup();
    for (index, left) in unique.iter().enumerate() {
        for right in unique.iter().skip(index + 1) {
            if let Some((a, b)) = ordered_pair(*left, *right) {
                pairs.push(CoOccurrence {
                    left: a,
                    right: b,
                    kind,
                    evidence: evidence.to_string(),
                });
            }
        }
    }
}

/// True when two events share a closed interval (or an instant treated as a zero-width interval).
pub fn intervals_overlap(
    left_start: chrono::DateTime<chrono::Utc>,
    left_end: Option<chrono::DateTime<chrono::Utc>>,
    right_start: chrono::DateTime<chrono::Utc>,
    right_end: Option<chrono::DateTime<chrono::Utc>>,
) -> bool {
    let left_end = left_end.unwrap_or(left_start);
    let right_end = right_end.unwrap_or(right_start);
    left_start <= right_end && right_start <= left_end
}

pub fn event_involves(event: &Event, data: &InvestigationData, entity_id: Uuid) -> bool {
    let members = data.member_ids_of(entity_id);
    event
        .entity_ids
        .iter()
        .any(|id| members.contains(&data.canonical_id(*id)) || members.contains(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        Entity, EntityKind, Event, Fields, Investigation, Provenance, Source, SourceKind,
        TimelineEvent, TimelineKind, now,
    };
    use chrono::{TimeZone, Utc};

    #[test]
    fn empty_investigation_returns_no_pairs() {
        let data = InvestigationData::empty(sample_investigation());
        let actual = co_occur(&data, &CoOccurrenceQuery::default());
        assert!(actual.is_empty());
    }

    #[test]
    fn fixture_returns_event_source_and_window_pairs() {
        let (data, alpha, beta, gamma) = fixture();
        let actual = co_occur(&data, &CoOccurrenceQuery::default());
        assert!(
            actual.iter().all(|pair| pair.left < pair.right),
            "left must be the lesser canonical UUID {actual:?}"
        );
        let (ab_left, ab_right) = ordered_pair(alpha, beta).expect("alpha != beta");
        let (ag_left, ag_right) = ordered_pair(alpha, gamma).expect("alpha != gamma");
        let mut expected = vec![
            (ab_left, ab_right, CoOccurrenceKind::SharedEvent.as_str()),
            (ag_left, ag_right, CoOccurrenceKind::SharedSource.as_str()),
            (ab_left, ab_right, CoOccurrenceKind::SharedWindow.as_str()),
        ];
        expected.sort();
        let keys: Vec<_> = actual
            .iter()
            .map(|pair| (pair.left, pair.right, pair.kind.as_str()))
            .collect();
        assert_eq!(keys, expected, "pairs must match and already be sorted");
        assert!(
            actual
                .windows(2)
                .all(|window| cmp_pair(&window[0], &window[1]) != std::cmp::Ordering::Greater)
        );
        let filtered = co_occur(
            &data,
            &CoOccurrenceQuery {
                entity_ids: vec![alpha, beta],
                window: None,
            },
        );
        assert!(filtered.iter().all(|pair| {
            pair.left < pair.right && Some((pair.left, pair.right)) == ordered_pair(alpha, beta)
        }));
        assert!(
            filtered
                .windows(2)
                .all(|window| cmp_pair(&window[0], &window[1]) != std::cmp::Ordering::Greater)
        );
    }

    #[test]
    fn intervals_overlap_closed_bounds() {
        let start = Utc.with_ymd_and_hms(2024, 1, 1, 10, 0, 0).unwrap();
        let mid = Utc.with_ymd_and_hms(2024, 1, 1, 11, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
        assert!(intervals_overlap(start, Some(end), mid, None));
        assert!(!intervals_overlap(
            start,
            Some(mid),
            end,
            Some(end + Duration::hours(1))
        ));
    }

    fn sample_investigation() -> Investigation {
        Investigation {
            id: Uuid::new_v4(),
            name: "cooccur".into(),
            description: String::new(),
            tags: Vec::new(),
            marking: crate::model::Marking::Unmarked,
            created_at: now(),
            updated_at: now(),
        }
    }

    fn fixture() -> (InvestigationData, Uuid, Uuid, Uuid) {
        let investigation = sample_investigation();
        let t0 = Utc.with_ymd_and_hms(2024, 5, 1, 12, 0, 0).unwrap();
        let t1 = t0 + Duration::hours(1);
        let alpha = sample_entity(&investigation, 1, "192.0.2.1");
        let beta = sample_entity(&investigation, 2, "192.0.2.2");
        let gamma = sample_entity(&investigation, 3, "192.0.2.3");
        let source = Source {
            id: Uuid::new_v4(),
            investigation_id: investigation.id,
            seq: 1,
            kind: SourceKind::Manual,
            title: "shared note".into(),
            url: None,
            retrieved_at: t0,
            last_seen_at: None,
            published_at: None,
            author: None,
            collector: None,
            content_hash: None,
            snapshot_path: None,
            content: String::new(),
            fields: Fields::new(),
            notes: String::new(),
            provenance: Provenance::Analyst,
            page_id: None,
            job_id: None,
        };
        let mut alpha_e = alpha.clone();
        alpha_e.source_ids = vec![source.id];
        let mut gamma_e = gamma.clone();
        gamma_e.source_ids = vec![source.id];
        let event = Event {
            id: Uuid::new_v4(),
            investigation_id: investigation.id,
            seq: 1,
            kind: TimelineKind::Observed,
            title: "joint window".into(),
            description: String::new(),
            started_at: t0,
            ended_at: Some(t1),
            provenance: Provenance::External,
            source_id: None,
            page_id: None,
            fields: Fields::new(),
            entity_ids: vec![alpha.id, beta.id],
            created_at: t0,
        };
        let timeline = TimelineEvent {
            id: Uuid::new_v4(),
            investigation_id: investigation.id,
            seq: 1,
            occurred_at: t0 + Duration::minutes(30),
            kind: TimelineKind::Observed,
            title: "beta seen".into(),
            description: String::new(),
            provenance: Provenance::Collected,
            source_id: None,
            entity_id: Some(beta.id),
            page_id: None,
            created_at: t0,
        };
        let mut data = InvestigationData::empty(investigation);
        let alpha_id = alpha_e.id;
        let beta_id = beta.id;
        let gamma_id = gamma_e.id;
        data.entities = vec![alpha_e, beta, gamma_e];
        data.sources = vec![source];
        data.events = vec![event];
        data.timeline = vec![timeline];
        (data, alpha_id, beta_id, gamma_id)
    }

    fn sample_entity(investigation: &Investigation, seq: i64, value: &str) -> Entity {
        Entity {
            id: Uuid::new_v4(),
            investigation_id: investigation.id,
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
