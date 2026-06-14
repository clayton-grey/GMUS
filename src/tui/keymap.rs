use std::collections::HashMap;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::ListItem;
use rusqlite::Connection;

use crate::db::{self, SavedKeyBinding};

use super::formatting::{display_width, fit_to_width, truncate_to_width};
use super::{App, FocusPane};

#[derive(Debug, Default)]
pub(super) struct KeymapPanelState {
    selected_row: usize,
    capture_action: Option<KeyAction>,
}

impl KeymapPanelState {
    pub(super) fn selected_row(&self) -> usize {
        self.selected_row
    }

    pub(super) fn select_row(&mut self, row: usize) {
        if self.selected_row != row {
            self.selected_row = row;
            self.cancel_capture();
        }
    }

    pub(super) fn clamp_selection(&mut self, row_count: usize) {
        let row = if row_count == 0 {
            0
        } else {
            self.selected_row.min(row_count - 1)
        };
        self.select_row(row);
    }

    pub(super) fn move_selection(&mut self, direction: i32, amount: usize, row_count: usize) {
        if row_count == 0 {
            self.selected_row = 0;
            self.cancel_capture();
            return;
        }
        self.selected_row = if direction >= 0 {
            self.selected_row.saturating_add(amount).min(row_count - 1)
        } else {
            self.selected_row.saturating_sub(amount)
        };
        self.cancel_capture();
    }

    pub(super) fn capture_action(&self) -> Option<KeyAction> {
        self.capture_action
    }

    pub(super) fn begin_capture(&mut self, action: KeyAction) {
        self.capture_action = Some(action);
    }

    pub(super) fn cancel_capture(&mut self) {
        self.capture_action = None;
    }

