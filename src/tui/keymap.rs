use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::ListItem;
use rusqlite::Connection;

use crate::db::{self, SavedKeyBinding};

use super::formatting::{display_width, fit_to_width, truncate_to_width};
use super::{App, FocusPane};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub(super) enum KeyAction {
    ToggleFocus,
    MoveDown,
    MoveUp,
    PageDown,
    PageUp,
    Activate,
    SpaceAction,
    Escape,
    ToggleArtistExpansion,
    ToggleKeymap,
    ToggleInfo,
    OpenPlaylist,
    ShrinkPane,
    GrowPane,
    CommandMode,
    FilterMode,
    PlaySelected,
    TogglePause,
    Stop,
    PlayNext,
    PlayPrevious,
    SeekBack,
    SeekForward,
    SeekBackMinute,
    SeekForwardMinute,
    ToggleContinuous,
    TogglePlayTarget,
    ToggleRepeat,
    ToggleShuffle,
    SelectCurrent,
    AddToPlaylist,
    RemoveFromPlaylist,
    RefreshLibrary,
    Quit,
}

impl KeyAction {
    fn id(self) -> &'static str {
        match self {
            Self::ToggleFocus => "toggle-focus",
            Self::MoveDown => "move-down",
            Self::MoveUp => "move-up",
            Self::PageDown => "page-down",
            Self::PageUp => "page-up",
            Self::Activate => "activate",
            Self::SpaceAction => "space-action",
            Self::Escape => "escape",
            Self::ToggleArtistExpansion => "toggle-artist-expansion",
            Self::ToggleKeymap => "toggle-keymap",
            Self::ToggleInfo => "toggle-info",
            Self::OpenPlaylist => "open-playlist",
            Self::ShrinkPane => "shrink-pane",
            Self::GrowPane => "grow-pane",
            Self::CommandMode => "command-mode",
            Self::FilterMode => "filter-mode",
            Self::PlaySelected => "play-selected",
            Self::TogglePause => "toggle-pause",
            Self::Stop => "stop",
            Self::PlayNext => "play-next",
            Self::PlayPrevious => "play-previous",
            Self::SeekBack => "seek-back",
            Self::SeekForward => "seek-forward",
            Self::SeekBackMinute => "seek-back-minute",
            Self::SeekForwardMinute => "seek-forward-minute",
            Self::ToggleContinuous => "toggle-continuous",
            Self::TogglePlayTarget => "toggle-play-target",
            Self::ToggleRepeat => "toggle-repeat",
            Self::ToggleShuffle => "toggle-shuffle",
            Self::SelectCurrent => "select-current",
            Self::AddToPlaylist => "add-to-playlist",
            Self::RemoveFromPlaylist => "remove-from-playlist",
            Self::RefreshLibrary => "refresh-library",
            Self::Quit => "quit",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "toggle-focus" => Self::ToggleFocus,
            "move-down" => Self::MoveDown,
            "move-up" => Self::MoveUp,
            "page-down" => Self::PageDown,
            "page-up" => Self::PageUp,
            "activate" => Self::Activate,
            "space-action" => Self::SpaceAction,
            "escape" => Self::Escape,
            "toggle-artist-expansion" => Self::ToggleArtistExpansion,
            "toggle-keymap" => Self::ToggleKeymap,
            "toggle-info" => Self::ToggleInfo,
            "open-playlist" => Self::OpenPlaylist,
            "shrink-pane" => Self::ShrinkPane,
            "grow-pane" => Self::GrowPane,
            "command-mode" => Self::CommandMode,
            "filter-mode" => Self::FilterMode,
            "play-selected" => Self::PlaySelected,
            "toggle-pause" => Self::TogglePause,
            "stop" => Self::Stop,
            "play-next" => Self::PlayNext,
            "play-previous" => Self::PlayPrevious,
            "seek-back" => Self::SeekBack,
            "seek-forward" => Self::SeekForward,
            "seek-back-minute" => Self::SeekBackMinute,
            "seek-forward-minute" => Self::SeekForwardMinute,
            "toggle-continuous" => Self::ToggleContinuous,
            "toggle-play-target" => Self::TogglePlayTarget,
            "toggle-repeat" => Self::ToggleRepeat,
            "toggle-shuffle" => Self::ToggleShuffle,
            "select-current" => Self::SelectCurrent,
            "add-to-playlist" => Self::AddToPlaylist,
            "remove-from-playlist" => Self::RemoveFromPlaylist,
            "refresh-library" => Self::RefreshLibrary,
            "quit" => Self::Quit,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub(super) struct KeySpec {
    code: KeyCode,
    modifiers: KeyModifiers,
}

impl KeySpec {
    const fn new(code: KeyCode) -> Self {
        Self {
            code,
            modifiers: KeyModifiers::NONE,
        }
    }

    const fn ctrl(code: KeyCode) -> Self {
        Self {
            code,
            modifiers: KeyModifiers::CONTROL,
        }
    }

    fn from_event(event: KeyEvent) -> Option<Self> {
        let modifiers = normalized_modifiers(event);
        match event.code {
            KeyCode::Char(_)
            | KeyCode::Enter
            | KeyCode::Tab
            | KeyCode::Backspace
            | KeyCode::Delete
            | KeyCode::Esc
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Home
            | KeyCode::End => Some(Self {
                code: event.code,
                modifiers,
            }),
            _ => None,
        }
    }

    fn label(&self) -> String {
        let mut parts = Vec::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            parts.push(String::from("Ctrl"));
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            parts.push(String::from("Alt"));
        }
        if self.modifiers.contains(KeyModifiers::SUPER) {
            parts.push(String::from("Super"));
        }
        parts.push(code_label(&self.code));
        parts.join("-")
    }

    fn storage(&self) -> String {
        let mut modifiers = Vec::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            modifiers.push("ctrl");
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            modifiers.push("alt");
        }
        if self.modifiers.contains(KeyModifiers::SUPER) {
            modifiers.push("super");
        }
        if modifiers.is_empty() {
            modifiers.push("none");
        }
        format!("{}:{}", modifiers.join(","), code_storage(&self.code))
    }

