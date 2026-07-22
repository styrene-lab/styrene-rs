//! Semantic TUI actions and context-derived bindings.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, Focus, InputMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    FocusNext,
    FocusPrevious,
    WorkspaceNext,
    WorkspacePrevious,
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    Activate,
    Back,
    Toggle,
    OpenPalette,
    OpenHelp,
    Search,
    Compose,
    PageUp,
    PageDown,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionHint {
    pub action: Action,
    pub key: &'static str,
    pub label: &'static str,
}

pub fn action_for_key(app: &App, key: KeyEvent) -> Option<Action> {
    if app.input_mode != InputMode::Normal {
        return None;
    }
    match (key.code, key.modifiers) {
        (KeyCode::Tab, modifiers) if !modifiers.contains(KeyModifiers::SHIFT) => {
            Some(Action::FocusNext)
        }
        (KeyCode::BackTab, _) | (KeyCode::Tab, KeyModifiers::SHIFT) => Some(Action::FocusPrevious),
        (KeyCode::Left, modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::WorkspacePrevious)
        }
        (KeyCode::Right, modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::WorkspaceNext)
        }
        (KeyCode::Up, _) => Some(Action::MoveUp),
        (KeyCode::Down, _) => Some(Action::MoveDown),
        (KeyCode::Left, _) => Some(Action::MoveLeft),
        (KeyCode::Right, _) => Some(Action::MoveRight),
        (KeyCode::Enter, _) => Some(Action::Activate),
        (KeyCode::Esc, _) => Some(Action::Back),
        (KeyCode::Char(' '), _) => Some(Action::Toggle),
        (KeyCode::Char('p'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::OpenPalette)
        }
        (KeyCode::F(1), _) | (KeyCode::Char('?'), _) => Some(Action::OpenHelp),
        (KeyCode::Char('/'), _) => Some(Action::Search),
        (KeyCode::PageUp, _) => Some(Action::PageUp),
        (KeyCode::PageDown, _) => Some(Action::PageDown),
        (KeyCode::Char('c'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::Quit)
        }
        _ => None,
    }
}

pub fn contextual_hints(app: &App) -> Vec<ActionHint> {
    let mut hints = match app.focus {
        Focus::Sidebar => vec![
            ActionHint { action: Action::MoveDown, key: "↑↓", label: "Select" },
            ActionHint { action: Action::Activate, key: "Enter", label: "Open" },
            ActionHint { action: Action::FocusNext, key: "Tab", label: "Next region" },
        ],
        Focus::Main => vec![
            ActionHint { action: Action::MoveDown, key: "↑↓", label: "Navigate" },
            ActionHint { action: Action::Activate, key: "Enter", label: "Activate" },
            ActionHint { action: Action::FocusNext, key: "Tab", label: "Next region" },
            ActionHint { action: Action::Back, key: "Esc", label: "Back" },
        ],
        Focus::Input => vec![
            ActionHint { action: Action::Activate, key: "Enter", label: "Confirm" },
            ActionHint { action: Action::Back, key: "Esc", label: "Cancel" },
        ],
    };
    if app.focus != Focus::Input {
        hints.push(ActionHint { action: Action::OpenPalette, key: "Ctrl+P", label: "Actions" });
        hints.push(ActionHint { action: Action::OpenHelp, key: "?", label: "Help" });
    }
    hints.truncate(5);
    hints
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn invariant_navigation_keys_map_to_semantic_actions() {
        let app = App::new();
        assert_eq!(
            action_for_key(&app, key(KeyCode::Tab, KeyModifiers::NONE)),
            Some(Action::FocusNext)
        );
        assert_eq!(
            action_for_key(&app, key(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Some(Action::FocusPrevious)
        );
        assert_eq!(
            action_for_key(&app, key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(Action::Activate)
        );
        assert_eq!(action_for_key(&app, key(KeyCode::Esc, KeyModifiers::NONE)), Some(Action::Back));
    }

    #[test]
    fn workspace_switching_is_explicit_and_tab_is_not_workspace_navigation() {
        let app = App::new();
        assert_eq!(
            action_for_key(&app, key(KeyCode::Left, KeyModifiers::CONTROL)),
            Some(Action::WorkspacePrevious)
        );
        assert_eq!(
            action_for_key(&app, key(KeyCode::Right, KeyModifiers::CONTROL)),
            Some(Action::WorkspaceNext)
        );
    }

    #[test]
    fn hidden_vim_aliases_are_not_part_of_required_vocabulary() {
        let app = App::new();
        assert_eq!(action_for_key(&app, key(KeyCode::Char('j'), KeyModifiers::NONE)), None);
        assert_eq!(action_for_key(&app, key(KeyCode::Char('k'), KeyModifiers::NONE)), None);
    }
}
