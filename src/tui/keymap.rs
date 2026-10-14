//! Maps terminal key events to `Action`s. Documented bindings live in the command registry.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::commands::{map_registered_key, tab_action};
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
    if let Overlay::Proposals { armed, .. } = overlay {
        return map_proposal_key(key, *armed);
    }
    if editing {
        return map_editing_key(key, overlay);
    }
    if matches!(overlay, Overlay::Confirm { .. }) {
        return map_confirm_key(key);
    }
    if !matches!(overlay, Overlay::None) {
        return map_overlay_key(key);
    }
    if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) && !ctrl {
        return Some(tab_action(key.modifiers));
    }
    map_registered_key(key.code, view, ctrl, overlay)
}

fn map_proposal_key(key: KeyEvent, armed: bool) -> Option<Action> {
    if armed {
        return match key.code {
            KeyCode::Char('y') => Some(Action::Accept),
            _ => Some(Action::Disarm),
        };
    }
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    match key.code {
        KeyCode::Esc => Some(Action::Cancel),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::Move(-1)),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::Move(1)),
        KeyCode::Char('A') | KeyCode::Char('a') if shift => Some(Action::Arm),
        KeyCode::Char('A') => Some(Action::Arm),
        KeyCode::Char('R' | 'r' | 'n') => Some(Action::Reject),
        _ => None,
    }
}

fn map_editing_key(key: KeyEvent, overlay: &Overlay) -> Option<Action> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => Some(Action::Cancel),
        KeyCode::Enter => match overlay {
            Overlay::Palette { .. } | Overlay::Search { .. } => Some(Action::Select),
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
        KeyCode::Char('p') if ctrl => Some(Action::Palette),
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

pub fn help_text() -> String {
    crate::app::commands::help_text()
}

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
            map_key(press(KeyCode::Char('[')), &overlay, View::Timeline, false),
            Some(Action::TimelineFacetMove(-1))
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char(']')), &overlay, View::Timeline, false),
            Some(Action::TimelineFacetMove(1))
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('f')), &overlay, View::Timeline, false),
            Some(Action::TimelineApplyFilter)
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('x')), &overlay, View::Timeline, false),
            Some(Action::TimelineClearFilter)
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('l')), &overlay, View::Timeline, false),
            Some(Action::ToggleNearby)
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('l')), &overlay, View::Entities, false),
            Some(Action::ToggleNearby)
        ));
        assert!(map_key(press(KeyCode::Char('l')), &overlay, View::Overview, false).is_none());
        assert!(map_key(press(KeyCode::Char('f')), &overlay, View::Entities, false).is_none());
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
        assert!(map_key(press(KeyCode::Enter), &overlay, View::Agent, true).is_none());
        let save = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert!(matches!(
            map_key(save, &overlay, View::Agent, true),
            Some(Action::Submit)
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('i')), &overlay, View::Entities, true),
            Some(Action::InsertChar('i'))
        ));
    }

    fn proposals(armed: bool) -> Overlay {
        Overlay::Proposals {
            run_id: uuid::Uuid::nil(),
            selected: 0,
            armed,
        }
    }

    #[test]
    fn proposals_require_shift_a_then_y() {
        let overlay = proposals(false);
        assert!(map_key(press(KeyCode::Enter), &overlay, View::Agent, false).is_none());
        assert!(map_key(press(KeyCode::Char('a')), &overlay, View::Agent, false).is_none());
        assert!(map_key(press(KeyCode::Char('y')), &overlay, View::Agent, false).is_none());
        let shift_a = KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT);
        assert!(matches!(
            map_key(shift_a, &overlay, View::Agent, false),
            Some(Action::Arm)
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('A')), &overlay, View::Agent, false),
            Some(Action::Arm)
        ));
        let armed = proposals(true);
        assert!(matches!(
            map_key(press(KeyCode::Char('y')), &armed, View::Agent, false),
            Some(Action::Accept)
        ));
        assert!(matches!(
            map_key(press(KeyCode::Enter), &armed, View::Agent, false),
            Some(Action::Disarm)
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('a')), &armed, View::Agent, false),
            Some(Action::Disarm)
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('j')), &armed, View::Agent, false),
            Some(Action::Disarm)
        ));
        assert!(matches!(
            map_key(press(KeyCode::Char('r')), &overlay, View::Agent, false),
            Some(Action::Reject)
        ));
    }

    #[test]
    fn confirm_overlay_requires_y_not_enter() {
        let overlay = Overlay::Confirm {
            title: "Confirm".into(),
            message: "Write?".into(),
            command: "confirm_resolve".into(),
        };
        assert!(map_key(press(KeyCode::Enter), &overlay, View::Entities, false).is_none());
        assert!(matches!(
            map_key(press(KeyCode::Char('y')), &overlay, View::Entities, false),
            Some(Action::Select)
        ));
        assert!(matches!(
            map_key(press(KeyCode::Esc), &overlay, View::Entities, false),
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
            map_key(press(KeyCode::Enter), &overlay, View::Overview, true),
            Some(Action::Select)
        ));
        let save = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert!(matches!(
            map_key(save, &overlay, View::Overview, true),
            Some(Action::Submit)
        ));
    }

    #[test]
    fn ingest_key_is_unique_in_normal_mode() {
        let overlay = Overlay::None;
        let ingest = map_key(press(KeyCode::Char('i')), &overlay, View::Entities, false);
        assert!(matches!(ingest, Some(Action::Command("ingest"))));
        let occupied: &[char] = &[
            'q', 'a', 'p', 'n', 'e', 'r', 'v', 'g', 's', 'm', 'u', 'j', 'k', '/', '?', 'b', 'l',
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
            'q', 'a', 'p', 'n', 'e', 'r', 'v', 'g', 's', 'm', 'u', 'j', 'k', '/', '?', 'i', 'l',
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
    fn nearby_key_is_unique_in_normal_mode() {
        let overlay = Overlay::None;
        let nearby = map_key(press(KeyCode::Char('l')), &overlay, View::Entities, false);
        assert!(matches!(nearby, Some(Action::ToggleNearby)));
        let occupied: &[char] = &[
            'q', 'a', 'p', 'n', 'e', 'r', 'v', 'g', 's', 'm', 'u', 'j', 'k', '/', '?', 'i', 'b',
        ];
        assert!(!occupied.contains(&'l'));
        for ch in occupied {
            let action = map_key(press(KeyCode::Char(*ch)), &overlay, View::Entities, false);
            assert!(action.is_some(), "expected {ch} to stay bound");
            assert!(
                !matches!(action, Some(Action::ToggleNearby)),
                "{ch} must not also map to nearby"
            );
        }
        assert!(matches!(
            map_key(press(KeyCode::Char('l')), &overlay, View::Entities, true),
            Some(Action::InsertChar('l'))
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
