//! Maps terminal key events to `Action`s.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::{Action, Overlay, View};

/// Converts a key press into an action. `Repeat` is treated like `Press` so held arrows scroll.
pub fn map_key(key: KeyEvent, overlay: &Overlay, view: View, editing: bool) -> Option<Action> {
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && matches!(key.code, KeyCode::Char('c')) {
        return Some(Action::Quit);
    }
    if matches!(overlay, Overlay::Proposals { .. }) {
        return map_proposal_key(key);
    }
    if editing {
        return map_editing_key(key, overlay);
    }
    if !matches!(overlay, Overlay::None) {
        return map_overlay_key(key);
    }
    map_normal_key(key, view, ctrl)
}

fn map_proposal_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc => Some(Action::Cancel),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::Move(-1)),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::Move(1)),
        KeyCode::Char('A' | 'a' | 'y') => Some(Action::Accept),
        KeyCode::Char('R' | 'r' | 'n') => Some(Action::Reject),
        KeyCode::Enter => Some(Action::Accept),
        _ => None,
    }
}

fn map_editing_key(key: KeyEvent, overlay: &Overlay) -> Option<Action> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => Some(Action::Cancel),
        KeyCode::Enter => Some(Action::Submit),
        KeyCode::Tab => Some(if key.modifiers.contains(KeyModifiers::SHIFT) {
            Action::Move(-1)
        } else {
            Action::Move(1)
        }),
        KeyCode::BackTab => Some(Action::Move(-1)),
        KeyCode::Up => Some(Action::Move(-1)),
        KeyCode::Down => Some(Action::Move(1)),
        KeyCode::Left if matches!(overlay, Overlay::Form(_)) => Some(Action::Cycle(-1)),
        KeyCode::Right if matches!(overlay, Overlay::Form(_)) => Some(Action::Cycle(1)),
        KeyCode::Left => Some(Action::Left),
        KeyCode::Right => Some(Action::Right),
        KeyCode::Home => Some(Action::Home),
        KeyCode::End => Some(Action::End),
        KeyCode::Backspace => Some(Action::Backspace),
        KeyCode::Delete => Some(Action::Delete),
        KeyCode::Char('p') if ctrl => Some(Action::Palette),
        KeyCode::Char(ch) if !ctrl => Some(Action::InsertChar(ch)),
        _ => None,
    }
}

fn map_overlay_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => Some(Action::Cancel),
        KeyCode::Enter => Some(Action::Select),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::Move(-1)),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::Move(1)),
        KeyCode::PageUp => Some(Action::Scroll(-10)),
        KeyCode::PageDown => Some(Action::Scroll(10)),
        KeyCode::Char('?') => Some(Action::Help),
        _ => None,
    }
}

