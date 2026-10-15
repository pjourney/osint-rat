//! Command registry: palette entries, hint-bar keys, and generated help.
//!
//! [`map_registered_key`] walks [`COMMANDS`] and matches [`Command::hint`]. Overlay and
//! editing still live in `tui/keymap.rs`. Help is generated from this table so it
//! cannot drift from the keys the TUI actually binds.

use crossterm::event::{KeyCode, KeyModifiers};

use super::{Action, Overlay, View};

/// One command in the cockpit registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command {
    pub id: &'static str,
    pub title: &'static str,
    /// Key shown in the palette, hint bar, and help. Empty if palette-only.
    pub hint: &'static str,
    pub summary: &'static str,
    pub in_palette: bool,
    pub on_hint_bar: bool,
    /// True when the command writes case data. Enter must never be this.
    pub writes: bool,
    /// Empty means the key is bound on every view.
    pub views: &'static [View],
}

const fn cmd(
    id: &'static str,
    title: &'static str,
    hint: &'static str,
    summary: &'static str,
    in_palette: bool,
    on_hint_bar: bool,
    writes: bool,
) -> Command {
    Command {
        id,
        title,
        hint,
        summary,
        in_palette,
        on_hint_bar,
        writes,
        views: &[],
    }
}

#[allow(clippy::too_many_arguments)]
const fn cmd_views(
    id: &'static str,
    title: &'static str,
    hint: &'static str,
    summary: &'static str,
    in_palette: bool,
    on_hint_bar: bool,
    writes: bool,
    views: &'static [View],
) -> Command {
    Command {
        id,
        title,
        hint,
        summary,
        in_palette,
        on_hint_bar,
        writes,
        views,
    }
}

impl Command {
    pub fn applies_to(self, view: View) -> bool {
        self.views.is_empty() || self.views.contains(&view)
    }
}

/// Whether this palette/registry id writes case data.
///
/// [`Command::writes`] is the source of truth for registered ids. Internal confirm
/// ids and `run_tool:*` are treated as writes too.
pub fn command_writes(id: &str) -> bool {
    if id.starts_with("run_tool:") || matches!(id, "confirm_resolve" | "confirm_unresolve") {
        return true;
    }
    COMMANDS
        .iter()
        .any(|command| command.id == id && command.writes)
}