    pub(super) fn is_capturing(&self) -> bool {
        self.capture_action.is_some()
    }
}

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
    RateMode,
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
            Self::RateMode => "rate-mode",
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
        if modifiers.intersects(KeyModifiers::HYPER | KeyModifiers::META) {
            return None;
        }
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
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            parts.push(String::from("Shift"));
        }
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
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            modifiers.push("shift");
        }
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

    fn is_reserved_quit_key(&self) -> bool {
        self.code == KeyCode::Char('c') && self.modifiers.contains(KeyModifiers::CONTROL)
    }

    fn is_reserved_key(&self) -> bool {
        self.is_reserved_command_key()
            || self.is_reserved_enter_key()
            || self.is_reserved_escape_key()
            || self.is_reserved_quit_key()
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
const RATE_MODE_KEYS: &[KeySpec] = &[KeySpec::new(KeyCode::Char('r'))];
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
const QUIT_KEYS: &[KeySpec] = &[
    KeySpec::new(KeyCode::Char('q')),
    KeySpec::ctrl(KeyCode::Char('c')),
];
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
        action: KeyAction::RateMode,
        default_keys: RATE_MODE_KEYS,
        label: "enter playback rate",
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
        let saved_bindings = db::key_bindings(conn)?;
        let mut assigned_keys = Vec::new();
        for known_binding in KEY_BINDINGS {
            for binding in saved_bindings
                .iter()
                .filter(|binding| binding.action == known_binding.action.id())
            {
                let Some(key) = KeySpec::from_storage(&binding.key) else {
                    db::delete_key_binding_key(conn, known_binding.action.id(), &binding.key)?;
                    continue;
                };
                let canonical_key = key.storage();
                if binding.key != canonical_key {
                    db::delete_key_binding_key(conn, known_binding.action.id(), &binding.key)?;
                    if saved_bindings.iter().any(|candidate| {
                        candidate.action == binding.action && candidate.key == canonical_key
                    }) {
                        continue;
                    }
                }
                if action_is_reserved(known_binding.action)
                    || key.is_reserved_key()
                    || assigned_keys.contains(&key)
                {
                    db::delete_key_binding_key(conn, known_binding.action.id(), &binding.key)?;
                    continue;
                }
                if binding.key != canonical_key {
                    db::save_key_binding(
                        conn,
                        &SavedKeyBinding {
                            action: known_binding.action.id().to_string(),
                            key: canonical_key,
                        },
                    )?;
                }
                assigned_keys.push(key.clone());
                self.key_bindings
                    .entry(known_binding.action)
                    .or_default()
                    .push(key);
            }
        }
        Ok(())
    }

    pub(super) fn toggle_keymap_panel(&mut self) {
        if self.management_panel.keymap_open() {
            self.management_panel.hide_keymap();
            if self.focus == FocusPane::Keymap {
                self.focus = FocusPane::Tree;
            }
            self.message = String::from("keymap panel hidden");
        } else {
            self.management_panel.show_keymap();
            if self.management_panel.keymap.selected_row() >= keymap_row_count() {
                self.management_panel.keymap.select_row(0);
            }
            self.focus = FocusPane::Keymap;
            self.message = String::from("keymap panel");
        }
        self.apply_selection_state();
        self.show_transient_status(self.message.clone());
    }

    pub(super) fn move_keymap_selection(&mut self, direction: i32, amount: usize) {
        self.management_panel
            .keymap
            .move_selection(direction, amount, keymap_row_count());
        self.apply_selection_state();
    }

    pub(super) fn activate_keymap_selection(&mut self) {
        let Some(binding) = selected_keymap_binding(self.management_panel.keymap.selected_row())
        else {
            self.message = String::from("select a key binding row to edit");
            self.show_transient_status(self.message.clone());
            return;
        };
        if binding_has_reserved_key(binding) {
            self.message = reserved_action_message(binding.action).to_string();
            self.show_transient_status(self.message.clone());
            return;
        }
        self.management_panel.begin_keymap_capture(binding.action);
        self.message = format!("press new key for {}", binding.label);
        self.show_transient_status(self.message.clone());
    }

    pub(super) fn capture_key_binding(&mut self, conn: &Connection, key: KeyEvent) -> Result<bool> {
        let Some(action) = self.management_panel.keymap.capture_action() else {
            return Ok(false);
        };

        let Some(spec) = KeySpec::from_event(key) else {
            self.message = String::from("unsupported key");
            self.show_transient_status(self.message.clone());
            return Ok(true);
        };
        if spec.is_reserved_escape_key() {
            self.management_panel.keymap.cancel_capture();
            self.message = String::from("Esc is reserved for cancellation and recovery");
            self.show_transient_status(self.message.clone());
            return Ok(true);
        }
        if spec.modifiers == KeyModifiers::NONE
            && matches!(spec.code, KeyCode::Backspace | KeyCode::Delete)
        {
            self.reset_key_binding(conn, action)?;
            return Ok(true);
        }

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
            self.management_panel.keymap.cancel_capture();
            self.message = format!("{} already includes {}", action_label(action), spec.label());
            self.show_transient_status(self.message.clone());
            return Ok(());
        }

        let conflicts = remove_custom_key_conflicts(
            &mut self.key_bindings,
            action,
            std::slice::from_ref(&spec),
        );
        for (conflicting_action, conflicting_key) in conflicts {
            db::delete_key_binding_key(conn, conflicting_action.id(), &conflicting_key.storage())?;
        }
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
        self.management_panel.keymap.cancel_capture();
        self.message = format!("{} mapped to {}", action_label(action), spec.label());
        self.show_transient_status(self.message.clone());
        Ok(())
    }

    pub(super) fn reset_key_binding(&mut self, conn: &Connection, action: KeyAction) -> Result<()> {
        let default_keys = binding_for_action(action)
            .map(|binding| binding.default_keys)
            .unwrap_or_default();
        let conflicts = remove_custom_key_conflicts(&mut self.key_bindings, action, default_keys);
        for (conflicting_action, conflicting_key) in conflicts {
            db::delete_key_binding_key(conn, conflicting_action.id(), &conflicting_key.storage())?;
        }
        self.key_bindings.remove(&action);
        db::delete_key_binding(conn, action.id())?;
        self.management_panel.keymap.cancel_capture();
        self.message = format!("{} reset to default", action_label(action));
        self.show_transient_status(self.message.clone());
        Ok(())
    }

    pub(super) fn reset_key_bindings(&mut self, conn: &Connection) -> Result<()> {
        self.key_bindings.clear();
        db::delete_key_bindings(conn)?;
        self.management_panel.keymap.cancel_capture();
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
        if spec.is_reserved_quit_key() {
            return Some(KeyAction::Quit);
        }
        custom_action_for_key(self, &spec).or_else(|| {
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
    if let Some(capture_action) = app.management_panel.keymap.capture_action() {
        if capture_action == binding.action {
            action = String::from("press new key, Esc cancels, Backspace resets");
        }
    } else if app.key_bindings.contains_key(&binding.action) {
        let defaults = default_key_text(app, binding);
        if !defaults.is_empty() {
            action = format!("{action}  (default {defaults})");
        }
    }
    if reserved && app.management_panel.keymap.capture_action() != Some(binding.action) {
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
    let text = effective_keys_for_binding(app, binding)
        .iter()
        .map(KeySpec::label)
        .collect::<Vec<_>>()
        .join(" / ");
    if text.is_empty() {
        String::from("unbound")
    } else {
        text
    }
}

fn effective_keys_for_binding(app: &App, binding: &KeyBinding) -> Vec<KeySpec> {
    let mut keys = available_default_keys_for_binding(app, binding);
    if let Some(custom_keys) = app.key_bindings.get(&binding.action) {
        for key in custom_keys {
            if !keys.contains(key) {
                keys.push(key.clone());
            }
        }
    }
    keys
}

fn available_default_keys_for_binding(app: &App, binding: &KeyBinding) -> Vec<KeySpec> {
    binding
        .default_keys
        .iter()
        .filter(|key| custom_action_for_key(app, key).is_none_or(|action| action == binding.action))
        .cloned()
        .collect()
}

fn effective_keys_for_action(app: &App, action: KeyAction) -> Vec<KeySpec> {
    binding_for_action(action)
        .map(|binding| effective_keys_for_binding(app, binding))
        .unwrap_or_default()
}

fn default_key_text(app: &App, binding: &KeyBinding) -> String {
    available_default_keys_for_binding(app, binding)
        .iter()
        .map(KeySpec::label)
        .collect::<Vec<_>>()
        .join(" / ")
}

fn custom_action_for_key(app: &App, key: &KeySpec) -> Option<KeyAction> {
    KEY_BINDINGS.iter().find_map(|binding| {
        app.key_bindings
            .get(&binding.action)
            .is_some_and(|custom_keys| custom_keys.contains(key))
            .then_some(binding.action)
    })
}

fn remove_custom_key_conflicts(
    bindings: &mut HashMap<KeyAction, Vec<KeySpec>>,
    action: KeyAction,
    keys: &[KeySpec],
) -> Vec<(KeyAction, KeySpec)> {
    let mut conflicts = Vec::new();
    for (other_action, other_keys) in bindings.iter_mut() {
        if *other_action == action {
            continue;
        }
        other_keys.retain(|other_key| {
            if keys.contains(other_key) {
                conflicts.push((*other_action, other_key.clone()));
                false
            } else {
                true
            }
        });
    }
    bindings.retain(|_action, keys| !keys.is_empty());
    conflicts
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
    } else if spec.is_reserved_quit_key() {
        Some("Ctrl-C combinations are reserved for quitting")
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
    binding.default_keys.iter().any(|key| {
        key.is_reserved_command_key() || key.is_reserved_enter_key() || key.is_reserved_escape_key()
    })
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
        let mut characters = character.chars();
        let character = characters.next()?;
        return characters
            .next()
            .is_none()
            .then_some(KeyCode::Char(character));
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
            "shift" => modifiers.insert(KeyModifiers::SHIFT),
            "ctrl" => modifiers.insert(KeyModifiers::CONTROL),
            "alt" => modifiers.insert(KeyModifiers::ALT),
            "super" => modifiers.insert(KeyModifiers::SUPER),
            _ => return None,
        }
    }
    Some(modifiers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_movement_and_clamping_are_owned_together() {
        let mut panel = KeymapPanelState::default();
        panel.select_row(2);

        panel.move_selection(1, usize::MAX, 5);
        assert_eq!(panel.selected_row(), 4);

        panel.move_selection(-1, usize::MAX, 5);
        assert_eq!(panel.selected_row(), 0);

        panel.select_row(8);
        panel.clamp_selection(3);
        assert_eq!(panel.selected_row(), 2);

        panel.clamp_selection(0);
        assert_eq!(panel.selected_row(), 0);
    }

    #[test]
    fn closing_or_moving_cancels_capture_without_resetting_selection() {
        let mut panel = KeymapPanelState::default();
        panel.select_row(3);
        panel.begin_capture(KeyAction::ToggleInfo);
        panel.cancel_capture();

        assert!(!panel.is_capturing());
        assert_eq!(panel.selected_row(), 3);

        panel.begin_capture(KeyAction::ToggleInfo);
        panel.move_selection(1, 1, 6);
        assert!(!panel.is_capturing());
        assert_eq!(panel.selected_row(), 4);

        panel.begin_capture(KeyAction::ToggleInfo);
        panel.select_row(2);
        assert!(!panel.is_capturing());

        panel.begin_capture(KeyAction::ToggleInfo);
        panel.clamp_selection(1);
        assert!(!panel.is_capturing());
        assert_eq!(panel.selected_row(), 0);
    }

    #[test]
    fn key_specs_round_trip_supported_modifiers_and_reject_unsupported_ones() {
        let spec = KeySpec::from_event(KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT)).unwrap();

        assert_eq!(spec.label(), "Shift-Up");
        assert_eq!(spec.storage(), "shift:up");
        assert_eq!(KeySpec::from_storage(&spec.storage()), Some(spec));
        assert!(KeySpec::from_event(KeyEvent::new(KeyCode::Up, KeyModifiers::HYPER)).is_none());
        assert!(KeySpec::from_event(KeyEvent::new(KeyCode::Up, KeyModifiers::META)).is_none());
    }
}
