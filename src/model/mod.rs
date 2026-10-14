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
pub mod batch;
pub mod common;
pub mod entity;
pub mod finding;
pub mod graph;
pub mod ids;
pub mod investigation;
pub mod job;
pub mod note;
pub mod ontology;
pub mod page;
pub mod relationship;
pub mod source;
pub mod target;
pub mod timeline;
pub mod workspace;

pub use agent::{AgentAction, AgentRun, Proposal, ProposalStatus};
pub use artifact::{Artifact, ArtifactKind};
pub use batch::{
    EntityKey, EvidenceBatch, NewArtifact, NewEntity, NewRelationship, NewSource, NewTimelineEvent,
    SourceKey,
};
pub use common::{Confidence, Fields, Provenance, Timestamp, now};
pub use entity::{Entity, EntityKind};
pub use finding::Finding;
pub use graph::{
    DEFAULT_EDGE_CAP, DEFAULT_GRAPH_HOPS, DEFAULT_NODE_CAP, DEFAULT_SEARCH_HOPS, FacetAxis,
    FacetChip, GraphEdge, GraphFacets, GraphFilters, GraphNode, MAX_SEARCH_HOPS, SearchAroundQuery,
    SearchAroundResult, VisibleGraph, apply_graph_filters, facet_chips, graph_facets,
    layout_columns, merge_search_results, offset_hops, search_around,
};
pub use ids::{RefKind, ShortRef, find_refs, parse_ref_list};
pub use investigation::{Investigation, InvestigationSummary};
pub use job::{JobRecord, JobStatus};
pub use note::Note;
pub use ontology::{
    ACTION_TYPES, ActionType, CataloguedLink, LINK_TYPES, LinkDirection, LinkType, OBJECT_TYPES,
    ObjectType, ObjectView, action_type, inverse_of, link_group_label, link_type, local_actor,
    object_type,
};
pub use page::{Page, PageKind};
pub use relationship::{Relationship, kinds as relation, normalize_relationship_kind};
pub use source::{Source, SourceKind};
pub use target::{Target, TargetError, TargetKind};
pub use timeline::{TimelineEvent, TimelineKind};
pub use workspace::{InvestigationData, PageEntityLink, Workspace};