pub const COMMANDS: &[Command] = &[
    cmd(
        "quit",
        "Quit",
        "q",
        "Leave the workbench. Confirms if jobs are running.",
        false,
        false,
        false,
    ),
    cmd(
        "back",
        "Back",
        "Esc",
        "Close an overlay or return to the previous screen.",
        false,
        false,
        false,
    ),
    cmd(
        "next_pane",
        "Next pane",
        "Tab",
        "Move focus to the next pane.",
        false,
        false,
        false,
    ),
    cmd(
        "prev_pane",
        "Previous pane",
        "Shift+Tab",
        "Move focus to the previous pane.",
        false,
        false,
        false,
    ),
    cmd(
        "move_down",
        "Move down",
        "j",
        "Move the selection down. Arrow keys work too.",
        false,
        false,
        false,
    ),
    cmd(
        "move_up",
        "Move up",
        "k",
        "Move the selection up. Arrow keys work too.",
        false,
        false,
        false,
    ),
    cmd(
        "select",
        "Open",
        "Enter",
        "Open or drill in. Enter never writes case data.",
        false,
        false,
        false,
    ),
    cmd(
        "search",
        "Search",
        "/",
        "Filter the current table, or search the case.",
        false,
        true,
        false,
    ),
    cmd(
        "palette",
        "Command palette",
        "Ctrl+K",
        "Open the command palette. : is an alias.",
        false,
        true,
        false,
    ),
    cmd(
        "focus_left",
        "Focus previous pane",
        "h",
        "Move focus Rail ◀ Board ◀ Inspector.",
        false,
        false,
        false,
    ),
    cmd(
        "focus_right",
        "Focus next pane",
        "l",
        "Move focus Rail ▶ Board ▶ Inspector.",
        false,
        false,
        false,
    ),
    cmd(
        "help",
        "Help",
        "?",
        "Help for the current screen, generated from this registry.",
        true,
        true,
        false,
    ),
    cmd(
        "new_investigation",
        "New Investigation",
        "n",
        "Create an investigation, a page, or a finding, depending on the screen.",
        true,
        false,
        true,
    ),
    cmd_views(
        "open_investigation",
        "Open Investigation",
        "",
        "Open an existing investigation (welcome screen or palette).",
        true,
        false,
        false,
        &[View::Welcome],
    ),
    cmd(
        "new_page",
        "New Page",
        "n",
        "Add a page to the open investigation.",
        true,
        false,
        true,
    ),
    cmd(
        "run_collectors",
        "Run Collectors",
        "c",
        "Collect on the selected target (passive collectors only).",
        true,
        true,
        true,
    ),
    cmd(
        "run_dns",
        "Run DNS Collection",
        "",
        "Run the DNS collector only.",
        true,
        false,
        true,
    ),
    cmd(
        "run_rdap",
        "Run RDAP Collection",
        "",
        "Run the RDAP collector only.",
        true,
        false,
        true,
    ),
    cmd(
        "run_http",
        "Fetch URL / HTTP",
        "",
        "Run the HTTP collector only.",
        true,
        false,
        true,
    ),
    cmd(
        "run_tls",
        "Run TLS Collection",
        "",
        "Run the TLS collector only.",
        true,
        false,
        true,
    ),
    cmd(
        "run_ct",
        "Run Certificate Transparency",
        "",
        "Run the certificate-transparency collector only.",
        true,
        false,
        true,
    ),
    cmd(
        "run_ip",
        "Run IP Intelligence",
        "",
        "Run the IP intelligence collector only.",
        true,
        false,
        true,
    ),
    cmd(
        "run_email",
        "Run Email Identity Pack",
        "",
        "Run the email identity pack only.",
        true,
        false,
        true,
    ),
    cmd(
        "run_username",
        "Run Username Pack",
        "",
        "Run the username pack only.",
        true,
        false,
        true,
    ),
    cmd(
        "run_subdomain",
        "Run Subdomain Expansion",
        "",
        "Run passive subdomain expansion only.",
        true,
        false,
        true,
    ),
    cmd(
        "ingest",
        "Ingest file",
        "",
        "Ingest STIX, MISP, CSV, JSON, or an Evidence Batch.",
        true,
        false,
        true,
    ),
    cmd(
        "run_playbook",
        "Run playbook",
        "",
        "Run a bundled YAML playbook. Active tools are refused.",
        true,
        false,
        true,
    ),
    cmd(
        "playbook_prompt",
        "Playbook prompt",
        "",
        "Seed the agent question with suggested playbook Actions.",
        true,
        false,
        true,
    ),
    cmd(
        "watch_playbook",
        "Watch playbook",
        "",
        "Re-run collect steps. Automations propose; they do not write findings.",
        true,
        false,
        true,
    ),
    cmd(
        "ask_agent",
        "Ask Agent",
        "a",
        "Ask the agent about the selection. Results land in Review.",
        true,
        true,
        true,
    ),
    cmd(
        "generate_report",
        "Generate Report",
        "",
        "Write a Markdown or JSON report.",
        true,
        false,
        true,
    ),
    cmd(
        "add_finding",
        "Add Finding",
        "",
        "Record an analyst finding. Cite sources by short id.",
        true,
        false,
        true,
    ),
    cmd(
        "add_note",
        "Add Note",
        "e",
        "Edit notes in $EDITOR, or open Settings from that screen.",
        true,
        false,
        true,
    ),
    cmd(
        "open_case",
        "Open Case",
        "1",
        "Jump to Case.",
        false,
        false,
        false,
    ),
    cmd(
        "open_entities",
        "Open Entities",
        "2",
        "Jump to Entities.",
        true,
        false,
        false,
    ),
    cmd(
        "open_evidence",
        "Open Evidence",
        "3",
        "Jump to Evidence.",
        true,
        false,
        false,
    ),
    cmd(
        "open_graph",
        "Graph",
        "4",
        "Jump to Graph.",
        true,
        true,
        false,
    ),
    cmd(
        "open_review",
        "Open Review",
        "5",
        "Jump to Review. AI and merge proposals wait here.",
        true,
        false,
        false,
    ),
    cmd(
        "open_jobs",
        "Show Jobs",
        "6",
        "Jump to Jobs.",
        true,
        false,
        false,
    ),
    cmd(
        "open_overview",
        "Open Case",
        "",
        "Alias for Open Case.",
        false,
        false,
        false,
    ),
    cmd(
        "open_pages",
        "Open Targets",
        "",
        "Targets live on Case.",
        true,
        false,
        false,
    ),
    cmd(
        "open_sources",
        "Open Sources",
        "",
        "Sources live on Evidence and the Inspector Sources tab.",
        true,
        false,
        false,
    ),
    cmd(
        "open_findings",
        "Open Findings",
        "",
        "Findings live on Evidence.",
        true,
        false,
        false,
    ),
    cmd(
        "open_timeline",
        "Open Timeline",
        "",
        "Time lens on Evidence / Graph. Toggle with t.",
        true,
        false,
        false,
    ),
    cmd(
        "open_agent",
        "Agent History",
        "",
        "Agent transcript is the Inspector for a Review run.",
        true,
        false,
        false,
    ),
    cmd(
        "open_tools",
        "Show Tools",
        "",
        "List tools. Active tools are refused, with no enable option.",
        true,
        false,
        false,
    ),
    cmd(
        "open_settings",
        "Configure Agent",
        "",
        "Agent profile and theme.",
        true,
        false,
        false,
    ),
    cmd(
        "open_notes",
        "Open Notes",
        "",
        "Notes live on the Inspector Notes tab. e edits in $EDITOR.",
        true,
        false,
        false,
    ),
    cmd(
        "open_audit",
        "Open Audit",
        "",
        "Open the append-only audit log.",
        true,
        false,
        false,
    ),
    cmd(
        "set_marking",
        "Set marking",
        "",
        "Set the Traffic Light Protocol marking on the case.",
        true,
        false,
        true,
    ),
    cmd(
        "add_member",
        "Add team member",
        "",
        "Add a local case member.",
        true,
        false,
        true,
    ),
    cmd_views(
        "toggle_nearby",
        "Nearby (country / prefix)",
        "",
        "List entities nearby by country or prefix.",
        true,
        false,
        false,
        &[View::Timeline, View::Entities, View::Evidence],
    ),
    cmd(
        "open_artifacts",
        "Open Artifacts",
        "",
        "Open stored snapshots, reports, and context packs.",
        true,
        false,
        false,
    ),
    cmd(
        "open_object",
        "Object View",
        "",
        "Open Object View for the highlighted entity or the current page. Enter also opens.",
        true,
        false,
        false,
    ),
    cmd(
        "search_around_graph",
        "Open selection in Graph",
        "g",
        "Open the selection in Graph.",
        false,
        true,
        false,
    ),
    cmd(
        "search_around",
        "Search Around",
        "s",
        "Search Around from the selection (default 2 hops).",
        true,
        true,
        false,
    ),
    cmd_views(
        "merge_entities",
        "Merge selected entities",
        "m",
        "Propose a merge of marked rows. Nothing is written until you confirm.",
        true,
        false,
        true,
        &[View::Entities],
    ),
    cmd_views(
        "unresolve_entity",
        "Unresolve entity",
        "",
        "Propose unresolving the current bag. Lands in Review.",
        true,
        false,
        true,
        &[View::Entities, View::Object],
    ),
    cmd(
        "resolution_suggestions",
        "Resolution suggestions",
        "",
        "Open scored merge suggestions. They never auto-merge.",
        true,
        false,
        false,
    ),
    cmd_views(
        "doctor",
        "Doctor",
        "",
        "Run environment checks.",
        true,
        false,
        false,
        &[View::Welcome],
    ),
    cmd_views(
        "cancel_job",
        "Cancel Running Job",
        "",
        "Cancel the selected job.",
        true,
        false,
        true,
        &[View::Jobs],
    ),
    cmd_views(
        "graph_hops_inc",
        "More hops",
        "+",
        "Graph hops, or time-lens bucket.",
        false,
        false,
        false,
        &[View::Graph, View::Evidence, View::Timeline],
    ),
    cmd_views(
        "graph_hops_dec",
        "Fewer hops",
        "-",
        "Graph hops, or time-lens bucket.",
        false,
        false,
        false,
        &[View::Graph, View::Evidence, View::Timeline],
    ),
    cmd_views(
        "facet_prev",
        "Previous facet",
        "",
        "Move the facet cursor left.",
        true,
        false,
        false,
        &[View::Graph, View::Timeline, View::Evidence],
    ),
    cmd_views(
        "facet_next",
        "Next facet",
        "",
        "Move the facet cursor right.",
        true,
        false,
        false,
        &[View::Graph, View::Timeline, View::Evidence],
    ),
    cmd_views(
        "apply_facet",
        "Apply facet",
        "",
        "Apply the highlighted facet.",
        true,
        false,
        false,
        &[View::Graph, View::Timeline, View::Evidence],
    ),
    cmd_views(
        "reject_review",
        "Reject",
        "x",
        "Reject the highlighted Review item.",
        false,
        false,
        false,
        &[View::Review],
    ),
    cmd_views(
        "clear_filters",
        "Clear filters",
        "x",
        "Clear the filter or time lens.",
        false,
        false,
        false,
        &[View::Graph, View::Evidence, View::Timeline],
    ),
    cmd_views(
        "toggle_time_lens",
        "Time lens",
        "t",
        "Toggle the time lens on Evidence or Graph.",
        false,
        false,
        false,
        &[View::Evidence, View::Graph, View::Entities, View::Timeline],
    ),
    cmd(
        "toggle_zoom",
        "Zoom Board",
        "z",
        "Zoom the Board. Any key returns.",
        false,
        false,
        false,
    ),
    cmd_views(
        "arm_review",
        "Select for accept",
        "A",
        "Arm the highlighted Review item. y confirms.",
        false,
        false,
        false,
        &[View::Review],
    ),
    cmd_views(
        "mark",
        "Mark row",
        "Space",
        "Mark a row for merge or batch accept.",
        false,
        false,
        false,
        &[View::Entities, View::Review],
    ),
];