fn map_normal_key(key: KeyEvent, view: View, ctrl: bool) -> Option<Action> {
    if ctrl {
        return match key.code {
            KeyCode::Char('p') => Some(Action::Palette),
            KeyCode::Char('f') => Some(Action::Search),
            _ => None,
        };
    }
    match key.code {
        KeyCode::Char('q') => Some(Action::Quit),
        KeyCode::Esc => Some(Action::Back),
        KeyCode::Tab => Some(if key.modifiers.contains(KeyModifiers::SHIFT) {
            Action::PrevPane
        } else {
            Action::NextPane
        }),
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

pub const HELP_TEXT: &str = "\
os-int keyboard shortcuts

  q            Quit (or close an overlay)
  Esc          Back / close overlay
  Ctrl+C       Quit from anywhere
  Tab          Next pane          Shift+Tab  previous pane
  j / k        Move selection     Enter      open / select
  /  Ctrl+F    Search             p  Ctrl+P  command palette
  a            Ask agent          n          new investigation, page, or finding
  e            Edit notes in $EDITOR (Overview) or open Settings
  r            Run default collectors for this page
  i            Ingest STIX / MISP / CSV / JSON / Evidence Batch
  b            Run playbook (bundled YAML or a file)
  v            Object View for the selected entity or page
  g            Graph (entities as nodes, relationships as edges)
  s            Search Around from the selection (default 2 hops)
  + / -        Graph: more / fewer hops
  [ / ]        Graph: move facet cursor
  f            Graph: apply (or toggle) the highlighted facet
  x            Graph: clear facet filters
  Space        Entities: mark / unmark for merge
  m            Entities: propose merge of marked rows
  u            Object View / Entities: unresolve the current bag
  c            Cancel the selected job (Jobs view)
  ?            This help

  1 Overview  2 Pages  3 Sources  4 Findings  5 Entities
  6 Timeline  7 Agent  8 Jobs     9 Tools     0 Settings
  Graph is in the Navigate column (g or palette).

Enter on Entities opens Object View (properties, catalogued
links, sources, findings). Esc returns. Unknown link verbs
appear under Other.

Enter on Graph opens Object View for the selected node.
Search Around merges new hops into the canvas. Caps require
confirm before rendering a large neighborhood.

Mark two or more entities (Space) and press m to merge.
Suggestions never auto-merge. Object View lists constituents;
u unresolves. Esc cancels a confirm without applying.

Notes and Artifacts are in the Navigate column. On the welcome
screen, o opens a case and d runs doctor.

Agent proposals: Enter accepts, R rejects, Esc dismisses the list.

Badges: [COL] collected  [EXT] original material
        [ANL] you wrote  [AI]  agent inference

A domain page collects DNS, RDAP, HTTP, TLS, certificate
transparency, and subdomain expansion. An IP page collects RDAP
and IP intelligence. An email page collects DNS, CT, and the
email identity pack. A username page collects public profiles.
Press p and type dns, rdap, http, tls, ct, ip, email, username,
or subdomain for one collector. i ingests a STIX/MISP/CSV file.
Press b (or palette Run playbook) for domain-infra and other playbooks.
Watch and Playbook prompt are palette commands. Automations propose.

Collected material is untrusted evidence. Accepting an AI finding
keeps the AI badge. os-int observes public information.
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Action, Overlay};

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn palette_and_agent_keys_work_from_normal_mode() {
        let overlay = Overlay::None;
        assert!(matches!(
            map_key(press(KeyCode::Char('p')), &overlay, View::Overview, false),
            Some(Action::Palette)
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('a')), &overlay, View::Overview, false),
            Some(Action::AskAgent)
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('/')), &overlay, View::Overview, false),
            Some(Action::Search)
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('r')), &overlay, View::Overview, false),
            Some(Action::Refresh)
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('i')), &overlay, View::Entities, false),
            Some(Action::Command("ingest"))
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('b')), &overlay, View::Entities, false),
            Some(Action::Command("run_playbook"))
        ));
        assert!(map_key(press(KeyCode::Char('i')), &overlay, View::Graph, false).is_some());
        assert!(matches!(
            map_key(press(KeyCode::Char('v')), &overlay, View::Entities, false),
            Some(Action::OpenObjectView)
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('g')), &overlay, View::Entities, false),
            Some(Action::OpenGraph)
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('s')), &overlay, View::Entities, false),
            Some(Action::SearchAround)
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('+')), &overlay, View::Graph, false),
            Some(Action::GraphHops(1))
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('-')), &overlay, View::Graph, false),
            Some(Action::GraphHops(-1))
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('[')), &overlay, View::Graph, false),
            Some(Action::GraphFacetMove(-1))
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char(']')), &overlay, View::Graph, false),
            Some(Action::GraphFacetMove(1))
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('x')), &overlay, View::Graph, false),
            Some(Action::GraphClearFilters)
        ));
        assert!(map_key(press(KeyCode::Char('f')), &overlay, View::Overview, false).is_none());
        assert!(matches!(
            map_key(press(KeyCode::Char('f')), &overlay, View::Graph, false),
            Some(Action::GraphApplyFacet)
        ));
        assert!(map_key(press(KeyCode::Char('-')), &overlay, View::Entities, false).is_none());
        assert!(map_key(press(KeyCode::Char('[')), &overlay, View::Overview, false).is_none());
        assert!(map_key(press(KeyCode::Char(']')), &overlay, View::Pages, false).is_none());
        assert!(map_key(press(KeyCode::Char('x')), &overlay, View::Findings, false).is_none());
        assert!(map_key(press(KeyCode::Char('+')), &overlay, View::Entities, false).is_none());
        assert!(matches!(
            map_key(press(KeyCode::Char(' ')), &overlay, View::Entities, false),
            Some(Action::ToggleMergeMark)
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('m')), &overlay, View::Entities, false),
            Some(Action::ProposeMerge)
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('u')), &overlay, View::Object, false),
            Some(Action::Unresolve)
        ));
        assert!(map_key(press(KeyCode::Char(' ')), &overlay, View::Graph, false).is_none());
        assert!(map_key(press(KeyCode::Char('m')), &overlay, View::Overview, false).is_none());
        assert!(map_key(press(KeyCode::Char('u')), &overlay, View::Pages, false).is_none());
    }

    #[test]
    fn editing_inserts_characters_and_does_not_treat_them_as_commands() {
        let overlay = Overlay::AgentPrompt {
            input: crate::app::Input::default(),
        };
        assert!(matches!(
            map_key(press(KeyCode::Char('a')), &overlay, View::Agent, true),
            Some(Action::InsertChar('a'))
        ));
        assert!(matches!(
            map_key(press(KeyCode::Esc), &overlay, View::Agent, true),
            Some(Action::Cancel)
        ));
        assert!(matches!(
            map_key(press(KeyCode::Enter), &overlay, View::Agent, true),
            Some(Action::Submit)
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('i')), &overlay, View::Entities, true),
            Some(Action::InsertChar('i'))
        ));
    }

    #[test]
    fn ingest_key_is_unique_in_normal_mode() {
        let overlay = Overlay::None;
        let ingest = map_key(press(KeyCode::Char('i')), &overlay, View::Entities, false);
        assert!(matches!(ingest, Some(Action::Command("ingest"))));
        let occupied: &[char] = &[
            'q', 'a', 'p', 'n', 'e', 'r', 'v', 'g', 's', 'm', 'u', 'j', 'k', '/', '?', 'b',
        ];
        assert!(!occupied.contains(&'i'));
        for ch in occupied {
            let action = map_key(press(KeyCode::Char(*ch)), &overlay, View::Entities, false);
            assert!(
                action.is_some(),
                "expected {ch} to stay bound so ingest cannot collide silently"
            );
            assert!(
                !matches!(action, Some(Action::Command("ingest"))),
                "{ch} must not also map to ingest"
            );
        }
    }

    #[test]
    fn playbook_key_is_unique_in_normal_mode() {
        let overlay = Overlay::None;
        let playbook = map_key(press(KeyCode::Char('b')), &overlay, View::Entities, false);
        assert!(matches!(playbook, Some(Action::Command("run_playbook"))));
        let occupied: &[char] = &[
            'q', 'a', 'p', 'n', 'e', 'r', 'v', 'g', 's', 'm', 'u', 'j', 'k', '/', '?', 'i',
        ];
        assert!(!occupied.contains(&'b'));
        for ch in occupied {
            let action = map_key(press(KeyCode::Char(*ch)), &overlay, View::Entities, false);
            assert!(
                action.is_some(),
                "expected {ch} to stay bound so playbook cannot collide silently"
            );
            assert!(
                !matches!(action, Some(Action::Command("run_playbook"))),
                "{ch} must not also map to run_playbook"
            );
        }
        assert!(matches!(
            map_key(press(KeyCode::Char('b')), &overlay, View::Entities, true),
            Some(Action::InsertChar('b'))
        ));
    }

    #[test]
    fn ctrl_c_always_quits() {
        let overlay = Overlay::Help;
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(matches!(
            map_key(key, &overlay, View::Overview, false),
            Some(Action::Quit)
        ));
    }
}
