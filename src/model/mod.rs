//! Domain model. These types are independent of the terminal UI and of storage.
//!
//! An [`Investigation`] owns [`Page`]s. Collectors attach [`Source`]s, [`Entity`] values,
//! [`Relationship`]s, [`Finding`]s, and [`TimelineEvent`]s. Short ids (`S-12`) come from
//! [`ShortRef`] and are scoped to one investigation; the UUID is the primary key.
//!
//! Provenance ([`Provenance`]) is stored on the records analysts read: `collected`, `external`,
//! `analyst`, and `agent`. Agent inference stays `agent` after a proposal is accepted.

pub mod agent;
pub mod artifact;
pub mod audit;
pub mod batch;
pub mod collaboration;
pub mod common;
pub mod cooccur;
pub mod entity;
pub mod event;
pub mod finding;
pub mod geo;
pub mod graph;
pub mod identity;
pub mod ids;
pub mod investigation;
pub mod job;
pub mod marking;
pub mod note;
pub mod ontology;
pub mod page;
pub mod playbook;
pub mod relationship;
pub mod resolution;
pub mod source;
pub mod target;
pub mod time;
pub mod timeline;
pub mod workspace;

pub use agent::{AgentAction, AgentRun, Proposal, ProposalStatus};
pub use artifact::{Artifact, ArtifactKind};
pub use audit::{AuditEntry, AuditKind};
pub use batch::{
    EntityKey, EvidenceBatch, NewArtifact, NewCollectedFinding, NewEntity, NewRelationship,
    NewSource, NewTimelineEvent, SourceKey,
};
pub use collaboration::{CONFLICT_POLICY, Member, Role, same_user};
pub use common::{Confidence, Fields, Provenance, Timestamp, now};
pub use cooccur::{
    CoOccurrence, CoOccurrenceKind, CoOccurrenceQuery, DEFAULT_WINDOW, co_occur, event_involves,
    intervals_overlap,
};
pub use entity::{Entity, EntityKind};
pub use event::{Event, NewEvent};
pub use finding::Finding;
pub use geo::{
    GeoDerivation, GeoJsonFeatureCollection, GeoLocation, GeoPoint, MAX_LATITUDE, MAX_LONGITUDE,
    MIN_LATITUDE, MIN_LONGITUDE, annotate_geo_fields, country_centroid, entity_kinds_with_geo,
    geojson_collection, nearby_entities, normalize_country, parse_ip, parse_latitude,
    parse_latitude_json, parse_longitude, parse_longitude_json, point_from_fields,
};
pub use graph::{
    DEFAULT_EDGE_CAP, DEFAULT_GRAPH_HOPS, DEFAULT_NODE_CAP, DEFAULT_SEARCH_HOPS, FacetAxis,
    FacetChip, GraphEdge, GraphFacets, GraphFilters, GraphNode, MAX_SEARCH_HOPS, SearchAroundQuery,
    SearchAroundResult, VisibleGraph, apply_graph_filters, facet_chips, graph_facets,
    layout_columns, merge_search_results, offset_hops, search_around,
};
pub use identity::Identity;
pub use ids::{RefKind, ShortRef, find_refs, parse_ref_list};
pub use investigation::{Investigation, InvestigationSummary};
pub use job::{JobRecord, JobStatus};
pub use marking::{Marking, MarkingError, OBJECT_MARKING_FIELD};
pub use note::Note;
pub use ontology::{
    ACTION_TYPES, ActionType, CataloguedLink, LINK_TYPES, LinkDirection, LinkType, OBJECT_TYPES,
    ObjectType, ObjectView, action_type, inverse_of, link_group_label, link_type, local_actor,
    object_type,
};
pub use page::{Page, PageKind};
pub use playbook::{
    Automation, MAX_COLLECT_STEPS, MAX_CONCURRENT_PLAYBOOK_JOBS, MAX_PLAYBOOK_BYTES,
    MAX_PLAYBOOK_PROPOSALS, MAX_PLAYBOOK_STEPS, MAX_TOOL_STEPS, PLAYBOOK_FORMAT,
    PLAYBOOK_FORMAT_VERSION, PlannedAction, Playbook, PlaybookOrigin, PlaybookOutput, PlaybookPlan,
    PlaybookRun, PlaybookSeed, PlaybookStep,
};
pub use relationship::{Relationship, kinds as relation, normalize_relationship_kind};
pub use resolution::{
    LABEL_WRITE_KEY, NOTES_WRITE_KEY, PropertyWrite, Resolution, ResolutionStatus,
    ResolutionSuggestion, SuggestionRule, new_resolution, plan_property_writes, plan_source_writes,
    suggest_resolutions,
};
pub use source::{Source, SourceKind};
pub use target::{Target, TargetError, TargetKind, is_subdomain_of};
pub use time::parse_timestamp_untrusted;
pub use timeline::{TimelineEvent, TimelineKind, TimelineRow, TimelineRowOrigin, overlay_timeline};
pub use workspace::{InvestigationData, PageEntityLink, Workspace, WorkspaceDelta};