pub fn filtered(query: &str) -> Vec<&'static Command> {
    let query = query.trim().to_ascii_lowercase();
    COMMANDS
        .iter()
        .filter(|command| command.in_palette)
        .filter(|command| {
            query.is_empty()
                || command.title.to_ascii_lowercase().contains(&query)
                || command.id.contains(&query)
        })
        .collect()
}

pub fn hint_commands() -> Vec<&'static Command> {
    hint_commands_for(View::Overview)
}

pub fn hint_commands_for(view: View) -> Vec<&'static Command> {
    COMMANDS
        .iter()
        .filter(|command| command.on_hint_bar && command.applies_to(view))
        .collect()
}

/// Context-bar commands for the focused pane.
pub fn context_commands(view: View, pane: super::Pane) -> Vec<&'static Command> {
    let _ = pane;
    let mut commands = hint_commands_for(view);
    for id in context_ids(view) {
        if let Some(command) = COMMANDS.iter().find(|command| command.id == *id)
            && !commands.iter().any(|existing| existing.id == command.id)
        {
            commands.push(command);
        }
    }
    commands
}

fn context_ids(view: View) -> &'static [&'static str] {
    match view {
        View::Overview => &[
            "run_collectors",
            "new_investigation",
            "ask_agent",
            "search_around",
        ],
        View::Entities => &[
            "search_around",
            "search_around_graph",
            "toggle_time_lens",
            "mark",
        ],
        View::Evidence => &["toggle_time_lens", "search"],
        View::Graph => &["search_around", "graph_hops_inc", "toggle_time_lens"],
        View::Review => &["arm_review", "reject_review"],
        View::Jobs => &["help"],
        _ => &["help"],
    }
}

