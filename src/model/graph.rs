//! Search Around and graph visibility over an investigation snapshot.
//!
//! The algorithm is pure: it walks [`InvestigationData`] and never touches SQLite. Storage
//! loads a snapshot and calls [`search_around`]. The Graph screen filters that result with
//! [`apply_graph_filters`] and lays nodes out by hop for a terminal canvas.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use super::common::{Provenance, Timestamp};
use super::entity::{Entity, EntityKind};
use super::ontology::{OTHER_LINK_GROUP, inverse_of, link_group_label};
use super::relationship::{Relationship, normalize_relationship_kind};
use super::workspace::InvestigationData;

/// Default Search Around depth (1–2 hops as specified for Phase 2).
pub const DEFAULT_SEARCH_HOPS: u32 = 2;
/// Depth used when first opening Graph (immediate neighborhood).
pub const DEFAULT_GRAPH_HOPS: u32 = 1;
/// Hard ceiling so a hop form cannot run away.
pub const MAX_SEARCH_HOPS: u32 = 8;
/// Maximum nodes rendered without an explicit confirm.
pub const DEFAULT_NODE_CAP: usize = 200;
/// Maximum edges rendered without an explicit confirm.
pub const DEFAULT_EDGE_CAP: usize = 400;

/// Filters and bounds for one Search Around expansion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchAroundQuery {
    pub seed_ids: Vec<Uuid>,
    pub hops: u32,
    pub link_types: Vec<String>,
    pub entity_kinds: Vec<EntityKind>,
    pub provenances: Vec<Provenance>,
    pub since: Option<Timestamp>,
    pub until: Option<Timestamp>,
    pub node_cap: usize,
    pub edge_cap: usize,
}

impl SearchAroundQuery {
    pub fn new(seed_ids: Vec<Uuid>, hops: u32) -> Self {
        Self {
            seed_ids,
            hops,
            link_types: Vec::new(),
            entity_kinds: Vec::new(),
            provenances: Vec::new(),
            since: None,
            until: None,
            node_cap: DEFAULT_NODE_CAP,
            edge_cap: DEFAULT_EDGE_CAP,
        }
    }

    pub fn clamped_hops(&self) -> u32 {
        self.hops.min(MAX_SEARCH_HOPS)
    }

    pub fn node_cap(&self) -> usize {
        self.node_cap.max(1)
    }

    pub fn edge_cap(&self) -> usize {
        self.edge_cap.max(1)
    }
}

/// One entity reached by Search Around, with the shortest hop from a seed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphNode {
    pub id: Uuid,
    pub hop: u32,
}

/// One relationship included in a Search Around result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphEdge {
    pub id: Uuid,
    pub from_id: Uuid,
    pub to_id: Uuid,
    pub kind: String,
    pub discovered_at_hop: u32,
}

/// The neighborhood produced by [`search_around`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchAroundResult {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub hops_used: u32,
    pub uncapped_node_count: usize,
    pub uncapped_edge_count: usize,
    pub truncated: bool,
    pub seed_ids: Vec<Uuid>,
    pub node_cap: usize,
    pub edge_cap: usize,
}

impl Default for SearchAroundResult {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            hops_used: 0,
            uncapped_node_count: 0,
            uncapped_edge_count: 0,
            truncated: false,
            seed_ids: Vec::new(),
            node_cap: DEFAULT_NODE_CAP,
            edge_cap: DEFAULT_EDGE_CAP,
        }
    }
}

impl SearchAroundResult {
    pub fn requires_confirm(&self) -> bool {
        self.uncapped_node_count > self.node_cap || self.uncapped_edge_count > self.edge_cap
    }

    pub fn node_ids(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.nodes.iter().map(|node| node.id)
    }

    pub fn hop_of(&self, id: Uuid) -> Option<u32> {
        self.nodes
            .iter()
            .find(|node| node.id == id)
            .map(|node| node.hop)
    }
}

/// Facet / histogram counts for the current graph set (before a facet is applied).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphFacets {
    pub kinds: Vec<FacetCount>,
    pub link_types: Vec<FacetCount>,
    pub provenances: Vec<FacetCount>,
}

/// One chip in the facet strip: a key and how many records carry it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FacetCount {
    pub key: String,
    pub label: String,
    pub count: usize,
}

/// Analyst-applied visibility filters on an already-computed Search Around result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphFilters {
    pub kind: Option<String>,
    pub link_type: Option<String>,
    pub provenance: Option<Provenance>,
}

impl GraphFilters {
    pub fn is_empty(&self) -> bool {
        self.kind.is_none() && self.link_type.is_none() && self.provenance.is_none()
    }
}

/// The subset of a Search Around result that the Graph screen should draw.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VisibleGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