    fn from_storage(value: &str) -> Option<Self> {
        let (modifiers, code) = value.split_once(':')?;
        Some(Self {
            code: code_from_storage(code)?,
            modifiers: modifiers_from_storage(modifiers)?,
        })
    }

    fn is_reserved_command_key(&self) -> bool {
        self.code == KeyCode::Char(':') && self.modifiers == KeyModifiers::NONE
    }

    fn is_reserved_enter_key(&self) -> bool {
        self.code == KeyCode::Enter && self.modifiers == KeyModifiers::NONE
    }

    fn is_reserved_escape_key(&self) -> bool {
        self.code == KeyCode::Esc && self.modifiers == KeyModifiers::NONE
    }

    fn is_reserved_key(&self) -> bool {
        self.is_reserved_command_key()
            || self.is_reserved_enter_key()
            || self.is_reserved_escape_key()
    }
}

#[derive(Debug, Clone)]
pub(super) struct KeyBinding {
    section: &'static str,
    action: KeyAction,
    default_keys: &'static [KeySpec],
    label: &'static str,
}

const TOGGLE_FOCUS_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Tab)];
const MOVE_DOWN_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Down)];
const MOVE_UP_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Up)];
const PAGE_DOWN_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::PageDown)];
const PAGE_UP_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::PageUp)];
// Bare Enter is reserved because it owns activation and confirmation behavior.
const ACTIVATE_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Enter)];
const SPACE_ACTION_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char(' '))];
// Bare Esc is reserved because it owns cancellation and recovery behavior.
const ESCAPE_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Esc)];
const TOGGLE_ARTIST_EXPANSION_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char('e'))];
const TOGGLE_KEYMAP_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char('k'))];
const TOGGLE_INFO_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char('i'))];
const OPEN_PLAYLIST_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char('p'))];
const SHRINK_PANE_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char('{'))];
const GROW_PANE_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char('}'))];
// Bare ':' is reserved because it owns the transition into the command interface.
const COMMAND_MODE_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char(':'))];
const FILTER_MODE_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char('/'))];
const PLAY_SELECTED_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char('x'))];
const TOGGLE_PAUSE_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char('c'))];
const STOP_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char('v'))];
const PLAY_NEXT_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char('b'))];
const PLAY_PREVIOUS_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char('z'))];
const SEEK_BACK_KEYS: &[KeySpec] = &[
    KeySpec::new(KeyCode::Left),
    KeySpec::new(KeyCode::Char('h')),
];
const SEEK_FORWARD_KEYS: &[KeySpec] = &[
    KeySpec::new(KeyCode::Right),
    KeySpec::new(KeyCode::Char('l')),
];
const SEEK_BACK_MINUTE_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char(','))];
const SEEK_FORWARD_MINUTE_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char('.'))];
const TOGGLE_CONTINUOUS_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char('C'))];
const TOGGLE_PLAY_TARGET_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char('L'))];
const TOGGLE_REPEAT_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char('R'))];
const TOGGLE_SHUFFLE_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char('S'))];
const SELECT_CURRENT_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char('I'))];
const ADD_TO_PLAYLIST_KEYS: &[KeySpec] = &[
    KeySpec::new(KeyCode::Char('+')),
    KeySpec::new(KeyCode::Char('=')),
];
const REMOVE_FROM_PLAYLIST_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char('-'))];
const QUIT_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char('q'))];
const REFRESH_KEYS: &[KeySpec] = &[KeySpec::ctrl(KeyCode::Char('r'))];

