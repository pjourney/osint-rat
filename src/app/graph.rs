//! Graph screen state and Search Around commands.

use crate::model::{
    DEFAULT_EDGE_CAP, DEFAULT_GRAPH_HOPS, DEFAULT_NODE_CAP, DEFAULT_SEARCH_HOPS, FacetAxis,
    FacetChip, GraphFilters, MAX_SEARCH_HOPS, SearchAroundQuery, SearchAroundResult, VisibleGraph,
    apply_graph_filters, facet_chips, graph_facets, merge_search_results, offset_hops,
    search_around,
};

use super::{App, Overlay, Pane, View};

/// In-memory Graph screen: last Search Around result, facet filters, and selection.
#[derive(Clone, Debug)]
pub struct GraphState {
    pub result: SearchAroundResult,
    pub filters: GraphFilters,
    pub hops: u32,
    /// Seeds used for `+/-` hop changes and session restore (not merge expansions).
    pub seed_ids: Vec<uuid::Uuid>,
    pub selected: usize,
    pub facet_index: usize,
    pub pending: Option<PendingGraphOp>,
    pub return_view: Option<View>,
    pub node_cap: usize,
    pub edge_cap: usize,
}

impl Default for GraphState {
    fn default() -> Self {
        Self {
            result: SearchAroundResult::default(),
            filters: GraphFilters::default(),
            hops: DEFAULT_GRAPH_HOPS,
            seed_ids: Vec::new(),
            selected: 0,
            facet_index: 0,
            pending: None,
            return_view: None,
            node_cap: DEFAULT_NODE_CAP,
            edge_cap: DEFAULT_EDGE_CAP,
        }
    }
}

/// A Search Around that exceeded a render cap, waiting for the analyst to confirm.
#[derive(Clone, Debug)]
pub struct PendingGraphOp {
    pub result: SearchAroundResult,
    pub replace_seeds: bool,
}

impl App {
    pub fn visible_graph(&self) -> VisibleGraph {
        let Some(workspace) = self.workspace.as_ref() else {
            return VisibleGraph::default();
        };
        apply_graph_filters(&workspace.data, &self.graph.result, &self.graph.filters)
    }

    pub fn graph_facet_chips(&self) -> Vec<FacetChip> {
        let Some(workspace) = self.workspace.as_ref() else {
            return Vec::new();
        };
        facet_chips(&graph_facets(&workspace.data, &self.graph.result))
    }

    pub(crate) fn open_graph(&mut self) {
        self.merge_marks.clear();
        if self.workspace.is_none() {
            self.status = "Create or open an investigation first.".into();
            return;
        }
        let Some(seed) = self.graph_seed_id() else {
            self.enter_empty_graph();
            return;
        };
        self.apply_graph_result(self.graph_query(vec![seed], DEFAULT_GRAPH_HOPS), false);
    }

    pub(crate) fn search_around_from_selection(&mut self) {
        if self.workspace.is_none() {
            self.status = "Create or open an investigation first.".into();
            return;
        }
        let Some(seed) = self.graph_seed_id() else {
            self.status = "Select an entity to Search Around.".into();
            return;
        };
        let query = self.graph_query(vec![seed], DEFAULT_SEARCH_HOPS);
        if self.view == View::Graph && !self.graph.result.nodes.is_empty() {
            self.apply_graph_result(query, true);
            return;
        }
        self.apply_graph_result(query, false);
    }

    pub(crate) fn confirm_graph_cap(&mut self) {
        let Some(pending) = self.graph.pending.take() else {
            self.status = "Nothing to confirm.".into();
            return;
        };
        self.commit_graph_result(pending.result, pending.replace_seeds);
    }

    pub(crate) fn change_graph_hops(&mut self, delta: i32) {
        if self.view != View::Graph {
            self.open_graph();
            return;
        }
        let hops = (self.graph.hops as i32 + delta).clamp(0, MAX_SEARCH_HOPS as i32) as u32;
        let mut seeds = self.graph.seed_ids.clone();
        if seeds.is_empty()
            && let Some(seed) = self.graph.result.seed_ids.first().copied()
        {
            seeds.push(seed);
        }
        if seeds.is_empty()
            && let Some(seed) = self.graph_seed_id()
        {
            seeds.push(seed);
        }
        if seeds.is_empty() {
            self.status = "No entities to graph.".into();
            return;
        }
        self.apply_graph_result(self.graph_query(seeds, hops), false);
    }

    pub(crate) fn move_graph_facet(&mut self, delta: i32) {
        let count = self.graph_facet_chips().len();
        if count == 0 {
            self.graph.facet_index = 0;
            return;
        }
        self.graph.facet_index =
            (self.graph.facet_index as i32 + delta).rem_euclid(count as i32) as usize;
    }

