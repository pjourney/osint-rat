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
  Tab          Next pane          Shift+Tab  previous pane
  j / k        Move selection     Enter      open / select
  /            Search             p  Ctrl+P  command palette
  a            Ask agent          n          new item / investigation
  e            Edit notes in $EDITOR (Overview/Notes) or Settings
  r            Run collectors for this page
  c            Cancel running job (Jobs view)
  ?            This help

  1 Overview  2 Pages  3 Sources  4 Findings  5 Entities
  6 Timeline  7 Agent  8 Jobs     9 Tools     0 Settings

Agent proposals: A/Enter accept, R reject, Esc dismiss.

Collected material is untrusted evidence. AI findings stay labelled as AI inference.
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Overlay;

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