pub(super) const KEY_BINDINGS: &[KeyBinding] = &[
    KeyBinding {
        section: "Navigation",
        action: KeyAction::ToggleFocus,
        default_keys: TOGGLE_FOCUS_KEYS,
        label: "cycle panes",
    },
    KeyBinding {
        section: "Navigation",
        action: KeyAction::MoveDown,
        default_keys: MOVE_DOWN_KEYS,
        label: "move down",
    },
    KeyBinding {
        section: "Navigation",
        action: KeyAction::MoveUp,
        default_keys: MOVE_UP_KEYS,
        label: "move up",
    },
    KeyBinding {
        section: "Navigation",
        action: KeyAction::PageDown,
        default_keys: PAGE_DOWN_KEYS,
        label: "page down",
    },
    KeyBinding {
        section: "Navigation",
        action: KeyAction::PageUp,
        default_keys: PAGE_UP_KEYS,
        label: "page up",
    },
    KeyBinding {
        section: "Navigation",
        action: KeyAction::Activate,
        default_keys: ACTIVATE_KEYS,
        label: "play or activate selection",
    },
    KeyBinding {
        section: "Navigation",
        action: KeyAction::SpaceAction,
        default_keys: SPACE_ACTION_KEYS,
        label: "expand or context action",
    },
    KeyBinding {
        section: "Navigation",
        action: KeyAction::Escape,
        default_keys: ESCAPE_KEYS,
        label: "cancel or clear active mode",
    },
    KeyBinding {
        section: "Navigation",
        action: KeyAction::ToggleArtistExpansion,
        default_keys: TOGGLE_ARTIST_EXPANSION_KEYS,
        label: "expand or collapse selected tree item",
    },
    KeyBinding {
        section: "Panes",
        action: KeyAction::ToggleKeymap,
        default_keys: TOGGLE_KEYMAP_KEYS,
        label: "toggle keymap pane",
    },
    KeyBinding {
        section: "Panes",
        action: KeyAction::ToggleInfo,
        default_keys: TOGGLE_INFO_KEYS,
        label: "toggle track info pane",
    },
    KeyBinding {
        section: "Panes",
        action: KeyAction::OpenPlaylist,
        default_keys: OPEN_PLAYLIST_KEYS,
        label: "open playlist pane",
    },
    KeyBinding {
        section: "Panes",
        action: KeyAction::ShrinkPane,
        default_keys: SHRINK_PANE_KEYS,
        label: "move boundary left/up",
    },
    KeyBinding {
        section: "Panes",
        action: KeyAction::GrowPane,
        default_keys: GROW_PANE_KEYS,
        label: "move boundary right/down",
    },
    KeyBinding {
        section: "Panes",
        action: KeyAction::CommandMode,
        default_keys: COMMAND_MODE_KEYS,
        label: "enter command mode",
    },
    KeyBinding {
        section: "Panes",
        action: KeyAction::FilterMode,
        default_keys: FILTER_MODE_KEYS,
        label: "enter filter mode",
    },
    KeyBinding {
        section: "Playback",
        action: KeyAction::PlaySelected,
        default_keys: PLAY_SELECTED_KEYS,
        label: "play selected item",
    },
    KeyBinding {
        section: "Playback",
        action: KeyAction::TogglePause,
        default_keys: TOGGLE_PAUSE_KEYS,
        label: "play or pause",
    },
    KeyBinding {
        section: "Playback",
        action: KeyAction::Stop,
        default_keys: STOP_KEYS,
        label: "stop",
    },
    KeyBinding {
        section: "Playback",
        action: KeyAction::PlayNext,
        default_keys: PLAY_NEXT_KEYS,
        label: "next track",
    },
    KeyBinding {
        section: "Playback",
        action: KeyAction::PlayPrevious,
        default_keys: PLAY_PREVIOUS_KEYS,
        label: "previous track",
    },
    KeyBinding {
        section: "Playback",
        action: KeyAction::SeekBack,
        default_keys: SEEK_BACK_KEYS,
        label: "seek back five seconds",
    },
    KeyBinding {
        section: "Playback",
        action: KeyAction::SeekForward,
        default_keys: SEEK_FORWARD_KEYS,
        label: "seek forward five seconds",
    },
    KeyBinding {
        section: "Playback",
        action: KeyAction::SeekBackMinute,
        default_keys: SEEK_BACK_MINUTE_KEYS,
        label: "seek back one minute",
    },
    KeyBinding {
        section: "Playback",
        action: KeyAction::SeekForwardMinute,
        default_keys: SEEK_FORWARD_MINUTE_KEYS,
        label: "seek forward one minute",
    },
    KeyBinding {
        section: "Playback",
        action: KeyAction::ToggleContinuous,
        default_keys: TOGGLE_CONTINUOUS_KEYS,
        label: "toggle continuous",
    },
    KeyBinding {
        section: "Playback",
        action: KeyAction::TogglePlayTarget,
        default_keys: TOGGLE_PLAY_TARGET_KEYS,
        label: "cycle play target",
    },
    KeyBinding {
        section: "Playback",
        action: KeyAction::ToggleRepeat,
        default_keys: TOGGLE_REPEAT_KEYS,
        label: "toggle repeat",
    },
    KeyBinding {
        section: "Playback",
        action: KeyAction::ToggleShuffle,
        default_keys: TOGGLE_SHUFFLE_KEYS,
        label: "toggle shuffle",
    },
    KeyBinding {
        section: "Playback",
        action: KeyAction::SelectCurrent,
        default_keys: SELECT_CURRENT_KEYS,
        label: "select current track",
    },
    KeyBinding {
        section: "Playlists",
        action: KeyAction::AddToPlaylist,
        default_keys: ADD_TO_PLAYLIST_KEYS,
        label: "add selected item to active playlist",
    },
    KeyBinding {
        section: "Playlists",
        action: KeyAction::RemoveFromPlaylist,
        default_keys: REMOVE_FROM_PLAYLIST_KEYS,
        label: "remove selected item from active playlist",
    },
    KeyBinding {
        section: "System",
        action: KeyAction::Quit,
        default_keys: QUIT_KEYS,
        label: "quit",
    },
    KeyBinding {
        section: "System",
        action: KeyAction::RefreshLibrary,
        default_keys: REFRESH_KEYS,
        label: "refresh library from database",
    },
];