    pub(crate) fn apply_graph_facet(&mut self) {
        let chips = self.graph_facet_chips();
        let Some(chip) = chips.get(self.graph.facet_index) else {
            return;
        };
        match chip.axis {
            FacetAxis::Kind => toggle_string_filter(&mut self.graph.filters.kind, &chip.key),
            FacetAxis::Link => toggle_string_filter(&mut self.graph.filters.link_type, &chip.key),
            FacetAxis::Provenance => {
                let parsed = crate::model::Provenance::parse(&chip.key);
                if self.graph.filters.provenance == parsed {
                    self.graph.filters.provenance = None;
                } else {
                    self.graph.filters.provenance = parsed;
                }
            }
        }
        self.refresh_graph_selection();
        self.status = graph_filter_status(&self.graph.filters);
    }

    pub(crate) fn clear_graph_filters(&mut self) {
        self.graph.filters = GraphFilters::default();
        self.refresh_graph_selection();
        self.status = "Graph filters cleared.".into();
    }

    pub(crate) fn close_graph(&mut self) {
        let return_view = self.graph.return_view.take().unwrap_or(View::Entities);
        self.view = if matches!(return_view, View::Graph | View::Object) {
            View::Entities
        } else {
            return_view
        };
        self.restore_list_selection_from_focus();
        if let Some(index) = self
            .nav_items()
            .iter()
            .position(|(_, candidate)| *candidate == self.view)
        {
            self.nav_index = index;
        }
        let _ = self.save_session();
    }

    pub(crate) fn rebuild_graph_after_restore(&mut self) {
        let hops = self.graph.hops.min(MAX_SEARCH_HOPS);
        let Some(workspace) = self.workspace.as_ref() else {
            return;
        };
        let mut seeds: Vec<uuid::Uuid> = self
            .graph
            .seed_ids
            .iter()
            .copied()
            .filter(|id| workspace.entity(*id).is_some())
            .collect();
        if seeds.is_empty()
            && let Some(id) = self.entity_id
            && workspace.entity(id).is_some()
        {
            seeds.push(id);
        }
        if seeds.is_empty() {
            self.view = View::Graph;
            self.pane = Pane::Main;
            self.select_nav_for_graph();
            return;
        }
        let result = search_around(&workspace.data, &self.graph_query(seeds.clone(), hops));
        self.graph.seed_ids = seeds;
        self.graph.result = result;
        self.graph.hops = self.graph.result.hops_used;
        self.view = View::Graph;
        self.pane = Pane::Main;
        self.select_nav_for_graph();
        self.revalidate_graph_filters();
        self.refresh_graph_selection();
    }

    pub(crate) fn move_graph_selection(&mut self, delta: i32) {
        let count = self.visible_graph().nodes.len();
        if count == 0 {
            self.graph.selected = 0;
            self.entity_id = None;
            return;
        }
        self.graph.selected =
            (self.graph.selected as i32 + delta).rem_euclid(count as i32) as usize;
        self.sync_graph_selection();
    }

    pub(crate) fn open_selected_graph_node(&mut self) {
        self.sync_graph_selection();
        if self.entity_id.is_none() {
            self.status = "No node selected.".into();
            return;
        }
        self.open_object_view();
    }

    pub(crate) fn apply_graph_result(&mut self, query: SearchAroundQuery, merge: bool) {
        let Some(workspace) = self.workspace.as_ref() else {
            self.status = "Create or open an investigation first.".into();
            return;
        };
        let expansion = search_around(&workspace.data, &query);
        let result = if merge && !self.graph.result.nodes.is_empty() {
            let origin_hop = query
                .seed_ids
                .first()
                .and_then(|origin| self.graph.result.hop_of(*origin))
                .unwrap_or(0);
            merge_search_results(&self.graph.result, &offset_hops(&expansion, origin_hop))
        } else {
            expansion
        };
        if result.requires_confirm() {
            self.overlay = Overlay::Confirm {
                title: "Large graph".into(),
                message: format!(
                    "Search Around found {} nodes and {} edges (caps {} / {}). Render the first {} nodes?",
                    result.uncapped_node_count,
                    result.uncapped_edge_count,
                    result.node_cap,
                    result.edge_cap,
                    result.node_cap
                ),
                command: "graph_confirm_cap".into(),
            };
            self.graph.pending = Some(PendingGraphOp {
                result,
                replace_seeds: !merge,
            });
            return;
        }
        self.commit_graph_result(result, !merge);
    }

    pub(crate) fn refresh_graph_selection(&mut self) {
        self.clamp_graph_selection();
        self.sync_graph_selection();
    }

    fn graph_seed_id(&self) -> Option<uuid::Uuid> {
        let workspace = self.workspace.as_ref()?;
        if self.view == View::Graph
            && let Some(id) = self.visible_graph_entity_id(self.graph.selected)
            && workspace.entity(id).is_some()
        {
            return Some(id);
        }
        if self.view == View::Entities
            && let Some(entity) = workspace.visible_entities().get(self.selected)
        {
            return Some(entity.id);
        }
        if let Some(id) = self.entity_id
            && workspace.entity(id).is_some()
        {
            return Some(id);
        }
        if let Some(page_id) = self.page_id
            && let Some(entity_id) = workspace.page(page_id).and_then(|page| page.entity_id)
            && workspace.entity(entity_id).is_some()
        {
            return Some(entity_id);
        }
        workspace
            .visible_entities()
            .first()
            .map(|entity| entity.id)
            .or_else(|| workspace.data.entities.first().map(|entity| entity.id))
    }

