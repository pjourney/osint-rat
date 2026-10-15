//! Maps terminal key events to `Action`s. Documented bindings live in the command registry.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::commands::{map_registered_key, tab_action};
use crate::app::{Action, Overlay, View};

/// Converts a key press into an action. `Repeat` is treated like `Press` so held arrows scroll.
pub fn map_key(
    key: KeyEvent,
    overlay: &Overlay,
    view: View,
    editing: bool,
    review_armed: bool,
    board_zoomed: bool,
) -> Option<Action> {
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && matches!(key.code, KeyCode::Char('c')) {
        return Some(Action::Quit);
    }
    if board_zoomed {
        return Some(Action::Unzoom);
    }
    if view == View::Review
        && matches!(overlay, Overlay::None)
        && let Some(action) = map_review_key(key, review_armed)
    {
        return Some(action);
    }
    if editing {
        return map_editing_key(key, overlay);
    }
    if matches!(overlay, Overlay::Confirm { .. }) {
        return map_confirm_key(key);
    }
    if matches!(overlay, Overlay::ThemeCycle { .. }) {
        return map_theme_cycle_key(key);
    }
    if !matches!(overlay, Overlay::None) {
        return map_overlay_key(key);
    }
    if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) && !ctrl {
        return Some(tab_action(key.modifiers));
    }
    map_registered_key(key.code, view, ctrl, overlay)
}

fn map_review_key(key: KeyEvent, armed: bool) -> Option<Action> {
    if armed {
        return match key.code {
            KeyCode::Char('y' | 'Y') => Some(Action::Accept),
            _ => Some(Action::Disarm),
        };
    }
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    match key.code {
        KeyCode::Char('A') | KeyCode::Char('a') if shift => Some(Action::Arm),
        KeyCode::Char('y' | 'Y') => None,
        _ => None,
    }
}

fn map_editing_key(key: KeyEvent, overlay: &Overlay) -> Option<Action> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => Some(Action::Cancel),
        KeyCode::Enter => match overlay {
            Overlay::Palette { .. } | Overlay::Search { .. } | Overlay::TableFilter { .. } => {
                Some(Action::Select)
            }
            _ => None,
        },
        KeyCode::Char('s') if ctrl => Some(Action::Submit),
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
        KeyCode::Char('k') if ctrl => Some(Action::Palette),
        KeyCode::Char(ch) if !ctrl => Some(Action::InsertChar(ch)),
        _ => None,
    }
}

fn map_confirm_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q' | 'n') => Some(Action::Cancel),
        KeyCode::Char('y' | 'Y') => Some(Action::Select),
        KeyCode::Enter => None,
        KeyCode::Up | KeyCode::Char('k') => Some(Action::Move(-1)),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::Move(1)),
        KeyCode::Char('?') => Some(Action::Help),
        _ => None,
    }
}

fn map_theme_cycle_key(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => Some(Action::Cancel),
        KeyCode::Char('y' | 'Y') => Some(Action::Select),
        KeyCode::Enter => None,
        KeyCode::Up | KeyCode::Char('k') => Some(Action::Move(-1)),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::Move(1)),
        KeyCode::Char('?') => Some(Action::Help),
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