impl App {
    pub(super) fn load_key_bindings(&mut self, conn: &Connection) -> Result<()> {
        self.key_bindings.clear();
        for binding in db::key_bindings(conn)? {
            let Some(action) = KeyAction::from_id(&binding.action) else {
                continue;
            };
            let Some(key) = KeySpec::from_storage(&binding.key) else {
                continue;
            };
            if action_is_reserved(action) || key.is_reserved_key() {
                db::delete_key_binding_key(conn, action.id(), &binding.key)?;
                continue;
            }
            self.key_bindings.entry(action).or_default().push(key);
        }
        Ok(())
    }

    pub(super) fn toggle_keymap_panel(&mut self) {
        if self.keymap_panel_open {
            self.keymap_panel_open = false;
            self.keymap_capture_action = None;
            if self.focus == FocusPane::Keymap {
                self.focus = FocusPane::Tree;
            }
            self.message = String::from("keymap panel hidden");
        } else {
            self.keymap_panel_open = true;
            self.playlist_panel_open = false;
            if self.selected_keymap_row >= keymap_row_count() {
                self.selected_keymap_row = 0;
            }
            self.focus = FocusPane::Keymap;
            self.message = String::from("keymap panel");
        }
        self.apply_selection_state();
        self.show_transient_status(self.message.clone());
    }