impl VisibleGraph {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// One selectable chip in the facet strip (kind, link type, or provenance).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FacetChip {
    pub axis: FacetAxis,
    pub key: String,
    pub label: String,
    pub count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FacetAxis {
    Kind,
    Link,
    Provenance,
}

/// Expands from `query.seed_ids` up to `query.hops` along allowed relationships.
///
/// Traversal is undirected (incoming and outgoing). Seeds that are missing from `data` are
/// dropped. Empty seeds or an empty investigation produce an empty result — never a panic.
pub fn search_around(data: &InvestigationData, query: &SearchAroundQuery) -> SearchAroundResult {
    let hops = query.clamped_hops();
    let node_cap = query.node_cap();
    let edge_cap = query.edge_cap();
    let entities = entity_index(data);
    let remapped: Vec<Uuid> = query
        .seed_ids
        .iter()
        .map(|id| data.canonical_id(*id))
        .collect();
    let seeds = resolve_seeds(&remapped, &entities);
    if seeds.is_empty() {
        return SearchAroundResult {
            hops_used: hops,
            node_cap,
            edge_cap,
            ..SearchAroundResult::default()
        };
    }
    let allowed = Allowed::from_query(query);
    let adjacency = build_adjacency(data, &allowed);
    let hop_of = walk_hops(&seeds, hops, &adjacency, &entities, &allowed);
    let edges = induced_edges(data, &allowed, &hop_of);
    let uncapped_node_count = hop_of.len();
    let uncapped_edge_count = edges.len();
    let (nodes, truncated_nodes) = cap_nodes(hop_of, &entities, &seeds, node_cap);
    let kept: HashSet<Uuid> = nodes.iter().map(|node| node.id).collect();
    let (edges, truncated_edges) = cap_edges(edges, &kept, edge_cap);
    SearchAroundResult {
        nodes,
        edges,
        hops_used: hops,
        uncapped_node_count,
        uncapped_edge_count,
        truncated: truncated_nodes || truncated_edges,
        seed_ids: seeds,
        node_cap,
        edge_cap,
    }
}

/// Unions two Search Around results, keeping the shortest hop for each node.
///
/// Caps from `base` are reused. `truncated` is set when the union exceeds those caps or
/// either input was already truncated.
pub fn merge_search_results(
    base: &SearchAroundResult,
    extra: &SearchAroundResult,
) -> SearchAroundResult {
    let mut hop_of: HashMap<Uuid, u32> =
        base.nodes.iter().map(|node| (node.id, node.hop)).collect();
    for node in &extra.nodes {
        hop_of
            .entry(node.id)
            .and_modify(|hop| *hop = (*hop).min(node.hop))
            .or_insert(node.hop);
    }
    let mut edges: HashMap<Uuid, GraphEdge> = base
        .edges
        .iter()
        .map(|edge| (edge.id, edge.clone()))
        .collect();
    for edge in &extra.edges {
        edges.entry(edge.id).or_insert_with(|| edge.clone());
    }
    let mut seed_ids = base.seed_ids.clone();
    for seed in &extra.seed_ids {
        if !seed_ids.contains(seed) {
            seed_ids.push(*seed);
        }
    }
    let node_cap = base.node_cap.max(1);
    let edge_cap = base.edge_cap.max(1);
    let uncapped_node_count = hop_of.len();
    let uncapped_edge_count = edges.len();
    let mut nodes: Vec<GraphNode> = hop_of
        .into_iter()
        .map(|(id, hop)| GraphNode { id, hop })
        .collect();
    nodes.sort_by_key(|node| (node.hop, node.id));
    let truncated_nodes = nodes.len() > node_cap;
    nodes.truncate(node_cap);
    let kept: HashSet<Uuid> = nodes.iter().map(|node| node.id).collect();
    let mut merged_edges: Vec<GraphEdge> = edges.into_values().collect();
    merged_edges.sort_by_key(|edge| edge.id);
    let (edges, truncated_edges) = cap_edges(merged_edges, &kept, edge_cap);
    SearchAroundResult {
        nodes,
        edges,
        hops_used: base.hops_used.max(extra.hops_used),
        uncapped_node_count,
        uncapped_edge_count,
        truncated: base.truncated || extra.truncated || truncated_nodes || truncated_edges,
        seed_ids,
        node_cap,
        edge_cap,
    }
}

/// Offsets every hop (and discovered-at hop) by `offset`, used when merging an expansion
/// that started at a node already on the graph.
pub fn offset_hops(result: &SearchAroundResult, offset: u32) -> SearchAroundResult {
    let mut shifted = result.clone();
    for node in &mut shifted.nodes {
        node.hop = node.hop.saturating_add(offset);
    }
    for edge in &mut shifted.edges {
        edge.discovered_at_hop = edge.discovered_at_hop.saturating_add(offset);
    }
    shifted.hops_used = shifted.hops_used.saturating_add(offset);
    shifted
}

/// Restricts a Search Around result to the selected facet keys. Seeds are not privileged:
/// a kind filter of `ip` hides a domain seed.
pub fn apply_graph_filters(
    data: &InvestigationData,
    result: &SearchAroundResult,
    filters: &GraphFilters,
) -> VisibleGraph {
    let entities = entity_index(data);
    let candidates: Vec<GraphNode> = result
        .nodes
        .iter()
        .filter(|node| node_matches_filters(entities.get(&node.id).copied(), filters))
        .cloned()
        .collect();
    let candidate_ids: HashSet<Uuid> = candidates.iter().map(|node| node.id).collect();
    let link_kinds = filters
        .link_type
        .as_deref()
        .map(|kind| expand_link_kinds(std::iter::once(kind)));
    let edges: Vec<GraphEdge> = result
        .edges
        .iter()
        .filter(|edge| {
            candidate_ids.contains(&edge.from_id)
                && candidate_ids.contains(&edge.to_id)
                && link_kinds
                    .as_ref()
                    .is_none_or(|kinds| kinds.contains(&normalize_relationship_kind(&edge.kind)))
        })
        .cloned()
        .collect();
    let nodes = if link_kinds.is_some() {
        let incident: HashSet<Uuid> = edges
            .iter()
            .flat_map(|edge| [edge.from_id, edge.to_id])
            .collect();
        candidates
            .into_iter()
            .filter(|node| incident.contains(&node.id))
            .collect()
    } else {
        candidates
    };
    VisibleGraph { nodes, edges }
}

/// Histogram counts for the unfiltered Search Around set (so chips stay stable as filters change).
pub fn graph_facets(data: &InvestigationData, result: &SearchAroundResult) -> GraphFacets {
    let entities = entity_index(data);
    let mut kinds: HashMap<String, usize> = HashMap::new();
    let mut provenances: HashMap<String, usize> = HashMap::new();
    for node in &result.nodes {
        let Some(entity) = entities.get(&node.id) else {
            continue;
        };
        *kinds.entry(entity.kind.as_str().to_string()).or_insert(0) += 1;
        *provenances
            .entry(entity.provenance.as_str().to_string())
            .or_insert(0) += 1;
    }
    let mut links: HashMap<String, usize> = HashMap::new();
    for edge in &result.edges {
        *links.entry(edge.kind.clone()).or_insert(0) += 1;
    }
    GraphFacets {
        kinds: sort_counts(kinds, entity_kind_label),
        link_types: sort_counts(links, |key| {
            let label = link_group_label(key);
            if label == OTHER_LINK_GROUP {
                format!("{OTHER_LINK_GROUP} ({key})")
            } else {
                label.to_string()
            }
        }),
        provenances: sort_counts(provenances, provenance_label),
    }
}

/// Flattened facet chips in a stable order: kinds, then link types, then provenance.
pub fn facet_chips(facets: &GraphFacets) -> Vec<FacetChip> {
    let mut chips = Vec::new();
    for count in &facets.kinds {
        chips.push(FacetChip {
            axis: FacetAxis::Kind,
            key: count.key.clone(),
            label: count.label.clone(),
            count: count.count,
        });
    }
    for count in &facets.link_types {
        chips.push(FacetChip {
            axis: FacetAxis::Link,
            key: count.key.clone(),
            label: count.label.clone(),
            count: count.count,
        });
    }
    for count in &facets.provenances {
        chips.push(FacetChip {
            axis: FacetAxis::Provenance,
            key: count.key.clone(),
            label: count.label.clone(),
            count: count.count,
        });
    }
    chips
}

/// Groups visible nodes into hop columns for the terminal layout (left = seeds).
pub fn layout_columns(nodes: &[GraphNode]) -> Vec<(u32, Vec<Uuid>)> {
    if nodes.is_empty() {
        return Vec::new();
    }
    let max_hop = nodes.iter().map(|node| node.hop).max().unwrap_or(0);
    (0..=max_hop)
        .filter_map(|hop| {
            let ids: Vec<Uuid> = nodes
                .iter()
                .filter(|node| node.hop == hop)
                .map(|node| node.id)
                .collect();
            if ids.is_empty() {
                None
            } else {
                Some((hop, ids))
            }
        })
        .collect()
}

struct Allowed {
    link_types: HashSet<String>,
    entity_kinds: HashSet<EntityKind>,
    provenances: HashSet<Provenance>,
    since: Option<Timestamp>,
    until: Option<Timestamp>,
}

impl Allowed {
    fn from_query(query: &SearchAroundQuery) -> Self {
        Self {
            link_types: expand_link_kinds(query.link_types.iter()),
            entity_kinds: query.entity_kinds.iter().cloned().collect(),
            provenances: query.provenances.iter().copied().collect(),
            since: query.since,
            until: query.until,
        }
    }