pub fn help_text_for(ascii: bool) -> String {
    crate::app::commands::help_text(ascii)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Action, Overlay};

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn map(key: KeyEvent, overlay: &Overlay, view: View) -> Option<Action> {
        map_key(key, overlay, view, false, false, false)
    }

    #[test]
    fn cockpit_keys_work_from_normal_mode() {
        let overlay = Overlay::None;
        let ctrl_k = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL);
        assert!(matches!(
            map(ctrl_k, &overlay, View::Overview),
            Some(Action::Palette)
        ));
        assert!(matches!(
            map(press(KeyCode::Char(':')), &overlay, View::Overview),
            Some(Action::Palette)
        ));
        assert!(map(press(KeyCode::Char('p')), &overlay, View::Overview).is_none());
        assert!(matches!(
            map(press(KeyCode::Char('a')), &overlay, View::Overview),
            Some(Action::AskAgent)
        ));
        assert!(matches!(
            map(press(KeyCode::Char('/')), &overlay, View::Overview),
            Some(Action::Search)
        ));
        assert!(matches!(
            map(press(KeyCode::Char('c')), &overlay, View::Overview),
            Some(Action::Refresh)
        ));
        assert!(map(press(KeyCode::Char('r')), &overlay, View::Overview).is_none());
        assert!(map(press(KeyCode::Char('i')), &overlay, View::Entities).is_none());
        assert!(map(press(KeyCode::Char('b')), &overlay, View::Entities).is_none());
        assert!(map(press(KeyCode::Char('v')), &overlay, View::Entities).is_none());
        assert!(matches!(
            map(press(KeyCode::Char('g')), &overlay, View::Entities),
            Some(Action::OpenGraph)
        ));
        assert!(matches!(
            map(press(KeyCode::Char('s')), &overlay, View::Entities),
            Some(Action::SearchAround)
        ));
        assert!(matches!(
            map(press(KeyCode::Char('+')), &overlay, View::Graph),
            Some(Action::GraphHops(1))
        ));
        assert!(matches!(
            map(press(KeyCode::Char('x')), &overlay, View::Graph),
            Some(Action::ClearBoard)
        ));
        assert!(matches!(
            map(press(KeyCode::Char('t')), &overlay, View::Evidence),
            Some(Action::ToggleTimeLens)
        ));
        assert!(matches!(
            map(press(KeyCode::Char('t')), &overlay, View::Entities),
            Some(Action::ToggleTimeLens)
        ));
        assert!(matches!(
            map(press(KeyCode::Char('x')), &overlay, View::Entities),
            Some(Action::ClearBoard)
        ));
        assert!(matches!(
            map(press(KeyCode::Char('z')), &overlay, View::Overview),
            Some(Action::ToggleZoom)
        ));
        assert!(matches!(
            map(press(KeyCode::Char('l')), &overlay, View::Entities),
            Some(Action::NextPane)
        ));
        assert!(matches!(
            map(press(KeyCode::Char('h')), &overlay, View::Entities),
            Some(Action::PrevPane)
        ));
        assert!(matches!(
            map(press(KeyCode::Char(' ')), &overlay, View::Entities),
            Some(Action::ToggleMergeMark)
        ));
        assert!(matches!(
            map(press(KeyCode::Char('m')), &overlay, View::Entities),
            Some(Action::ProposeMerge)
        ));
        assert!(map(press(KeyCode::Char('u')), &overlay, View::Object).is_none());
        assert!(map(press(KeyCode::Char('1')), &overlay, View::Jobs).is_some());
        assert!(map(press(KeyCode::Char('7')), &overlay, View::Overview).is_none());
    }

    #[test]
    fn editing_inserts_characters_and_does_not_treat_them_as_commands() {
        let overlay = Overlay::AgentPrompt {
            input: crate::app::Input::default(),
        };
        assert!(matches!(
            map_key(
                press(KeyCode::Char('a')),
                &overlay,
                View::Review,
                true,
                false,
                false
            ),
            Some(Action::InsertChar('a'))
        ));
        assert!(matches!(
            map_key(
                press(KeyCode::Esc),
                &overlay,
                View::Review,
                true,
                false,
                false
            ),
            Some(Action::Cancel)
        ));
        assert!(
            map_key(
                press(KeyCode::Enter),
                &overlay,
                View::Review,
                true,
                false,
                false
            )
            .is_none()
        );
        let save = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert!(matches!(
            map_key(save, &overlay, View::Review, true, false, false),
            Some(Action::Submit)
        ));
    }

    #[test]
    fn review_requires_shift_a_then_y() {
        let overlay = Overlay::None;
        assert!(
            map_key(
                press(KeyCode::Enter),
                &overlay,
                View::Review,
                false,
                false,
                false
            )
            .is_some_and(|action| matches!(action, Action::Select))
        );
        assert!(
            map_key(
                press(KeyCode::Char('a')),
                &overlay,
                View::Review,
                false,
                false,
                false
            )
            .is_some_and(|action| matches!(action, Action::AskAgent))
        );
        assert!(
            map_key(
                press(KeyCode::Char('y')),
                &overlay,
                View::Review,
                false,
                false,
                false
            )
            .is_none()
        );
        assert!(
            map_key(
                press(KeyCode::Char('A')),
                &overlay,
                View::Review,
                false,
                false,
                false
            )
            .is_none_or(|action| !matches!(action, Action::Arm)),
            "Caps Lock A without Shift must not arm"
        );
        let shift_a = KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT);
        assert!(matches!(
            map_key(shift_a, &overlay, View::Review, false, false, false),
            Some(Action::Arm)
        ));
        assert!(matches!(
            map_key(
                press(KeyCode::Char('y')),
                &overlay,
                View::Review,
                false,
                true,
                false
            ),
            Some(Action::Accept)
        ));
        assert!(matches!(
            map_key(
                press(KeyCode::Enter),
                &overlay,
                View::Review,
                false,
                true,
                false
            ),
            Some(Action::Disarm)
        ));
        assert!(matches!(
            map(press(KeyCode::Char('x')), &overlay, View::Review),
            Some(Action::Reject)
        ));
    }

    #[test]
    fn theme_cycle_saves_with_y_enter_is_noop() {
        let overlay = Overlay::ThemeCycle {
            original: "dark".into(),
            index: 0,
        };
        assert!(map(press(KeyCode::Enter), &overlay, View::Overview).is_none());
        assert!(matches!(
            map(press(KeyCode::Char('y')), &overlay, View::Overview),
            Some(Action::Select)
        ));
        assert!(matches!(
            map(press(KeyCode::Esc), &overlay, View::Overview),
            Some(Action::Cancel)
        ));
    }

    #[test]
    fn confirm_overlay_requires_y_not_enter() {
        let overlay = Overlay::Confirm {
            title: "Confirm".into(),
            message: "Write?".into(),
            command: "confirm_resolve".into(),
        };
        assert!(map(press(KeyCode::Enter), &overlay, View::Entities).is_none());
        assert!(matches!(
            map(press(KeyCode::Char('y')), &overlay, View::Entities),
            Some(Action::Select)
        ));
        assert!(matches!(
            map(press(KeyCode::Esc), &overlay, View::Entities),
            Some(Action::Cancel)
        ));
    }

    #[test]
    fn palette_enter_selects_without_submitting_a_write() {
        let overlay = Overlay::Palette {
            query: crate::app::Input::default(),
            selected: 0,
        };
        assert!(matches!(
            map_key(
                press(KeyCode::Enter),
                &overlay,
                View::Overview,
                true,
                false,
                false
            ),
            Some(Action::Select)
        ));
    }

    #[test]
    fn zoomed_board_any_key_returns() {
        let overlay = Overlay::None;
        assert!(matches!(
            map_key(
                press(KeyCode::Char('j')),
                &overlay,
                View::Entities,
                false,
                false,
                true
            ),
            Some(Action::Unzoom)
        ));
    }

    #[test]
    fn ctrl_c_always_quits() {
        let overlay = Overlay::Help;
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(matches!(
            map(key, &overlay, View::Overview),
            Some(Action::Quit)
        ));
    }
}
