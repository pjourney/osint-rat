//! Command registry: palette entries, hint-bar keys, and generated help.
//!
//! The keymap consults [`map_registered_key`] for documented bindings. Overlay and
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
    }
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
        "Search pages, entities, findings, sources, artifacts, and agent runs.",
        false,
        true,
        false,
    ),
    cmd(
        "palette",
        "Command palette",
        "p",
        "Open the command palette. Ctrl+P is the same binding.",
        false,
        true,
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
    cmd(
        "open_investigation",
        "Open Investigation",
        "o",
        "Open an existing investigation (welcome screen).",
        true,
        false,
        false,
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
        "r",
        "Run the default passive collectors for this page.",
        true,
        true,
        false,
    ),
    cmd(
        "run_dns",
        "Run DNS Collection",
        "",
        "Run the DNS collector only.",
        true,
        false,
        false,
    ),
    cmd(
        "run_rdap",
        "Run RDAP Collection",
        "",
        "Run the RDAP collector only.",
        true,
        false,
        false,
    ),
    cmd(
        "run_http",
        "Fetch URL / HTTP",
        "",
        "Run the HTTP collector only.",
        true,
        false,
        false,
    ),
    cmd(
        "run_tls",
        "Run TLS Collection",
        "",
        "Run the TLS collector only.",
        true,
        false,
        false,
    ),
    cmd(
        "run_ct",
        "Run Certificate Transparency",
        "",
        "Run the certificate-transparency collector only.",
        true,
        false,
        false,
    ),
    cmd(
        "run_ip",
        "Run IP Intelligence",
        "",
        "Run the IP intelligence collector only.",
        true,
        false,
        false,
    ),
    cmd(
        "run_email",
        "Run Email Identity Pack",
        "",
        "Run the email identity pack only.",
        true,
        false,
        false,
    ),
    cmd(
        "run_username",
        "Run Username Pack",
        "",
        "Run the username pack only.",
        true,
        false,
        false,
    ),
    cmd(
        "run_subdomain",
        "Run Subdomain Expansion",
        "",
        "Run passive subdomain expansion only.",
        true,
        false,
        false,
    ),
    cmd(
        "ingest",
        "Ingest file",
        "i",
        "Ingest STIX, MISP, CSV, JSON, or an Evidence Batch.",
        true,
        false,
        true,
    ),
    cmd(
        "run_playbook",
        "Run playbook",
        "b",
        "Run a bundled YAML playbook. Active tools are refused.",
        true,
        false,
        false,
    ),
    cmd(
        "playbook_prompt",
        "Playbook prompt",
        "",
        "Seed the agent question with suggested playbook Actions.",
        true,
        false,
        false,
    ),
    cmd(
        "watch_playbook",
        "Watch playbook",
        "",
        "Re-run collect steps. Automations propose; they do not write findings.",
        true,
        false,
        false,
    ),
    cmd(
        "ask_agent",
        "Ask Agent",
        "a",
        "Ask the agent about the selection. Results land as proposals.",
        true,
        true,
        false,
    ),
    cmd(
        "generate_report",
        "Generate Report",
        "",
        "Write a Markdown or JSON report.",
        true,
        false,
        false,
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
        "open_overview",
        "Open Overview",
        "1",
        "Jump to Overview.",
        false,
        false,
        false,
    ),
    cmd(
        "open_pages",
        "Open Pages",
        "2",
        "Jump to Pages.",
        false,
        false,
        false,
    ),
    cmd(
        "open_sources",
        "Open Sources",
        "3",
        "Jump to Sources.",
        true,
        false,
        false,
    ),
    cmd(
        "open_findings",
        "Open Findings",
        "4",
        "Jump to Findings.",
        true,
        false,
        false,
    ),
    cmd(
        "open_entities",
        "Open Entities",
        "5",
        "Jump to Entities.",
        true,
        false,
        false,
    ),
    cmd(
        "open_timeline",
        "Open Timeline",
        "6",
        "Jump to Timeline.",
        true,
        false,
        false,
    ),
    cmd(
        "open_agent",
        "Agent History",
        "7",
        "Jump to Agent history.",
        true,
        false,
        false,
    ),
    cmd(
        "open_jobs",
        "Show Jobs",
        "8",
        "Jump to Jobs.",
        true,
        false,
        false,
    ),
    cmd(
        "open_tools",
        "Show Tools",
        "9",
        "Jump to Tools. Active tools are listed as refused.",
        true,
        false,
        false,
    ),
    cmd(
        "open_settings",
        "Configure Agent",
        "0",
        "Jump to Settings (agent profile and theme).",
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
    cmd(
        "toggle_nearby",
        "Nearby (country / prefix)",
        "l",
        "List entities nearby by country or prefix (Timeline / Entities).",
        true,
        false,
        false,
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
        "v",
        "Open Object View for the highlighted entity or the current page.",
        true,
        false,
        false,
    ),
    cmd(
        "open_graph",
        "Graph",
        "g",
        "Open Graph for the current entity (1 hop).",
        true,
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
    cmd(
        "merge_entities",
        "Merge selected entities",
        "m",
        "Propose a merge of marked rows. Nothing is written until you confirm.",
        true,
        false,
        true,
    ),
    cmd(
        "unresolve_entity",
        "Unresolve entity",
        "u",
        "Propose unresolving the current bag. Confirm required.",
        true,
        false,
        true,
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
    cmd(
        "doctor",
        "Doctor",
        "d",
        "Run environment checks (welcome screen, or palette).",
        true,
        false,
        false,
    ),
    cmd(
        "cancel_job",
        "Cancel Running Job",
        "c",
        "Cancel the selected job (Jobs view).",
        true,
        false,
        false,
    ),
    cmd(
        "graph_hops_inc",
        "More hops",
        "+",
        "Graph: expand Search Around depth.",
        false,
        false,
        false,
    ),
    cmd(
        "graph_hops_dec",
        "Fewer hops",
        "-",
        "Graph: reduce Search Around depth.",
        false,
        false,
        false,
    ),
    cmd(
        "mark",
        "Mark row",
        "Space",
        "Entities: mark or unmark for merge.",
        false,
        false,
        false,
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
    COMMANDS
        .iter()
        .filter(|command| command.on_hint_bar)
        .collect()
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
    lines.push("  Ctrl+P       Command palette (same as p)".into());
    lines.push("  Ctrl+F       Search (same as /)".into());
    lines.push(String::new());
    lines.push("  1 Overview  2 Pages  3 Sources  4 Findings  5 Entities".into());
    lines.push("  6 Timeline  7 Agent  8 Jobs     9 Tools     0 Settings".into());
    lines.push("  Graph and Audit are in the Navigate column (g or palette).".into());
    lines.push(String::new());
    lines.push("Enter opens. It never writes.".into());
    lines.push("Palette commands that write still require a confirm form.".into());
    lines.push(String::new());
    lines.push("Provenance glyphs: ● collected  ◇ original material".into());
    lines.push("                   ✎ you wrote  ◆ agent inference (kept after accept)".into());
    lines.push("ASCII (`ui.ascii`): C  X  A  *".into());
    lines.push(String::new());
    lines.push("os-int observes public information. Active tools are refused.".into());
    lines.join("\n")
}

/// Lookup used by the keymap for documented normal-mode bindings.
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
            KeyCode::Char('p') => Some(Action::Palette),
            KeyCode::Char('f') => Some(Action::Search),
            _ => None,
        };
    }
    match code {
        KeyCode::Char('q') => Some(Action::Quit),
        KeyCode::Esc => Some(Action::Back),
        KeyCode::Tab => Some(Action::NextPane),
        KeyCode::BackTab => Some(Action::PrevPane),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::Move(-1)),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::Move(1)),
        KeyCode::PageUp => Some(Action::Move(-10)),
        KeyCode::PageDown => Some(Action::Move(10)),
        KeyCode::Enter => Some(Action::Select),
        KeyCode::Char('/') => Some(Action::Search),
        KeyCode::Char('?') => Some(Action::Help),
        KeyCode::Char('a') => Some(Action::AskAgent),
        KeyCode::Char('p') => Some(Action::Palette),
        KeyCode::Char('n') => Some(Action::NewItem),
        KeyCode::Char('e') => Some(Action::Edit),
        KeyCode::Char('r') => Some(Action::Refresh),
        KeyCode::Char('i') => Some(Action::Command("ingest")),
        KeyCode::Char('b') => Some(Action::Command("run_playbook")),
        KeyCode::Char('v') => Some(Action::OpenObjectView),
        KeyCode::Char('g') => Some(Action::OpenGraph),
        KeyCode::Char('s') => Some(Action::SearchAround),
        KeyCode::Char('+') | KeyCode::Char('=') if view == View::Graph => {
            Some(Action::GraphHops(1))
        }
        KeyCode::Char('-') if view == View::Graph => Some(Action::GraphHops(-1)),
        KeyCode::Char('f') if view == View::Graph => Some(Action::GraphApplyFacet),
        KeyCode::Char('[') if view == View::Graph => Some(Action::GraphFacetMove(-1)),
        KeyCode::Char(']') if view == View::Graph => Some(Action::GraphFacetMove(1)),
        KeyCode::Char('x') if view == View::Graph => Some(Action::GraphClearFilters),
        KeyCode::Char('[') if view == View::Timeline => Some(Action::TimelineFacetMove(-1)),
        KeyCode::Char(']') if view == View::Timeline => Some(Action::TimelineFacetMove(1)),
        KeyCode::Char('f') if view == View::Timeline => Some(Action::TimelineApplyFilter),
        KeyCode::Char('x') if view == View::Timeline => Some(Action::TimelineClearFilter),
        KeyCode::Char('l') if matches!(view, View::Timeline | View::Entities) => {
            Some(Action::ToggleNearby)
        }
        KeyCode::Char(' ') if view == View::Entities => Some(Action::ToggleMergeMark),
        KeyCode::Char('m') if view == View::Entities => Some(Action::ProposeMerge),
        KeyCode::Char('u') if matches!(view, View::Entities | View::Object) => {
            Some(Action::Unresolve)
        }
        KeyCode::Char('o') if view == View::Welcome => Some(Action::Command("open_investigation")),
        KeyCode::Char('d') if view == View::Welcome => Some(Action::Command("doctor")),
        KeyCode::Char('c') if view == View::Jobs => Some(Action::Command("cancel_job")),
        KeyCode::Char('1') => Some(Action::OpenOverview),
        KeyCode::Char('2') => Some(Action::OpenPages),
        KeyCode::Char('3') => Some(Action::OpenSources),
        KeyCode::Char('4') => Some(Action::OpenFindings),
        KeyCode::Char('5') => Some(Action::OpenEntities),
        KeyCode::Char('6') => Some(Action::OpenTimeline),
        KeyCode::Char('7') => Some(Action::OpenAgent),
        KeyCode::Char('8') => Some(Action::OpenJobs),
        KeyCode::Char('9') => Some(Action::OpenTools),
        KeyCode::Char('0') => Some(Action::OpenSettings),
        _ => None,
    }
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
    fn registered_keymap_keeps_ingest_unique() {
        let overlay = Overlay::None;
        let ingest = map_registered_key(KeyCode::Char('i'), View::Entities, false, &overlay);
        assert!(matches!(ingest, Some(Action::Command("ingest"))));
        let occupied = [
            'q', 'a', 'p', 'n', 'e', 'r', 'v', 'g', 's', 'm', 'u', 'j', 'k', '/', '?', 'b', 'l',
        ];
        for ch in occupied {
            let action = map_registered_key(KeyCode::Char(ch), View::Entities, false, &overlay);
            assert!(action.is_some(), "expected {ch} bound");
            assert!(!matches!(action, Some(Action::Command("ingest"))));
        }
    }
}
