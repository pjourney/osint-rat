//! Manual merge / unresolve and resolution suggestions. Nothing auto-merges.

use color_eyre::eyre::{Result, bail, eyre};
use uuid::Uuid;

use super::{Action, App, Overlay, View};
use crate::model::{AgentAction, Entity, ResolutionSuggestion, suggest_resolutions};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingResolve {
    pub entity_ids: Vec<Uuid>,
    pub canonical_id: Uuid,
    pub reason: String,
}

impl App {
    pub fn listed_entities(&self) -> Vec<&Entity> {
        let Some(workspace) = self.workspace.as_ref() else {
            return Vec::new();
        };
        let mut entities = if self.timeline.nearby && self.view == View::Entities {
            self.listed_nearby()
        } else {
            workspace.visible_entities()
        };
        let filter = self.parsed_board_filter();
        entities.retain(|entity| filter.matches_entity(entity, workspace));
        if let Some(sort) = self.effective_sort() {
            entities
                .sort_by(|left, right| super::filter::cmp_entities(left, right, workspace, sort));
        }
        entities
    }

    pub fn selected_entity_id(&self) -> Option<Uuid> {
        self.entity_id.or_else(|| {
            self.listed_entities()
                .get(self.selected)
                .map(|entity| entity.id)
        })
    }

    pub fn unfiltered_entity_count(&self) -> usize {
        let Some(workspace) = self.workspace.as_ref() else {
            return 0;
        };
        if self.timeline.nearby && self.view == View::Entities {
            self.listed_nearby().len()
        } else {
            workspace.visible_entities().len()
        }
    }

    pub fn listed_suggestions(&self) -> Vec<ResolutionSuggestion> {
        match &self.workspace {
            Some(workspace) => suggest_resolutions(&workspace.data)
                .into_iter()
                .filter(|suggestion| {
                    let mut key = suggestion.entity_ids.clone();
                    key.sort();
                    !self
                        .dismissed_suggestions
                        .iter()
                        .any(|dismissed| dismissed == &key)
                })
                .collect(),
            None => Vec::new(),
        }
    }

    pub(crate) fn remap_focus_after_resolution(&mut self) {
        let Some(workspace) = self.workspace.as_ref() else {
            self.entity_id = None;
            self.selected = 0;
            self.merge_marks.clear();
            return;
        };
        if let Some(id) = self.entity_id {
            if workspace.entity(id).is_none() {
                self.entity_id = None;
            } else {
                self.entity_id = Some(workspace.canonical_id(id));
            }
        }
        self.merge_marks
            .retain(|id| workspace.entity(*id).is_some() && !workspace.data.is_absorbed(*id));
        if self.view != View::Entities {
            return;
        }
        let visible = workspace.visible_entities();
        if let Some(id) = self.entity_id {
            if let Some(index) = visible.iter().position(|entity| entity.id == id) {
                self.selected = index;
            } else if visible.is_empty() {
                self.selected = 0;
            } else if self.selected >= visible.len() {
                self.selected = visible.len() - 1;
            }
        } else if visible.is_empty() {
            self.selected = 0;
        } else if self.selected >= visible.len() {
            self.selected = visible.len() - 1;
        }
    }

    pub(crate) fn toggle_merge_mark(&mut self) {
        if self.view != View::Entities {
            return;
        }
        let Some(id) = self
            .listed_entities()
            .get(self.selected)
            .map(|entity| entity.id)
        else {
            self.status = "No entity to mark.".into();
            return;
        };
        if let Some(index) = self.merge_marks.iter().position(|marked| *marked == id) {
            self.merge_marks.remove(index);
            self.status = "Removed from merge set.".into();
        } else {
            self.merge_marks.push(id);
            self.status = format!("Marked for merge ({})", self.merge_marks.len());
        }
        self.entity_id = Some(id);
    }

    pub(crate) fn propose_merge(&mut self) {
        let ids = self.merge_marks.clone();
        if ids.len() < 2 {
            self.status = "Mark at least two entities (Space), then press m to merge.".into();
            return;
        }
        let highlighted = self
            .listed_entities()
            .get(self.selected)
            .map(|entity| entity.id);
        let canonical_id = highlighted
            .filter(|id| ids.contains(id))
            .or_else(|| ids.first().copied())
            .unwrap_or(ids[0]);
        self.pending_merges.push(PendingResolve {
            entity_ids: ids.clone(),
            canonical_id,
            reason: "manual merge".into(),
        });
        self.open_review_lane();
        self.status = format!(
            "Merge of {} proposed. Review it, then Shift+A and y. Enter never writes.",
            self.format_entity_labels(&ids)
        );
    }

    pub(crate) fn confirm_resolve(&mut self) -> Result<()> {
        let Some(pending) = self.pending_resolve.take() else {
            self.status = "Nothing to merge.".into();
            return Ok(());
        };
        let workspace = self
            .workspace
            .as_ref()
            .ok_or_else(|| eyre!("no investigation"))?;
        let investigation_id = workspace.id();
        let actor = self.store.identity().user_id.clone();
        self.store.resolve_entities(
            investigation_id,
            &pending.entity_ids,
            pending.canonical_id,
            &pending.reason,
            &actor,
        )?;
        self.entity_id = Some(pending.canonical_id);
        self.merge_marks.clear();
        self.reload()?;
        self.remap_focus_after_resolution();
        if self.view == View::Graph {
            self.rebuild_graph_after_restore();
        }
        self.status =
            "Entities resolved. Object View shows constituents. Palette Unresolve entity reverses this."
                .into();
        Ok(())
    }

