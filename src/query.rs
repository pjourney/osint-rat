//! Scriptable read + Action propose helpers shared by the CLI and the HTTP API.
//!
//! Writes that change the ontology go through pending [`crate::model::AgentAction`]s.
//! This module never silently accepts them.

use color_eyre::eyre::{Result, eyre};
use serde::Serialize;
use uuid::Uuid;

use crate::model::{
    AgentAction, EntityKind, ObjectView, Proposal, RefKind, SearchAroundQuery, SearchAroundResult,
    Workspace, search_around, suggest_resolutions,
};
use crate::storage::Store;

#[derive(Clone, Debug, Serialize)]
pub struct ObjectSummary {
    pub id: Uuid,
    pub reference: String,
    pub kind: String,
    pub value: String,
    pub label: String,
    pub provenance: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SearchAroundView {
    pub hops_used: u32,
    pub truncated: bool,
    pub uncapped_node_count: usize,
    pub uncapped_edge_count: usize,
    pub nodes: Vec<SearchAroundNodeView>,
    pub edges: Vec<SearchAroundEdgeView>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SearchAroundNodeView {
    pub hop: u32,
    #[serde(flatten)]
    pub object: ObjectSummary,
}

#[derive(Clone, Debug, Serialize)]
pub struct SearchAroundEdgeView {
    pub id: Uuid,
    pub kind: String,
    pub from_id: Uuid,
    pub to_id: Uuid,
    pub from_ref: String,
    pub to_ref: String,
    pub discovered_at_hop: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct ResolutionSuggestionView {
    pub score: u32,
    pub rule: String,
    pub reason: String,
    pub entities: Vec<ObjectSummary>,
}

pub fn list_objects(workspace: &Workspace, kind: Option<&EntityKind>) -> Vec<ObjectSummary> {
    workspace
        .visible_entities()
        .into_iter()
        .filter(|entity| kind.is_none_or(|wanted| entity.kind == *wanted))
        .map(|entity| {
            object_summary(
                workspace,
                entity.id,
                entity.kind.as_str(),
                &entity.value,
                entity.display_name(),
                entity.provenance.as_str(),
            )
        })
        .collect()
}

pub fn object_view_for_ref(store: &Store, investigation_id: Uuid, raw: &str) -> Result<ObjectView> {
    let data = store.load_investigation(investigation_id)?;
    let workspace = Workspace::new(data);
    let (kind, record_id) = store
        .resolve_record_ref(investigation_id, raw)?
        .ok_or_else(|| eyre!("no object {raw:?} in this investigation"))?;
    match kind {
        RefKind::Entity => {
            let entity = workspace
                .entity(record_id)
                .ok_or_else(|| eyre!("entity {raw} is missing"))?;
            Ok(ObjectView::for_entity(&workspace, entity))
        }
        RefKind::Page => {
            let page = workspace
                .page(record_id)
                .ok_or_else(|| eyre!("page {raw} is missing"))?;
            Ok(ObjectView::for_page(&workspace, page))
        }
        other => Err(eyre!(
            "{raw} is a {} reference; Object View needs an entity (E-n) or page (P-n)",
            other.prefix()
        )),
    }
}

pub fn search_around_view(
    workspace: &Workspace,
    seed: &str,
    hops: u32,
    kinds: &[EntityKind],
    link_types: &[String],
) -> Result<SearchAroundView> {
    let seed_id = parse_entity_seed(workspace, seed)?;
    let mut query = SearchAroundQuery::new(vec![seed_id], hops);
    query.entity_kinds = kinds.to_vec();
    query.link_types = link_types.to_vec();
    let result = search_around(&workspace.data, &query);
    Ok(search_view(workspace, &result))
}

pub fn resolution_suggestion_views(workspace: &Workspace) -> Vec<ResolutionSuggestionView> {
    suggest_resolutions(&workspace.data)
        .into_iter()
        .map(|suggestion| ResolutionSuggestionView {
            score: suggestion.score,
            rule: suggestion.rule.as_str().to_string(),
            reason: suggestion.reason,
            entities: suggestion
                .entity_ids
                .iter()
                .filter_map(|id| {
                    let entity = workspace.entity(*id)?;
                    Some(object_summary(
                        workspace,
                        entity.id,
                        entity.kind.as_str(),
                        &entity.value,
                        entity.display_name(),
                        entity.provenance.as_str(),
                    ))
                })
                .collect(),
        })
        .collect()
}

/// Stores Actions as pending proposals. Does not apply them.
pub fn propose_actions(
    store: &mut Store,
    investigation_id: Uuid,
    origin: &str,
    actions: Vec<AgentAction>,
) -> Result<Vec<Proposal>> {
    store.insert_pending_actions(investigation_id, origin, actions)
}

pub fn parse_entity_seed(workspace: &Workspace, raw: &str) -> Result<Uuid> {
    if let Ok(uuid) = Uuid::parse_str(raw.trim()) {
        if workspace.entity(uuid).is_some() {
            return Ok(uuid);
        }
        return Err(eyre!("no entity {raw}"));
    }
    let reference: crate::model::ShortRef = raw.parse().map_err(|error| eyre!("{error}"))?;
    workspace
        .resolve_ref(reference)
        .filter(|_| reference.kind == RefKind::Entity)
        .ok_or_else(|| eyre!("no entity {raw}"))
}

fn search_view(workspace: &Workspace, result: &SearchAroundResult) -> SearchAroundView {
    let nodes = result
        .nodes
        .iter()
        .filter_map(|node| {
            let entity = workspace.entity(node.id)?;
            Some(SearchAroundNodeView {
                hop: node.hop,
                object: object_summary(
                    workspace,
                    entity.id,
                    entity.kind.as_str(),
                    &entity.value,
                    entity.display_name(),
                    entity.provenance.as_str(),
                ),
            })
        })
        .collect();
    let edges = result
        .edges
        .iter()
        .map(|edge| SearchAroundEdgeView {
            id: edge.id,
            kind: edge.kind.clone(),
            from_id: edge.from_id,
            to_id: edge.to_id,
            from_ref: workspace
                .ref_of(edge.from_id)
                .map(|reference| reference.to_string())
                .unwrap_or_default(),
            to_ref: workspace
                .ref_of(edge.to_id)
                .map(|reference| reference.to_string())
                .unwrap_or_default(),
            discovered_at_hop: edge.discovered_at_hop,
        })
        .collect();
    SearchAroundView {
        hops_used: result.hops_used,
        truncated: result.truncated,
        uncapped_node_count: result.uncapped_node_count,
        uncapped_edge_count: result.uncapped_edge_count,
        nodes,
        edges,
    }
}

fn object_summary(
    workspace: &Workspace,
    id: Uuid,
    kind: &str,
    value: &str,
    label: &str,
    provenance: &str,
) -> ObjectSummary {
    ObjectSummary {
        id,
        reference: workspace
            .ref_of(id)
            .map(|reference| reference.to_string())
            .unwrap_or_default(),
        kind: kind.to_string(),
        value: value.to_string(),
        label: label.to_string(),
        provenance: provenance.to_string(),
    }
}
