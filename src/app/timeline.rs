//! Timeline overlay, entity filter, and nearby list.

use uuid::Uuid;

use super::{App, Overlay, View};
use crate::model::{Entity, TimelineRow, nearby_entities, overlay_timeline};

/// In-memory Timeline screen: overlay rows, entity filter, nearby mode.
#[derive(Clone, Debug, Default)]
pub struct TimelineState {
    pub filter_entity_id: Option<Uuid>,
    pub nearby: bool,
    pub facet_index: usize,
    pub event_id: Option<Uuid>,
    pub saved_selected: usize,
    pub filter_saved_selected: Option<usize>,
    pub nearby_seed_id: Option<Uuid>,
}

impl App {
    pub fn listed_timeline_rows(&self) -> Vec<TimelineRow> {
        let Some(workspace) = self.workspace.as_ref() else {
            return Vec::new();
        };
        overlay_timeline(&workspace.data, self.timeline.filter_entity_id)
    }

    pub fn listed_nearby(&self) -> Vec<&Entity> {
        let Some(workspace) = self.workspace.as_ref() else {
            return Vec::new();
        };
        let Some(seed) = self.nearby_seed_entity() else {
            return Vec::new();
        };
        nearby_entities(&workspace.data, seed)
    }

    pub fn timeline_list_len(&self) -> usize {
        if self.timeline.nearby {
            self.listed_nearby().len()
        } else {
            self.listed_timeline_rows().len()
        }
    }

    pub fn timeline_filter_candidates(&self) -> Vec<Uuid> {
        let Some(workspace) = self.workspace.as_ref() else {
            return Vec::new();
        };
        let rows = overlay_timeline(&workspace.data, None);
        let mut ids = Vec::new();
        for row in rows {
            for entity_id in row.entity_ids {
                let canonical = workspace.canonical_id(entity_id);
                if !ids.contains(&canonical) {
                    ids.push(canonical);
                }
            }
        }
        ids
    }

    pub(crate) fn move_timeline_facet(&mut self, delta: i32) {
        if !self.is_time_lens() {
            return;
        }
        let count = self.timeline_filter_candidates().len().max(1);
        self.timeline.facet_index =
            (self.timeline.facet_index as i32 + delta).rem_euclid(count as i32) as usize;
        self.status = self.timeline_status();
    }

    pub(crate) fn apply_timeline_filter(&mut self) {
        if !self.is_time_lens() {
            return;
        }
        let candidates = self.timeline_filter_candidates();
        let Some(id) = candidates.get(self.timeline.facet_index).copied() else {
            self.status = "No entity on the timeline to filter.".into();
            return;
        };
        if self.timeline.filter_entity_id == Some(id) {
            self.clear_timeline_filter();
            return;
        }
        if self.timeline.filter_saved_selected.is_none() {
            self.timeline.filter_saved_selected = Some(self.selected);
        }
        self.timeline.filter_entity_id = Some(id);
        self.timeline.nearby = false;
        self.selected = 0;
        self.sync_timeline_selection();
        self.status = self.timeline_status();
    }

    pub(crate) fn clear_timeline_filter(&mut self) {
        if !self.is_time_lens() {
            return;
        }
        self.timeline.filter_entity_id = None;
        self.timeline.nearby = false;
        if let Some(saved) = self.timeline.filter_saved_selected.take() {
            self.selected = saved;
        }
        self.clamp_timeline_selection();
        self.sync_timeline_selection();
        self.status = self.timeline_status();
    }

    pub(crate) fn toggle_nearby(&mut self) {
        if !matches!(self.view, View::Timeline | View::Entities | View::Evidence) {
            return;
        }
        if self.timeline.nearby {
            self.timeline.nearby = false;
            self.timeline.nearby_seed_id = None;
            self.selected = self.timeline.saved_selected;
            if self.view == View::Timeline {
                self.clamp_timeline_selection();
                self.sync_timeline_selection();
            }
            self.status = "Nearby off.".into();
            return;
        }
        if self.nearby_seed_entity().is_none() {
            self.status = "Select an entity with a country or prefix for nearby.".into();
            return;
        }
        let seed_id = self.nearby_seed_entity().map(|entity| entity.id);
        self.timeline.saved_selected = self.selected;
        self.timeline.nearby_seed_id = seed_id;
        self.entity_id = seed_id;
        self.timeline.nearby = true;
        self.selected = 0;
        let count = self.listed_nearby().len();
        self.status = if count == 0 {
            "No nearby entities (same country or prefix).".into()
        } else {
            format!("{count} nearby entit(y/ies). Esc restores the previous list.")
        };
    }