    pub(super) fn move_keymap_selection(&mut self, direction: i32, amount: usize) {
        let len = keymap_row_count();
        if len == 0 {
            self.selected_keymap_row = 0;
            return;
        }

        if direction >= 0 {
            self.selected_keymap_row = (self.selected_keymap_row + amount).min(len - 1);
        } else {
            self.selected_keymap_row = self.selected_keymap_row.saturating_sub(amount);
        }
        self.keymap_capture_action = None;
        self.apply_selection_state();
    }

    pub(super) fn activate_keymap_selection(&mut self) {
        let Some(binding) = selected_keymap_binding(self.selected_keymap_row) else {
            self.message = String::from("select a key binding row to edit");
            self.show_transient_status(self.message.clone());
            return;
        };
        if binding_has_reserved_key(binding) {
            self.message = reserved_action_message(binding.action).to_string();
            self.show_transient_status(self.message.clone());
            return;
        }
        self.keymap_capture_action = Some(binding.action);
        self.message = format!("press new key for {}", binding.label);
        self.show_transient_status(self.message.clone());
    }

    pub(super) fn capture_key_binding(&mut self, conn: &Connection, key: KeyEvent) -> Result<bool> {
        let Some(action) = self.keymap_capture_action else {
            return Ok(false);
        };

        match key.code {
            KeyCode::Esc => {
                self.keymap_capture_action = None;
                self.message = String::from("Esc is reserved for cancellation and recovery");
                self.show_transient_status(self.message.clone());
                return Ok(true);
            }
            KeyCode::Backspace | KeyCode::Delete => {
                self.reset_key_binding(conn, action)?;
                return Ok(true);
            }
            _ => {}
        }

        let Some(spec) = KeySpec::from_event(key) else {
            self.message = String::from("unsupported key");
            self.show_transient_status(self.message.clone());
            return Ok(true);
        };

        self.set_key_binding(conn, action, spec)?;
        Ok(true)
    }