/// Help overlay body. Generated from [`COMMANDS`] so it cannot drift from the keymap.
pub fn help_text() -> String {
    let mut lines = vec![
        "os-int keyboard shortcuts".into(),
        String::new(),
        "Keys below are generated from the command registry.".into(),
        String::new(),
    ];
    let mut seen = std::collections::BTreeSet::new();
    for command in COMMANDS {
        if command.hint.is_empty() {
            continue;
        }
        if !seen.insert(command.hint) {
            continue;
        }
        lines.push(format!("  {:<12} {}", command.hint, command.summary));
    }
    lines.push(String::new());
    lines.push("  Ctrl+C       Quit from anywhere".into());
    lines.push("  Ctrl+K       Command palette (also :)".into());
    lines.push("  Ctrl+F       Search (same as /)".into());
    lines.push(String::new());
    lines.push("  1 Case  2 Entities  3 Evidence  4 Graph  5 Review  6 Jobs".into());
    lines.push("  Tools, settings, audit, artifacts, ingest, playbooks: palette.".into());
    lines.push(String::new());
    lines.push("Enter opens. It never writes.".into());
    lines.push("Shift+A then y accepts a Review item. Any other key disarms.".into());
    lines.push("y confirms a write prompt. Ctrl+S submits a form or agent prompt.".into());
    lines.push("Palette writes open a y-confirm. Enter on the palette does not run them.".into());
    lines.push(String::new());
    lines.push("Provenance glyphs: ● collected  ◇ original material".into());
    lines.push("                   ✎ you wrote  ◆ agent inference (kept after accept)".into());
    lines.push("ASCII (`ui.ascii`): C  X  A  *".into());
    lines.push(String::new());
    lines.push("os-int observes public information. Active tools are refused.".into());
    lines.join("\n")
}