    fn visible_graph_entity_id(&self, index: usize) -> Option<uuid::Uuid> {
        self.visible_graph().nodes.get(index).map(|node| node.id)
    }

    fn graph_query(&self, seed_ids: Vec<uuid::Uuid>, hops: u32) -> SearchAroundQuery {
        let mut query = SearchAroundQuery::new(seed_ids, hops);
        query.node_cap = self.graph.node_cap.max(1);
        query.edge_cap = self.graph.edge_cap.max(1);
        query
    }

    fn commit_graph_result(&mut self, result: SearchAroundResult, replace_seeds: bool) {
        if self.view != View::Graph {
            self.graph.return_view = Some(self.view);
        }
        if replace_seeds {
            self.graph.seed_ids = result.seed_ids.clone();
        }
        self.graph.result = result;
        self.graph.hops = self.graph.result.hops_used;
        self.graph.pending = None;
        self.view = View::Graph;
        self.pane = Pane::Main;
        self.overlay = Overlay::None;
        self.select_nav_for_graph();
        self.revalidate_graph_filters();
        self.refresh_graph_selection();
        let truncated = if self.graph.result.truncated {
            " · capped"
        } else {
            ""
        };
        self.status = format!(
            "Graph · {} hop(s) · {} nodes · {} edges{truncated}. s Search Around, +/- hops, t time lens.",
            self.graph.hops,
            self.graph.result.nodes.len(),
            self.graph.result.edges.len()
        );
        let _ = self.save_session();
    }

    fn enter_empty_graph(&mut self) {
        if self.view != View::Graph {
            self.graph.return_view = Some(self.view);
        }
        self.graph.result = SearchAroundResult::default();
        self.graph.filters = GraphFilters::default();
        self.graph.seed_ids = Vec::new();
        self.graph.selected = 0;
        self.entity_id = None;
        self.view = View::Graph;
        self.pane = Pane::Main;
        self.overlay = Overlay::None;
        self.select_nav_for_graph();
        self.status = "No entities to graph. Collect on a page first.".into();
        let _ = self.save_session();
    }

    fn select_nav_for_graph(&mut self) {
        if let Some(index) = self
            .nav_items()
            .iter()
            .position(|(_, candidate)| *candidate == View::Graph)
        {
            self.nav_index = index;
        }
    }

    fn restore_list_selection_from_focus(&mut self) {
        let Some(workspace) = self.workspace.as_ref() else {
            self.selected = 0;
            return;
        };
        let index = match self.view {
            View::Entities => self.entity_id.and_then(|id| {
                let canonical = workspace.canonical_id(id);
                workspace
                    .visible_entities()
                    .iter()
                    .position(|entity| entity.id == canonical)
            }),
            View::Pages | View::Overview => self
                .page_id
                .and_then(|id| workspace.data.pages.iter().position(|page| page.id == id)),
            _ => None,
        };
        self.selected = index.unwrap_or(0);
    }

    fn revalidate_graph_filters(&mut self) {
        if self.graph.filters.is_empty() {
            return;
        }
        let chips = self.graph_facet_chips();
        if let Some(kind) = self.graph.filters.kind.clone()
            && !chips
                .iter()
                .any(|chip| chip.axis == FacetAxis::Kind && chip.key == kind)
        {
            self.graph.filters.kind = None;
        }
        if let Some(link) = self.graph.filters.link_type.clone()
            && !chips
                .iter()
                .any(|chip| chip.axis == FacetAxis::Link && chip.key == link)
        {
            self.graph.filters.link_type = None;
        }
        if let Some(provenance) = self.graph.filters.provenance
            && !chips
                .iter()
                .any(|chip| chip.axis == FacetAxis::Provenance && chip.key == provenance.as_str())
        {
            self.graph.filters.provenance = None;
        }
        if !self.graph.filters.is_empty() && self.visible_graph().is_empty() {
            self.graph.filters = GraphFilters::default();
        }
    }

    fn clamp_graph_selection(&mut self) {
        let count = self.visible_graph().nodes.len();
        if count == 0 {
            self.graph.selected = 0;
            return;
        }
        if self.graph.selected >= count {
            self.graph.selected = count - 1;
        }
    }

    fn sync_graph_selection(&mut self) {
        self.entity_id = self.visible_graph_entity_id(self.graph.selected);
    }
}

fn toggle_string_filter(slot: &mut Option<String>, key: &str) {
    if slot.as_deref() == Some(key) {
        *slot = None;
    } else {
        *slot = Some(key.to_string());
    }
}

fn graph_filter_status(filters: &GraphFilters) -> String {
    if filters.is_empty() {
        return "Graph facet cleared.".into();
    }
    let mut parts = Vec::new();
    if let Some(kind) = &filters.kind {
        parts.push(format!("kind={kind}"));
    }
    if let Some(link) = &filters.link_type {
        parts.push(format!("link={link}"));
    }
    if let Some(provenance) = filters.provenance {
        parts.push(format!("provenance={}", provenance.as_str()));
    }
    format!("Graph filter: {}", parts.join(" · "))
}