    fn edge(&self, relationship: &Relationship) -> bool {
        if !self.link_types.is_empty() && !self.link_types.contains(&relationship.kind) {
            return false;
        }
        if !self.provenances.is_empty() && !self.provenances.contains(&relationship.provenance) {
            return false;
        }
        if self
            .since
            .is_some_and(|since| relationship.created_at < since)
        {
            return false;
        }
        if self
            .until
            .is_some_and(|until| relationship.created_at > until)
        {
            return false;
        }
        true
    }

    fn neighbor(&self, entity: &Entity, is_seed: bool) -> bool {
        if is_seed || self.entity_kinds.is_empty() {
            return true;
        }
        self.entity_kinds.contains(&entity.kind)
    }
}

fn entity_index(data: &InvestigationData) -> HashMap<Uuid, &Entity> {
    data.entities
        .iter()
        .map(|entity| (entity.id, entity))
        .collect()
}

fn resolve_seeds(seed_ids: &[Uuid], entities: &HashMap<Uuid, &Entity>) -> Vec<Uuid> {
    let mut seen = HashSet::new();
    let mut seeds = Vec::new();
    for id in seed_ids {
        if entities.contains_key(id) && seen.insert(*id) {
            seeds.push(*id);
        }
    }
    seeds
}

fn build_adjacency<'a>(
    data: &'a InvestigationData,
    allowed: &Allowed,
) -> HashMap<Uuid, Vec<(&'a Relationship, Uuid)>> {
    let mut adjacency: HashMap<Uuid, Vec<(&Relationship, Uuid)>> = HashMap::new();
    for relationship in &data.relationships {
        if !allowed.edge(relationship) {
            continue;
        }
        let from_id = data.canonical_id(relationship.from_id);
        let to_id = data.canonical_id(relationship.to_id);
        if from_id == to_id {
            continue;
        }
        adjacency
            .entry(from_id)
            .or_default()
            .push((relationship, to_id));
        adjacency
            .entry(to_id)
            .or_default()
            .push((relationship, from_id));
    }
    for neighbors in adjacency.values_mut() {
        neighbors.sort_by_key(|(relationship, neighbor)| (relationship.seq, *neighbor));
    }
    adjacency
}