/// Parses a registry hint into the key it documents.
pub fn hint_key(hint: &str) -> Option<KeyCode> {
    match hint {
        "Esc" => Some(KeyCode::Esc),
        "Tab" => Some(KeyCode::Tab),
        "Shift+Tab" => Some(KeyCode::BackTab),
        "Enter" => Some(KeyCode::Enter),
        "Space" => Some(KeyCode::Char(' ')),
        other if other.chars().count() == 1 => other.chars().next().map(KeyCode::Char),
        _ => None,
    }
}

fn action_for_command(id: &str, view: View) -> Option<Action> {
    Some(match id {
        "quit" => Action::Quit,
        "back" => Action::Back,
        "next_pane" => Action::NextPane,
        "prev_pane" => Action::PrevPane,
        "move_down" => Action::Move(1),
        "move_up" => Action::Move(-1),
        "select" => Action::Select,
        "search" => Action::Search,
        "palette" => Action::Palette,
        "help" => Action::Help,
        "focus_left" => Action::PrevPane,
        "focus_right" => Action::NextPane,
        "new_investigation" | "new_page" => Action::NewItem,
        "open_investigation" => Action::Command("open_investigation"),
        "run_collectors" => Action::Refresh,
        "ingest" => Action::Command("ingest"),
        "run_playbook" => Action::Command("run_playbook"),
        "ask_agent" => Action::AskAgent,
        "add_note" => Action::Edit,
        "open_overview" | "open_case" | "open_pages" => Action::OpenOverview,
        "open_sources" | "open_findings" | "open_evidence" => Action::OpenEvidence,
        "open_entities" => Action::OpenEntities,
        "open_timeline" => Action::OpenTimeline,
        "open_agent" => Action::OpenReview,
        "open_review" => Action::OpenReview,
        "open_jobs" => Action::OpenJobs,
        "open_tools" => Action::OpenTools,
        "open_settings" => Action::OpenSettings,
        "toggle_nearby" => Action::ToggleNearby,
        "open_object" => Action::OpenObjectView,
        "open_graph" | "search_around_graph" => Action::OpenGraph,
        "search_around" => Action::SearchAround,
        "toggle_time_lens" => Action::ToggleTimeLens,
        "toggle_zoom" => Action::ToggleZoom,
        "arm_review" => Action::Arm,
        "reject_review" => Action::Reject,
        "merge_entities" => Action::ProposeMerge,
        "unresolve_entity" => Action::Unresolve,
        "doctor" => Action::Command("doctor"),
        "cancel_job" => Action::Command("cancel_job"),
        "graph_hops_inc" if view == View::Graph => Action::GraphHops(1),
        "graph_hops_inc" => Action::TimelineFacetMove(1),
        "graph_hops_dec" if view == View::Graph => Action::GraphHops(-1),
        "graph_hops_dec" => Action::TimelineFacetMove(-1),
        "facet_prev" if view == View::Graph => Action::GraphFacetMove(-1),
        "facet_prev" => Action::TimelineFacetMove(-1),
        "facet_next" if view == View::Graph => Action::GraphFacetMove(1),
        "facet_next" => Action::TimelineFacetMove(1),
        "apply_facet" if view == View::Graph => Action::GraphApplyFacet,
        "apply_facet" => Action::TimelineApplyFilter,
        "clear_filters" if view == View::Graph => Action::GraphClearFilters,
        "clear_filters" => Action::TimelineClearFilter,
        "mark" => Action::ToggleMergeMark,
        _ => return None,
    })
}