    pub(super) fn set_key_binding(
        &mut self,
        conn: &Connection,
        action: KeyAction,
        spec: KeySpec,
    ) -> Result<()> {
        if action_is_reserved(action) {
            self.message = reserved_action_message(action).to_string();
            self.show_transient_status(self.message.clone());
            return Ok(());
        }
        if let Some(message) = reserved_key_message(action, &spec) {
            self.message = message.to_string();
            self.show_transient_status(self.message.clone());
            return Ok(());
        }

        if effective_keys_for_action(self, action)
            .iter()
            .any(|key| key == &spec)
        {
            self.keymap_capture_action = None;
            self.message = format!("{} already includes {}", action_label(action), spec.label());
            self.show_transient_status(self.message.clone());
            return Ok(());
        }

        let conflicting_actions = self
            .key_bindings
            .iter_mut()
            .filter_map(|(other_action, other_specs)| {
                if *other_action == action {
                    return None;
                }
                let before = other_specs.len();
                other_specs.retain(|other_spec| other_spec != &spec);
                (other_specs.len() != before).then_some(*other_action)
            })
            .collect::<Vec<_>>();
        for conflicting_action in &conflicting_actions {
            db::delete_key_binding_key(conn, conflicting_action.id(), &spec.storage())?;
        }
        self.key_bindings.retain(|_action, specs| !specs.is_empty());
        self.key_bindings
            .entry(action)
            .or_default()
            .push(spec.clone());
        db::save_key_binding(
            conn,
            &SavedKeyBinding {
                action: action.id().to_string(),
                key: spec.storage(),
            },
        )?;
        self.keymap_capture_action = None;
        self.message = format!("{} mapped to {}", action_label(action), spec.label());
        self.show_transient_status(self.message.clone());
        Ok(())
    }

    pub(super) fn reset_key_binding(&mut self, conn: &Connection, action: KeyAction) -> Result<()> {
        self.key_bindings.remove(&action);
        db::delete_key_binding(conn, action.id())?;
        self.keymap_capture_action = None;
        self.message = format!("{} reset to default", action_label(action));
        self.show_transient_status(self.message.clone());
        Ok(())
    }

    pub(super) fn reset_key_bindings(&mut self, conn: &Connection) -> Result<()> {
        self.key_bindings.clear();
        db::delete_key_bindings(conn)?;
        self.keymap_capture_action = None;
        self.message = String::from("keymap reset to defaults");
        self.show_transient_status(self.message.clone());
        Ok(())
    }

    pub(super) fn key_action_for_event(&self, key: KeyEvent) -> Option<KeyAction> {
        let spec = KeySpec::from_event(key)?;
        if spec.is_reserved_command_key() {
            return Some(KeyAction::CommandMode);
        }
        if spec.is_reserved_enter_key() {
            return Some(KeyAction::Activate);
        }
        if spec.is_reserved_escape_key() {
            return Some(KeyAction::Escape);
        }
        self.key_bindings
            .iter()
            .find_map(|(action, custom_specs)| custom_specs.contains(&spec).then_some(*action))
            .or_else(|| {
                let any_custom_uses_key = self
                    .key_bindings
                    .values()
                    .any(|custom_specs| custom_specs.contains(&spec));
                if any_custom_uses_key {
                    return None;
                }
                KEY_BINDINGS.iter().find_map(|binding| {
                    binding
                        .default_keys
                        .iter()
                        .any(|default_spec| default_spec == &spec)
                        .then_some(binding.action)
                })
            })
    }
}

pub(super) fn keymap_items(app: &App, width: usize) -> Vec<ListItem<'static>> {
    let mut items: Vec<ListItem<'static>> = keymap_lines(app, width)
        .into_iter()
        .map(ListItem::new)
        .collect();
    if items.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            truncate_to_width(" no hotkeys configured", width),
            Style::default().fg(Color::DarkGray),
        ))));
    }

    items
}

