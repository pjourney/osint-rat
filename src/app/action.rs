//! User and system actions. The TUI maps keys to these; `App::dispatch` applies them.
//!
//! Keyboard mapping lives in `tui::keymap`. Palette command ids (`"new_page"`, `"run_dns"`, …)
//! arrive as [`Action::Command`] and are handled by `App::run_command`.

use uuid::Uuid;

use crate::jobs::JobEvent;

/// One thing the workbench should do.
///
/// Variants that carry a number move by that many rows (`Move`) or cycle a form choice
/// (`Cycle`). Negative means up or previous.
#[derive(Debug)]
pub enum Action {
    /// Leave the workbench. Also bound to `Ctrl+C` from every overlay.
    Quit,
    /// Close the current overlay or abandon an in-progress edit.
    Cancel,
    /// Redraw elapsed job time. Emitted only while jobs are running.
    Tick,
    /// Focus the next pane (navigation, main, detail).
    NextPane,
    /// Focus the previous pane.
    PrevPane,
    /// Move the selection by this many rows.
    Move(i32),
    /// Open the highlighted row, or choose the highlighted palette or search hit.
    Select,
    /// Leave the current screen for the one that opened it.
    Back,
    /// Show the keyboard help overlay.
    Help,
    /// Open the command palette.
    Palette,
    /// Open search across pages, entities, findings, sources, artifacts, and agent runs.
    Search,
    /// Prompt for a question to the active agent.
    AskAgent,
    /// Create the record this screen creates: an investigation, a page, or a finding.
    NewItem,
    /// Edit page notes in `$EDITOR`, or open the agent and theme settings.
    Edit,
    /// Run the default collectors for the current page.
    Refresh,
    /// Jump to the Jobs screen.
    OpenJobs,
    /// Jump to the Tools screen.
    OpenTools,
    /// Jump to Settings.
    OpenSettings,
    /// Jump to Pages.
    OpenPages,
    /// Jump to Sources.
    OpenSources,
    /// Jump to Findings.
    OpenFindings,
    /// Jump to Entities.
    OpenEntities,
    /// Jump to the Timeline.
    OpenTimeline,
    /// Jump to the Agent screen.
    OpenAgent,
    /// Jump to Case (home).
    OpenOverview,
    /// Jump to Evidence (findings + sources).
    OpenEvidence,
    /// Jump to Review.
    OpenReview,
    /// Toggle the time lens on Evidence or Graph.
    ToggleTimeLens,
    /// Zoom the Board (Inspector hidden until any key).
    ToggleZoom,
    /// Leave Board zoom.
    Unzoom,
    /// Open Object View: highlighted entity on Entities, otherwise the current page.
    OpenObjectView,
    /// Open the Graph screen from the current entity (or the first entity).
    OpenGraph,
    /// Search Around from the current selection (default 2 hops).
    SearchAround,
    /// Change Search Around depth on the Graph screen.
    GraphHops(i32),
    /// Move the facet-strip cursor.
    GraphFacetMove(i32),
    /// Apply (or toggle) the highlighted facet as a graph filter.
    GraphApplyFacet,
    /// Clear kind / link / provenance filters on the Graph screen.
    GraphClearFilters,
    /// Move the Timeline entity-filter cursor.
    TimelineFacetMove(i32),
    /// Apply (or toggle) the highlighted Timeline entity filter.
    TimelineApplyFilter,
    /// Clear the Timeline entity filter.
    TimelineClearFilter,
    /// Toggle nearby (same country or prefix) on Timeline or Entities.
    ToggleNearby,
    /// Toggle the highlighted entity into the merge set (Entities).
    ToggleMergeMark,
    /// Propose merging the marked entities (confirm required).
    ProposeMerge,
    /// Propose unresolving the current object (confirm required).
    Unresolve,
    /// Open scored resolution suggestions. Nothing auto-merges.
    OpenSuggestions,
    /// Run a command-palette id. The ids are declared in `app::commands`.
    Command(&'static str),
    /// Confirm the open form or send the agent prompt.
    Submit,
    /// Cycle a form choice (page type, and similar fields) by this many steps.
    Cycle(i32),
    /// Accept the highlighted agent proposal (only after `Arm`).
    Accept,
    /// Reject the highlighted agent proposal.
    Reject,
    /// Arm Review (or the proposals overlay) so `y` can accept. Shift+A.
    Arm,
    /// Drop the armed accept. Any key other than `y` after `A`.
    Disarm,
    /// A progress or completion event from a background job.
    Job(JobEvent),
    /// Insert a character into the focused text field.
    InsertChar(char),
    /// Delete the character before the cursor.
    Backspace,
    /// Delete the character at the cursor.
    Delete,
    /// Move the cursor left, or cycle a form choice backward.
    Left,
    /// Move the cursor right, or cycle a form choice forward.
    Right,
    /// Move the cursor to the start of the field.
    Home,
    /// Move the cursor to the end of the field.
    End,
    /// Scroll a text overlay by this many lines.
    Scroll(i32),
    /// Open this investigation. Used after a case is created.
    OpenInvestigation(Uuid),
}