    pub(crate) fn propose_unresolve(&mut self) {
        let Some(workspace) = self.workspace.as_ref() else {
            self.status = "Create or open an investigation first.".into();
            return;
        };
        let Some(entity_id) = self.entity_id.or_else(|| {
            self.listed_entities()
                .get(self.selected)
                .map(|entity| entity.id)
        }) else {
            self.status = "Select a resolved entity to unresolve.".into();
            return;
        };
        let Some(resolution) = workspace.latest_active_resolution(entity_id) else {
            self.status = "This object is not part of an active resolution bag.".into();
            return;
        };
        if resolution.canonical_id != workspace.canonical_id(entity_id) {
            self.status = "Open the canonical object to unresolve this bag.".into();
            return;
        }
        self.pending_unresolve = Some(resolution.id);
        self.overlay = Overlay::Confirm {
            title: "Unresolve entities".into(),
            message: format!(
                "Split {}? Constituents return. Property writes from this bag are undone.",
                resolution.short_ref()
            ),
            command: "confirm_unresolve".into(),
        };
    }

    pub(crate) fn confirm_unresolve(&mut self) -> Result<()> {
        let Some(resolution_id) = self.pending_unresolve.take() else {
            self.status = "Nothing to unresolve.".into();
            return Ok(());
        };
        let actor = self.store.identity().user_id.clone();
        self.store.unresolve_entities(resolution_id, &actor)?;
        self.reload()?;
        self.remap_focus_after_resolution();
        if self.view == View::Graph {
            self.rebuild_graph_after_restore();
        }
        self.status = "Entities unresolved. Constituents are visible again.".into();
        Ok(())
    }

    pub(crate) fn open_suggestions(&mut self) {
        if self.workspace.is_none() {
            self.status = "Create or open an investigation first.".into();
            return;
        }
        let suggestions = self.listed_suggestions();
        if suggestions.is_empty() {
            self.status = "No resolution suggestions.".into();
            return;
        }
        self.open_review_lane();
        self.status = format!(
            "{} suggestion(s) in Review. Shift+A then y writes. Nothing auto-merges.",
            suggestions.len()
        );
    }

    pub(crate) fn apply_resolve_action(&mut self, action: &AgentAction) -> Result<()> {
        let workspace = self
            .workspace
            .as_ref()
            .ok_or_else(|| eyre!("no investigation"))?;
        let investigation_id = workspace.id();
        match action {
            AgentAction::ProposeResolve { entities, reason } => {
                let mut ids = Vec::new();
                for item in entities {
                    let id = resolve_entity_token(workspace, item)
                        .ok_or_else(|| eyre!("unknown entity {item}"))?;
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
                if ids.len() < 2 {
                    bail!("PROPOSE_RESOLVE needs at least two entities");
                }
                let canonical_id = ids[0];
                let actor = self.store.identity().user_id.clone();
                self.store.resolve_entities(
                    investigation_id,
                    &ids,
                    canonical_id,
                    reason,
                    &actor,
                )?;
            }
            AgentAction::ProposeUnresolve { entity, .. } => {
                let id = resolve_entity_token(workspace, entity)
                    .ok_or_else(|| eyre!("unknown entity {entity}"))?;
                let resolution = workspace
                    .latest_active_resolution(id)
                    .ok_or_else(|| eyre!("{entity} is not resolved"))?;
                let resolution_id = resolution.id;
                let actor = self.store.identity().user_id.clone();
                self.store.unresolve_entities(resolution_id, &actor)?;
            }
            _ => bail!("not a resolution action"),
        }
        Ok(())
    }

    fn format_entity_labels(&self, ids: &[Uuid]) -> String {
        let Some(workspace) = self.workspace.as_ref() else {
            return ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
        };
        ids.iter()
            .map(|id| {
                workspace
                    .entity(*id)
                    .map(|entity| format!("{} {}", entity.short_ref(), entity.display_name()))
                    .unwrap_or_else(|| id.to_string())
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn resolve_entity_token(workspace: &crate::model::Workspace, token: &str) -> Option<Uuid> {
    if let Ok(parsed) = token.parse() {
        return workspace.resolve_ref(parsed);
    }
    if let Ok(uuid) = Uuid::parse_str(token)
        && workspace.entity(uuid).is_some()
    {
        return Some(uuid);
    }
    workspace.entity_by_any_value(token).map(|entity| entity.id)
}

impl App {
    pub(crate) fn dispatch_resolution(&mut self, action: &Action) -> bool {
        match action {
            Action::ToggleMergeMark => {
                self.toggle_merge_mark();
                true
            }
            Action::ProposeMerge => {
                self.propose_merge();
                true
            }
            Action::Unresolve => {
                self.propose_unresolve();
                true
            }
            Action::OpenSuggestions => {
                self.open_suggestions();
                true
            }
            _ => false,
        }
    }
}