/// Lookup used by the keymap for documented normal-mode bindings.
///
/// Bindings come from [`COMMANDS`]. PageUp/PageDown, arrows, and `=` (alias of `+`)
/// are the only extras not listed as hints.
pub fn map_registered_key(
    code: KeyCode,
    view: View,
    ctrl: bool,
    overlay: &Overlay,
) -> Option<Action> {
    if !matches!(overlay, Overlay::None) {
        return None;
    }
    if ctrl {
        return match code {
            KeyCode::Char('k') | KeyCode::Char('p') => Some(Action::Palette),
            KeyCode::Char('f') => Some(Action::Search),
            _ => None,
        };
    }
    match code {
        KeyCode::Up => return Some(Action::Move(-1)),
        KeyCode::Down => return Some(Action::Move(1)),
        KeyCode::Left => return Some(Action::PrevPane),
        KeyCode::Right => return Some(Action::NextPane),
        KeyCode::PageUp => return Some(Action::Move(-10)),
        KeyCode::PageDown => return Some(Action::Move(10)),
        KeyCode::Char(':') => return Some(Action::Palette),
        KeyCode::Char('=') if view == View::Graph => return Some(Action::GraphHops(1)),
        _ => {}
    }
    for command in COMMANDS {
        let Some(bound) = hint_key(command.hint) else {
            continue;
        };
        if bound != code || !command.applies_to(view) {
            continue;
        }
        return action_for_command(command.id, view);
    }
    None
}