fn expand_link_kinds<S: AsRef<str>>(kinds: impl IntoIterator<Item = S>) -> HashSet<String> {
    let mut expanded = HashSet::new();
    for kind in kinds {
        let kind = normalize_relationship_kind(kind.as_ref());
        if kind.is_empty() {
            continue;
        }
        if let Some(inverse) = inverse_of(&kind) {
            expanded.insert(inverse.to_string());
        }
        expanded.insert(kind);
    }
    expanded
}

fn walk_hops(
    seeds: &[Uuid],
    hops: u32,
    adjacency: &HashMap<Uuid, Vec<(&Relationship, Uuid)>>,
    entities: &HashMap<Uuid, &Entity>,
    allowed: &Allowed,
) -> HashMap<Uuid, u32> {
    let mut hop_of: HashMap<Uuid, u32> = seeds.iter().copied().map(|id| (id, 0)).collect();
    let mut frontier: Vec<Uuid> = seeds.to_vec();
    let seed_set: HashSet<Uuid> = seeds.iter().copied().collect();
    for hop in 1..=hops {
        let mut next = Vec::new();
        for node in &frontier {
            let Some(neighbors) = adjacency.get(node) else {
                continue;
            };
            for (_relationship, neighbor) in neighbors {
                let Some(entity) = entities.get(neighbor) else {
                    continue;
                };
                if !allowed.neighbor(entity, seed_set.contains(neighbor)) {
                    continue;
                }
                if hop_of.contains_key(neighbor) {
                    continue;
                }
                hop_of.insert(*neighbor, hop);
                next.push(*neighbor);
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    hop_of
}

/// Every allowed edge whose endpoints were reached — the induced subgraph, including
/// same-hop sibling links that BFS never traversed as a discovery edge.
fn induced_edges(
    data: &InvestigationData,
    allowed: &Allowed,
    hop_of: &HashMap<Uuid, u32>,
) -> Vec<GraphEdge> {
    let mut edges = Vec::new();
    for relationship in &data.relationships {
        if !allowed.edge(relationship) {
            continue;
        }
        let from_id = data.canonical_id(relationship.from_id);
        let to_id = data.canonical_id(relationship.to_id);
        if from_id == to_id {
            continue;
        }
        let Some(&from_hop) = hop_of.get(&from_id) else {
            continue;
        };
        let Some(&to_hop) = hop_of.get(&to_id) else {
            continue;
        };
        edges.push(GraphEdge {
            id: relationship.id,
            from_id,
            to_id,
            kind: relationship.kind.clone(),
            discovered_at_hop: from_hop.max(to_hop),
        });
    }
    edges
}

fn cap_nodes(
    hop_of: HashMap<Uuid, u32>,
    entities: &HashMap<Uuid, &Entity>,
    seeds: &[Uuid],
    node_cap: usize,
) -> (Vec<GraphNode>, bool) {
    let seed_set: HashSet<Uuid> = seeds.iter().copied().collect();
    let mut nodes: Vec<GraphNode> = hop_of
        .into_iter()
        .map(|(id, hop)| GraphNode { id, hop })
        .collect();
    nodes.sort_by_key(|node| {
        let seed_rank = if seed_set.contains(&node.id) { 0 } else { 1 };
        let seq = entities
            .get(&node.id)
            .map(|entity| entity.seq)
            .unwrap_or(i64::MAX);
        (seed_rank, node.hop, seq, node.id)
    });
    let truncated = nodes.len() > node_cap;
    nodes.truncate(node_cap);
    (nodes, truncated)
}

fn cap_edges(
    mut edges: Vec<GraphEdge>,
    kept: &HashSet<Uuid>,
    edge_cap: usize,
) -> (Vec<GraphEdge>, bool) {
    edges.retain(|edge| kept.contains(&edge.from_id) && kept.contains(&edge.to_id));
    edges.sort_by_key(|edge| (edge.discovered_at_hop, edge.id));
    let truncated = edges.len() > edge_cap;
    edges.truncate(edge_cap);
    (edges, truncated)
}

fn node_matches_filters(entity: Option<&Entity>, filters: &GraphFilters) -> bool {
    let Some(entity) = entity else {
        return false;
    };
    if filters
        .kind
        .as_deref()
        .is_some_and(|kind| entity.kind.as_str() != kind)
    {
        return false;
    }
    if filters
        .provenance
        .is_some_and(|provenance| entity.provenance != provenance)
    {
        return false;
    }
    true
}

fn sort_counts(counts: HashMap<String, usize>, label: impl Fn(&str) -> String) -> Vec<FacetCount> {
    let mut items: Vec<FacetCount> = counts
        .into_iter()
        .map(|(key, count)| FacetCount {
            label: label(&key),
            key,
            count,
        })
        .collect();
    items.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.key.cmp(&right.key))
    });
    items
}