pub(super) fn keymap_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut current_section = "";

    for binding in KEY_BINDINGS {
        if binding.section != current_section {
            current_section = binding.section;
            lines.push(Line::from(Span::styled(
                truncate_to_width(&format!(" {current_section}"), width),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        lines.push(keymap_binding_line(app, binding, width));
    }

    lines
}

pub(super) fn keymap_row_count() -> usize {
    let mut rows = 0;
    let mut current_section = "";
    for binding in KEY_BINDINGS {
        if binding.section != current_section {
            current_section = binding.section;
            rows += 1;
        }
        rows += 1;
    }
    rows
}

#[cfg(test)]
pub(super) fn keymap_row_for_action(action: KeyAction) -> Option<usize> {
    let mut current_section = "";
    let mut row = 0;
    for binding in KEY_BINDINGS {
        if binding.section != current_section {
            current_section = binding.section;
            row += 1;
        }
        if binding.action == action {
            return Some(row);
        }
        row += 1;
    }
    None
}

fn keymap_binding_line(app: &App, binding: &KeyBinding, width: usize) -> Line<'static> {
    let key_width = 22.min(width.saturating_sub(1));
    let effective = effective_key_text(app, binding);
    let key_text = fit_to_width(&format!("   {effective}"), key_width);
    let mut action = binding.label.to_string();
    let reserved = binding_has_reserved_key(binding);
    if let Some(capture_action) = app.keymap_capture_action {
        if capture_action == binding.action {
            action = String::from("press new key, Esc cancels, Backspace resets");
        }
    } else if app.key_bindings.contains_key(&binding.action) {
        action = format!("{action}  (default {})", default_key_text(binding));
    }
    if reserved && app.keymap_capture_action != Some(binding.action) {
        action = format!("{action}  (reserved)");
    }
    let action_width = width.saturating_sub(display_width(&key_text) + 1);
    let key_style = if reserved {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::Cyan)
    };
    Line::from(vec![
        Span::styled(key_text, key_style),
        Span::raw(" "),
        Span::styled(
            truncate_to_width(&action, action_width),
            Style::default().fg(Color::Gray),
        ),
    ])
}

fn selected_keymap_binding(row: usize) -> Option<&'static KeyBinding> {
    let mut current_section = "";
    let mut current_row = 0;
    for binding in KEY_BINDINGS {
        if binding.section != current_section {
            current_section = binding.section;
            if current_row == row {
                return None;
            }
            current_row += 1;
        }
        if current_row == row {
            return Some(binding);
        }
        current_row += 1;
    }
    None
}

fn effective_key_text(app: &App, binding: &KeyBinding) -> String {
    effective_keys_for_binding(app, binding)
        .iter()
        .map(KeySpec::label)
        .collect::<Vec<_>>()
        .join(" / ")
}

fn effective_keys_for_binding(app: &App, binding: &KeyBinding) -> Vec<KeySpec> {
    let mut keys = binding.default_keys.to_vec();
    if let Some(custom_keys) = app.key_bindings.get(&binding.action) {
        for key in custom_keys {
            if !keys.contains(key) {
                keys.push(key.clone());
            }
        }
    }
    keys
}

fn effective_keys_for_action(app: &App, action: KeyAction) -> Vec<KeySpec> {
    binding_for_action(action)
        .map(|binding| effective_keys_for_binding(app, binding))
        .unwrap_or_default()
}

fn default_key_text(binding: &KeyBinding) -> String {
    binding
        .default_keys
        .iter()
        .map(KeySpec::label)
        .collect::<Vec<_>>()
        .join(" / ")
}

fn action_label(action: KeyAction) -> &'static str {
    binding_for_action(action)
        .map(|binding| binding.label)
        .unwrap_or("key binding")
}

fn binding_for_action(action: KeyAction) -> Option<&'static KeyBinding> {
    KEY_BINDINGS.iter().find(|binding| binding.action == action)
}