pub fn tab_action(modifiers: KeyModifiers) -> Action {
    if modifiers.contains(KeyModifiers::SHIFT) {
        Action::PrevPane
    } else {
        Action::NextPane
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_filter_hides_non_palette_commands() {
        let hits = filtered("");
        assert!(hits.iter().all(|command| command.in_palette));
        assert!(hits.iter().any(|command| command.id == "ingest"));
        assert!(hits.iter().all(|command| command.id != "quit"));
    }

    #[test]
    fn generated_help_lists_registry_keys() {
        let help = help_text();
        assert!(help.contains("Command palette"));
        assert!(help.contains("Enter never writes") || help.contains("never writes"));
        assert!(help.contains("Palette writes open a y-confirm"), "{help}");
        assert!(help.contains('●') || help.contains("collected"));
        for command in COMMANDS.iter().filter(|command| !command.hint.is_empty()) {
            assert!(
                help.contains(command.hint),
                "help missing key {}",
                command.hint
            );
        }
    }

    #[test]
    fn enter_is_not_a_write_command() {
        let enter = COMMANDS
            .iter()
            .find(|command| command.id == "select")
            .expect("select");
        assert!(!enter.writes);
        assert_eq!(enter.hint, "Enter");
    }

    #[test]
    fn writes_flag_is_the_source_of_truth() {
        for command in COMMANDS {
            assert_eq!(command_writes(command.id), command.writes, "{}", command.id);
        }
        assert!(command_writes("run_tool:whois"));
        assert!(command_writes("confirm_resolve"));
        assert!(command_writes("confirm_unresolve"));
        assert!(!command_writes("help"));
        assert!(!command_writes("graph_confirm_cap"));
    }

    #[test]
    fn every_bound_key_appears_in_the_registry() {
        let registry: std::collections::HashSet<KeyCode> = COMMANDS
            .iter()
            .filter_map(|command| hint_key(command.hint))
            .collect();
        let extras = [
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Char('='),
            KeyCode::Char(':'),
        ];
        let views = [
            View::Welcome,
            View::Overview,
            View::Evidence,
            View::Review,
            View::Pages,
            View::Sources,
            View::Findings,
            View::Entities,
            View::Timeline,
            View::Notes,
            View::Agent,
            View::Jobs,
            View::Tools,
            View::Settings,
            View::Artifacts,
            View::Object,
            View::Graph,
            View::Audit,
        ];
        let mut codes = vec![
            KeyCode::Esc,
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Enter,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Char(' '),
        ];
        for byte in b'!'..=b'~' {
            codes.push(KeyCode::Char(byte as char));
        }
        let overlay = Overlay::None;
        for view in views {
            for code in &codes {
                if map_registered_key(*code, view, false, &overlay).is_none() {
                    continue;
                }
                assert!(
                    registry.contains(code) || extras.contains(code),
                    "{code:?} on {view:?} is bound but missing from COMMANDS"
                );
            }
        }
    }

    #[test]
    fn number_keys_match_the_rail() {
        let overlay = Overlay::None;
        assert!(matches!(
            map_registered_key(KeyCode::Char('1'), View::Entities, false, &overlay),
            Some(Action::OpenOverview)
        ));
        assert!(matches!(
            map_registered_key(KeyCode::Char('2'), View::Overview, false, &overlay),
            Some(Action::OpenEntities)
        ));
        assert!(matches!(
            map_registered_key(KeyCode::Char('3'), View::Overview, false, &overlay),
            Some(Action::OpenEvidence)
        ));
        assert!(matches!(
            map_registered_key(KeyCode::Char('4'), View::Overview, false, &overlay),
            Some(Action::OpenGraph)
        ));
        assert!(matches!(
            map_registered_key(KeyCode::Char('5'), View::Overview, false, &overlay),
            Some(Action::OpenReview)
        ));
        assert!(matches!(
            map_registered_key(KeyCode::Char('6'), View::Overview, false, &overlay),
            Some(Action::OpenJobs)
        ));
        for ch in ['7', '8', '9', '0'] {
            assert!(
                map_registered_key(KeyCode::Char(ch), View::Overview, false, &overlay).is_none(),
                "{ch} must not jump views"
            );
        }
    }

    #[test]
    fn deleted_keys_are_unbound() {
        let overlay = Overlay::None;
        for ch in ['p', 'v', 'r', 'i', 'b', 'u', 'o', 'd', 'f', '[', ']'] {
            assert!(
                map_registered_key(KeyCode::Char(ch), View::Entities, false, &overlay).is_none(),
                "{ch} should be deleted"
            );
            assert!(
                map_registered_key(KeyCode::Char(ch), View::Welcome, false, &overlay).is_none(),
                "{ch} should be deleted on Welcome"
            );
        }
        assert!(
            map_registered_key(KeyCode::Char('l'), View::Entities, false, &overlay)
                .is_some_and(|action| matches!(action, Action::NextPane))
        );
    }

    #[test]
    fn no_key_has_two_meanings_on_one_view() {
        let overlay = Overlay::None;
        let views = [
            View::Welcome,
            View::Overview,
            View::Entities,
            View::Evidence,
            View::Graph,
            View::Review,
            View::Jobs,
        ];
        for view in views {
            let mut bound: std::collections::HashMap<KeyCode, &'static str> =
                std::collections::HashMap::new();
            for command in COMMANDS {
                let Some(code) = hint_key(command.hint) else {
                    continue;
                };
                if !command.applies_to(view) {
                    continue;
                }
                if let Some(existing) = bound.insert(code, command.id) {
                    let left = action_for_command(existing, view);
                    let right = action_for_command(command.id, view);
                    assert_eq!(
                        left.map(|action| format!("{action:?}")),
                        right.map(|action| format!("{action:?}")),
                        "{code:?} on {view:?} maps to both {existing} and {}",
                        command.id
                    );
                }
            }
            let _ = overlay;
        }
    }

    #[test]
    fn cut_functions_are_in_the_palette() {
        let hits = filtered("");
        for id in [
            "ingest",
            "run_playbook",
            "open_artifacts",
            "open_audit",
            "open_tools",
            "open_settings",
            "open_notes",
            "open_timeline",
            "open_agent",
            "unresolve_entity",
            "doctor",
            "open_investigation",
        ] {
            assert!(
                hits.iter().any(|command| command.id == id),
                "palette missing {id}"
            );
        }
    }

    #[test]
    fn collect_is_c_not_r() {
        let overlay = Overlay::None;
        assert!(matches!(
            map_registered_key(KeyCode::Char('c'), View::Overview, false, &overlay),
            Some(Action::Refresh)
        ));
        assert!(map_registered_key(KeyCode::Char('r'), View::Overview, false, &overlay).is_none());
    }
}