fn entity_kind_label(key: &str) -> String {
    EntityKind::parse(key).label().to_string()
}

fn provenance_label(key: &str) -> String {
    Provenance::parse(key)
        .map(|provenance| provenance.badge().to_string())
        .unwrap_or_else(|| key.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        Confidence, Entity, Fields, Investigation, InvestigationData, Provenance, Relationship, now,
    };

    fn investigation() -> Investigation {
        Investigation {
            id: Uuid::new_v4(),
            name: "Fixture".into(),
            description: String::new(),
            tags: Vec::new(),
            created_at: now(),
            updated_at: now(),
        }
    }

    fn entity(kind: EntityKind, value: &str, seq: i64, provenance: Provenance) -> Entity {
        let investigation_id = Uuid::new_v4();
        Entity {
            id: Uuid::new_v4(),
            investigation_id,
            seq,
            kind,
            value: value.into(),
            label: String::new(),
            notes: String::new(),
            provenance,
            fields: Fields::new(),
            source_ids: Vec::new(),
            created_at: now(),
            updated_at: now(),
        }
    }

    fn edge(
        from: &Entity,
        kind: &str,
        to: &Entity,
        seq: i64,
        created_at: Timestamp,
        provenance: Provenance,
    ) -> Relationship {
        Relationship {
            id: Uuid::new_v4(),
            investigation_id: from.investigation_id,
            seq,
            from_id: from.id,
            to_id: to.id,
            kind: kind.into(),
            provenance,
            confidence: Confidence::High,
            notes: String::new(),
            source_ids: Vec::new(),
            created_at,
        }
    }

    /// Domain → IP → ASN, plus a nameserver hop-2 IP, and an isolated person.
    fn infra_fixture() -> (
        InvestigationData,
        Entity,
        Entity,
        Entity,
        Entity,
        Entity,
        Entity,
    ) {
        let domain = entity(EntityKind::Domain, "example.com", 1, Provenance::Collected);
        let ip = entity(EntityKind::Ip, "192.0.2.10", 2, Provenance::Collected);
        let nameserver = entity(
            EntityKind::Hostname,
            "ns1.example.net",
            3,
            Provenance::Collected,
        );
        let asn = entity(EntityKind::Asn, "AS64496", 4, Provenance::Collected);
        let ns_ip = entity(EntityKind::Ip, "192.0.2.53", 5, Provenance::Collected);
        let isolated = entity(EntityKind::Person, "Alice", 6, Provenance::Analyst);
        let t0 = now();
        let t1 = t0 + chrono::Duration::seconds(60);
        let data = InvestigationData {
            investigation: investigation(),
            pages: Vec::new(),
            sources: Vec::new(),
            findings: Vec::new(),
            relationships: vec![
                edge(&domain, "resolves_to", &ip, 1, t0, Provenance::Collected),
                edge(
                    &domain,
                    "uses_nameserver",
                    &nameserver,
                    2,
                    t0,
                    Provenance::Collected,
                ),
                edge(&ip, "announced_by", &asn, 3, t1, Provenance::Collected),
                edge(
                    &nameserver,
                    "resolves_to",
                    &ns_ip,
                    4,
                    t1,
                    Provenance::External,
                ),
            ],
            entities: vec![
                domain.clone(),
                ip.clone(),
                nameserver.clone(),
                asn.clone(),
                ns_ip.clone(),
                isolated.clone(),
            ],
            notes: Vec::new(),
            artifacts: Vec::new(),
            timeline: Vec::new(),
            jobs: Vec::new(),
            agent_runs: Vec::new(),
            proposals: Vec::new(),
            page_entities: Vec::new(),
            resolutions: Vec::new(),
        };
        (data, domain, ip, nameserver, asn, ns_ip, isolated)
    }

    fn ids_at_hop(result: &SearchAroundResult, hop: u32) -> Vec<Uuid> {
        result
            .nodes
            .iter()
            .filter(|node| node.hop == hop)
            .map(|node| node.id)
            .collect()
    }

    #[test]
    fn empty_seeds_and_missing_ids_yield_empty_results() {
        let (data, ..) = infra_fixture();
        let empty = search_around(&data, &SearchAroundQuery::new(Vec::new(), 2));
        assert!(empty.nodes.is_empty());
        assert!(empty.edges.is_empty());
        assert!(!empty.truncated);
        let missing = search_around(&data, &SearchAroundQuery::new(vec![Uuid::new_v4()], 2));
        assert!(missing.nodes.is_empty());
        assert!(missing.seed_ids.is_empty());
    }

    #[test]
    fn hop_counts_follow_undirected_infra() {
        let (data, domain, ip, nameserver, asn, ns_ip, isolated) = infra_fixture();
        let one = search_around(&data, &SearchAroundQuery::new(vec![domain.id], 1));
        assert_eq!(one.hops_used, 1);
        assert_eq!(one.nodes.len(), 3);
        assert_eq!(ids_at_hop(&one, 0), vec![domain.id]);
        let hop1 = ids_at_hop(&one, 1);
        assert!(hop1.contains(&ip.id));
        assert!(hop1.contains(&nameserver.id));
        assert!(!hop1.contains(&asn.id));
        assert!(!one.node_ids().any(|id| id == isolated.id));
        let two = search_around(&data, &SearchAroundQuery::new(vec![domain.id], 2));
        assert_eq!(two.nodes.len(), 5);
        assert!(ids_at_hop(&two, 2).contains(&asn.id));
        assert!(ids_at_hop(&two, 2).contains(&ns_ip.id));
        assert!(!two.node_ids().any(|id| id == isolated.id));
        let zero = search_around(&data, &SearchAroundQuery::new(vec![domain.id], 0));
        assert_eq!(zero.nodes.len(), 1);
        assert_eq!(zero.nodes[0].id, domain.id);
        assert!(zero.edges.is_empty());
    }

    #[test]
    fn filters_restrict_link_types_kinds_provenance_and_time() {
        let (data, domain, ip, nameserver, asn, ns_ip, _) = infra_fixture();
        let mut query = SearchAroundQuery::new(vec![domain.id], 2);
        query.link_types = vec!["resolves_to".into()];
        let by_link = search_around(&data, &query);
        assert!(by_link.node_ids().any(|id| id == ip.id));
        assert!(!by_link.node_ids().any(|id| id == nameserver.id));
        assert!(!by_link.node_ids().any(|id| id == asn.id));
        query = SearchAroundQuery::new(vec![domain.id], 2);
        query.entity_kinds = vec![EntityKind::Ip];
        let by_kind = search_around(&data, &query);
        assert!(by_kind.node_ids().any(|id| id == domain.id));
        assert!(by_kind.node_ids().any(|id| id == ip.id));
        assert!(!by_kind.node_ids().any(|id| id == nameserver.id));
        assert!(!by_kind.node_ids().any(|id| id == asn.id));
        query = SearchAroundQuery::new(vec![nameserver.id], 1);
        query.provenances = vec![Provenance::External];
        let by_prov = search_around(&data, &query);
        assert_eq!(by_prov.edges.len(), 1);
        assert!(by_prov.node_ids().any(|id| id == ns_ip.id));
        assert!(!by_prov.node_ids().any(|id| id == domain.id));
        let t0 = data.relationships[0].created_at;
        let t1 = data.relationships[2].created_at;
        query = SearchAroundQuery::new(vec![domain.id], 2);
        query.until = Some(t0);
        let early = search_around(&data, &query);
        assert!(early.node_ids().any(|id| id == ip.id));
        assert!(early.node_ids().any(|id| id == nameserver.id));
        assert!(!early.node_ids().any(|id| id == asn.id));
        query = SearchAroundQuery::new(vec![ip.id], 1);
        query.since = Some(t1);
        query.until = None;
        let late = search_around(&data, &query);
        assert!(late.node_ids().any(|id| id == asn.id));
        assert!(!late.node_ids().any(|id| id == domain.id));
    }

    #[test]
    fn caps_truncate_and_require_confirm() {
        let (data, domain, ..) = infra_fixture();
        let mut query = SearchAroundQuery::new(vec![domain.id], 2);
        query.node_cap = 2;
        query.edge_cap = 400;
        let capped = search_around(&data, &query);
        assert_eq!(capped.nodes.len(), 2);
        assert_eq!(capped.uncapped_node_count, 5);
        assert!(capped.truncated);
        assert!(capped.requires_confirm());
        query.node_cap = 200;
        query.edge_cap = 1;
        let edge_capped = search_around(&data, &query);
        assert_eq!(edge_capped.edges.len(), 1);
        assert!(edge_capped.uncapped_edge_count > 1);
        assert!(edge_capped.requires_confirm());
        query.edge_cap = 400;
        let full = search_around(&data, &query);
        assert!(!full.requires_confirm());
        assert!(!full.truncated);
    }

    #[test]
    fn facet_filter_changes_visible_set() {
        let (data, domain, ip, nameserver, asn, ..) = infra_fixture();
        let result = search_around(&data, &SearchAroundQuery::new(vec![domain.id], 2));
        let facets = graph_facets(&data, &result);
        assert!(facets.kinds.iter().any(|item| item.key == "domain"));
        assert!(
            facets
                .link_types
                .iter()
                .any(|item| item.key == "resolves_to")
        );
        let only_ip = apply_graph_filters(
            &data,
            &result,
            &GraphFilters {
                kind: Some("ip".into()),
                ..GraphFilters::default()
            },
        );
        assert!(only_ip.nodes.iter().all(|node| {
            data.entities
                .iter()
                .find(|entity| entity.id == node.id)
                .is_some_and(|entity| entity.kind == EntityKind::Ip)
        }));
        assert!(!only_ip.nodes.iter().any(|node| node.id == domain.id));
        assert!(only_ip.nodes.iter().any(|node| node.id == ip.id));
        let only_announced = apply_graph_filters(
            &data,
            &result,
            &GraphFilters {
                link_type: Some("announced_by".into()),
                ..GraphFilters::default()
            },
        );
        assert_eq!(only_announced.edges.len(), 1);
        assert_eq!(only_announced.edges[0].kind, "announced_by");
        assert!(only_announced.nodes.iter().any(|node| node.id == asn.id));
        assert!(only_announced.nodes.iter().any(|node| node.id == ip.id));
        assert!(
            !only_announced
                .nodes
                .iter()
                .any(|node| node.id == nameserver.id)
        );
        assert!(!only_announced.nodes.iter().any(|node| node.id == domain.id));
        let chips = facet_chips(&facets);
        assert!(chips.iter().any(|chip| chip.axis == FacetAxis::Kind));
        assert!(chips.iter().any(|chip| chip.axis == FacetAxis::Link));
    }

    #[test]
    fn merge_and_offset_keep_shortest_hop() {
        let (data, domain, ip, _, asn, ..) = infra_fixture();
        let first = search_around(&data, &SearchAroundQuery::new(vec![domain.id], 1));
        let expansion = search_around(&data, &SearchAroundQuery::new(vec![ip.id], 1));
        let origin_hop = first.hop_of(ip.id).expect("ip on first hop");
        let merged = merge_search_results(&first, &offset_hops(&expansion, origin_hop));
        assert!(merged.node_ids().any(|id| id == asn.id));
        assert_eq!(merged.hop_of(asn.id), Some(2));
        assert_eq!(merged.hop_of(domain.id), Some(0));
        assert_eq!(merged.hop_of(ip.id), Some(1));
    }

    #[test]
    fn layout_columns_group_by_hop_and_tolerate_empty() {
        assert!(layout_columns(&[]).is_empty());
        let (data, domain, ..) = infra_fixture();
        let result = search_around(&data, &SearchAroundQuery::new(vec![domain.id], 2));
        let columns = layout_columns(&result.nodes);
        assert_eq!(columns.first().map(|(hop, _)| *hop), Some(0));
        assert_eq!(columns[0].1, vec![domain.id]);
        assert!(columns.iter().any(|(hop, _)| *hop == 2));
    }

    #[test]
    fn hops_are_clamped_to_the_ceiling() {
        let (data, domain, ..) = infra_fixture();
        let result = search_around(&data, &SearchAroundQuery::new(vec![domain.id], 99));
        assert_eq!(result.hops_used, MAX_SEARCH_HOPS);
        assert_eq!(result.nodes.len(), 5);
    }

    #[test]
    fn one_hop_includes_sibling_edges_between_neighbors() {
        let domain = entity(EntityKind::Domain, "example.com", 1, Provenance::Collected);
        let ip = entity(EntityKind::Ip, "192.0.2.10", 2, Provenance::Collected);
        let nameserver = entity(
            EntityKind::Hostname,
            "ns1.example.net",
            3,
            Provenance::Collected,
        );
        let t0 = now();
        let data = InvestigationData {
            investigation: investigation(),
            pages: Vec::new(),
            sources: Vec::new(),
            findings: Vec::new(),
            relationships: vec![
                edge(&domain, "resolves_to", &ip, 1, t0, Provenance::Collected),
                edge(
                    &domain,
                    "uses_nameserver",
                    &nameserver,
                    2,
                    t0,
                    Provenance::Collected,
                ),
                edge(
                    &ip,
                    "associated_with",
                    &nameserver,
                    3,
                    t0,
                    Provenance::Collected,
                ),
            ],
            entities: vec![domain.clone(), ip.clone(), nameserver.clone()],
            notes: Vec::new(),
            artifacts: Vec::new(),
            timeline: Vec::new(),
            jobs: Vec::new(),
            agent_runs: Vec::new(),
            proposals: Vec::new(),
            page_entities: Vec::new(),
            resolutions: Vec::new(),
        };
        let one = search_around(&data, &SearchAroundQuery::new(vec![domain.id], 1));
        assert_eq!(one.nodes.len(), 3);
        assert!(
            one.edges.iter().any(|edge| edge.kind == "associated_with"
                && ((edge.from_id == ip.id && edge.to_id == nameserver.id)
                    || (edge.from_id == nameserver.id && edge.to_id == ip.id))),
            "same-hop sibling edge must appear in the induced 1-hop subgraph"
        );
    }

    #[test]
    fn cycle_is_reachable_and_induced_includes_every_cycle_edge() {
        let alpha = entity(EntityKind::Domain, "a.example", 1, Provenance::Collected);
        let beta = entity(EntityKind::Domain, "b.example", 2, Provenance::Collected);
        let gamma = entity(EntityKind::Domain, "c.example", 3, Provenance::Collected);
        let t0 = now();
        let data = InvestigationData {
            investigation: investigation(),
            pages: Vec::new(),
            sources: Vec::new(),
            findings: Vec::new(),
            relationships: vec![
                edge(&alpha, "mentions", &beta, 1, t0, Provenance::Collected),
                edge(&beta, "mentions", &gamma, 2, t0, Provenance::Collected),
                edge(&gamma, "mentions", &alpha, 3, t0, Provenance::Collected),
            ],
            entities: vec![alpha.clone(), beta.clone(), gamma.clone()],
            notes: Vec::new(),
            artifacts: Vec::new(),
            timeline: Vec::new(),
            jobs: Vec::new(),
            agent_runs: Vec::new(),
            proposals: Vec::new(),
            page_entities: Vec::new(),
            resolutions: Vec::new(),
        };
        let one = search_around(&data, &SearchAroundQuery::new(vec![alpha.id], 1));
        assert_eq!(one.nodes.len(), 3);
        assert_eq!(one.edges.len(), 3);
        let two = search_around(&data, &SearchAroundQuery::new(vec![alpha.id], 2));
        assert_eq!(two.nodes.len(), 3);
        assert_eq!(two.edges.len(), 3);
    }

    #[test]
    fn link_type_filter_includes_catalog_inverse() {
        let domain = entity(EntityKind::Domain, "example.com", 1, Provenance::Collected);
        let ip = entity(EntityKind::Ip, "192.0.2.10", 2, Provenance::Collected);
        let host = entity(
            EntityKind::Hostname,
            "www.example.com",
            3,
            Provenance::Collected,
        );
        let t0 = now();
        let data = InvestigationData {
            investigation: investigation(),
            pages: Vec::new(),
            sources: Vec::new(),
            findings: Vec::new(),
            relationships: vec![
                edge(&domain, "resolves_to", &ip, 1, t0, Provenance::Collected),
                edge(&ip, "reverse_dns", &host, 2, t0, Provenance::Collected),
            ],
            entities: vec![domain.clone(), ip.clone(), host.clone()],
            notes: Vec::new(),
            artifacts: Vec::new(),
            timeline: Vec::new(),
            jobs: Vec::new(),
            agent_runs: Vec::new(),
            proposals: Vec::new(),
            page_entities: Vec::new(),
            resolutions: Vec::new(),
        };
        let mut query = SearchAroundQuery::new(vec![domain.id], 2);
        query.link_types = vec!["resolves_to".into()];
        let result = search_around(&data, &query);
        assert!(result.node_ids().any(|id| id == ip.id));
        assert!(
            result.node_ids().any(|id| id == host.id),
            "reverse_dns is the catalog inverse of resolves_to"
        );
        let filtered = apply_graph_filters(
            &data,
            &result,
            &GraphFilters {
                link_type: Some("resolves_to".into()),
                ..GraphFilters::default()
            },
        );
        assert!(filtered.edges.iter().any(|edge| edge.kind == "reverse_dns"));
        assert!(filtered.nodes.iter().any(|node| node.id == host.id));
    }
}