    pub(crate) fn select_timeline_row(&mut self) {
        if self.timeline.nearby {
            let id = self
                .listed_nearby()
                .get(self.selected)
                .map(|entity| entity.id);
            if let Some(id) = id {
                self.entity_id = Some(id);
                self.open_object_view();
            }
            return;
        }
        let row = self.listed_timeline_rows().get(self.selected).cloned();
        let Some(row) = row else {
            return;
        };
        self.timeline.event_id = Some(row.id);
        self.timeline.saved_selected = self.selected;
        if let Some(entity_id) = row.entity_ids.first().copied() {
            self.entity_id = Some(entity_id);
            self.open_object_view();
        } else {
            self.status = format!("{}  {}", row.short_ref, row.title);
        }
    }

    pub(crate) fn restore_timeline_selection(&mut self) {
        if !self.is_time_lens() {
            return;
        }
        if self.timeline.nearby {
            let nearby = self.listed_nearby();
            if let Some(id) = self.entity_id
                && let Some(index) = nearby.iter().position(|entity| entity.id == id)
            {
                self.selected = index;
                return;
            }
            self.selected = self.selected.min(nearby.len().saturating_sub(1));
            return;
        }
        let rows = self.listed_timeline_rows();
        if let Some(id) = self.timeline.event_id
            && let Some(index) = rows.iter().position(|row| row.id == id)
        {
            self.selected = index;
            return;
        }
        if let Some(entity_id) = self.entity_id
            && let Some(index) = rows
                .iter()
                .position(|row| row.entity_ids.contains(&entity_id))
        {
            self.selected = index;
            return;
        }
        self.selected = self
            .timeline
            .saved_selected
            .min(rows.len().saturating_sub(1));
    }

    pub(crate) fn clamp_timeline_selection(&mut self) {
        let count = self.timeline_list_len();
        if count == 0 {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(count - 1);
        }
    }

    pub(crate) fn sync_timeline_selection(&mut self) {
        if self.timeline.nearby {
            self.entity_id = self
                .listed_nearby()
                .get(self.selected)
                .map(|entity| entity.id);
            return;
        }
        if let Some(row) = self.listed_timeline_rows().get(self.selected) {
            self.timeline.event_id = Some(row.id);
            self.entity_id = row.entity_ids.first().copied();
        }
    }

    fn nearby_seed_entity(&self) -> Option<&Entity> {
        let workspace = self.workspace.as_ref()?;
        if let Some(id) = self
            .timeline
            .nearby_seed_id
            .or(self.timeline.filter_entity_id)
            .or(self.entity_id)
            && let Some(entity) = workspace.entity(workspace.canonical_id(id))
        {
            return Some(entity);
        }
        match self.view {
            View::Entities => {
                let index = if self.timeline.nearby {
                    self.timeline.saved_selected
                } else {
                    self.selected
                };
                workspace.visible_entities().get(index).copied()
            }
            View::Timeline | View::Evidence => {
                let rows = self.listed_timeline_rows();
                let row = rows.get(self.selected)?;
                let id = row.entity_ids.first().copied()?;
                workspace.entity(workspace.canonical_id(id))
            }
            _ => None,
        }
    }

    fn timeline_status(&self) -> String {
        if self.timeline.nearby {
            return format!("{} nearby. Esc restores.", self.listed_nearby().len());
        }
        let rows = self.listed_timeline_rows().len();
        match self.timeline.filter_entity_id.and_then(|id| {
            self.workspace
                .as_ref()
                .and_then(|workspace| workspace.entity(id))
                .map(|entity| entity.display_name().to_string())
        }) {
            Some(name) => format!("Time lens: {name} ({rows} row(s)). x clears."),
            None => format!("Time lens: {rows} row(s). t toggles. x clears."),
        }
    }

    pub(crate) fn cancel_timeline_mode(&mut self) -> bool {
        if !self.is_time_lens() && !(self.view == View::Entities && self.timeline.nearby) {
            return false;
        }
        if self.overlay != Overlay::None {
            return false;
        }
        if self.timeline.nearby {
            self.toggle_nearby();
            return true;
        }
        if self.is_time_lens() && self.timeline.filter_entity_id.is_some() {
            self.clear_timeline_filter();
            return true;
        }
        false
    }
}

impl App {
    pub(crate) fn selected_timeline_row(&self) -> Option<TimelineRow> {
        if self.timeline.nearby {
            return None;
        }
        self.listed_timeline_rows().get(self.selected).cloned()
    }
}