fn reserved_key_message(action: KeyAction, spec: &KeySpec) -> Option<&'static str> {
    if spec.is_reserved_command_key() && action != KeyAction::CommandMode {
        Some("':' is reserved for command mode")
    } else if spec.is_reserved_enter_key() && action != KeyAction::Activate {
        Some("Enter is reserved for activation and confirmation")
    } else if spec.is_reserved_escape_key() && action != KeyAction::Escape {
        Some("Esc is reserved for cancellation and recovery")
    } else {
        None
    }
}

fn action_is_reserved(action: KeyAction) -> bool {
    binding_for_action(action).is_some_and(binding_has_reserved_key)
}

fn reserved_action_message(action: KeyAction) -> &'static str {
    match action {
        KeyAction::Activate => "Enter is reserved for activation and confirmation",
        KeyAction::CommandMode => "':' is reserved for command mode",
        KeyAction::Escape => "Esc is reserved for cancellation and recovery",
        _ => "reserved key cannot be edited",
    }
}

fn binding_has_reserved_key(binding: &KeyBinding) -> bool {
    binding.default_keys.iter().any(KeySpec::is_reserved_key)
}

fn normalized_modifiers(event: KeyEvent) -> KeyModifiers {
    let mut modifiers = event.modifiers;
    if matches!(event.code, KeyCode::Char(_)) {
        modifiers.remove(KeyModifiers::SHIFT);
    }
    modifiers
}

fn code_label(code: &KeyCode) -> String {
    match code {
        KeyCode::Char(' ') => String::from("Space"),
        KeyCode::Char(character) => character.to_string(),
        KeyCode::Enter => String::from("Enter"),
        KeyCode::Tab => String::from("Tab"),
        KeyCode::Backspace => String::from("Backspace"),
        KeyCode::Delete => String::from("Delete"),
        KeyCode::Esc => String::from("Esc"),
        KeyCode::Left => String::from("Left"),
        KeyCode::Right => String::from("Right"),
        KeyCode::Up => String::from("Up"),
        KeyCode::Down => String::from("Down"),
        KeyCode::PageUp => String::from("PageUp"),
        KeyCode::PageDown => String::from("PageDown"),
        KeyCode::Home => String::from("Home"),
        KeyCode::End => String::from("End"),
        _ => String::from("?"),
    }
}

fn code_storage(code: &KeyCode) -> String {
    match code {
        KeyCode::Char(character) => format!("char:{character}"),
        KeyCode::Enter => String::from("enter"),
        KeyCode::Tab => String::from("tab"),
        KeyCode::Backspace => String::from("backspace"),
        KeyCode::Delete => String::from("delete"),
        KeyCode::Esc => String::from("esc"),
        KeyCode::Left => String::from("left"),
        KeyCode::Right => String::from("right"),
        KeyCode::Up => String::from("up"),
        KeyCode::Down => String::from("down"),
        KeyCode::PageUp => String::from("pageup"),
        KeyCode::PageDown => String::from("pagedown"),
        KeyCode::Home => String::from("home"),
        KeyCode::End => String::from("end"),
        _ => String::from("unknown"),
    }
}

fn code_from_storage(value: &str) -> Option<KeyCode> {
    if let Some(character) = value.strip_prefix("char:") {
        return character.chars().next().map(KeyCode::Char);
    }
    Some(match value {
        "enter" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "esc" => KeyCode::Esc,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        _ => return None,
    })
}

fn modifiers_from_storage(value: &str) -> Option<KeyModifiers> {
    if value == "none" {
        return Some(KeyModifiers::NONE);
    }

    let mut modifiers = KeyModifiers::NONE;
    for part in value.split(',') {
        match part {
            "ctrl" => modifiers.insert(KeyModifiers::CONTROL),
            "alt" => modifiers.insert(KeyModifiers::ALT),
            "super" => modifiers.insert(KeyModifiers::SUPER),
            _ => return None,
        }
    }
    Some(modifiers)
}
