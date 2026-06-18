use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use rusqlite::Connection;

use crate::db::{self, SavedPaneLayout};

use super::command::{parse_playback_rate, playback_rate_message, RATE_USAGE};
use super::keymap::KeyAction;
use super::layout::{
    info_panel_target_height, library_pane_percent, INFO_PANE_STEP_ROWS, LIBRARY_PANE_STEP_PERCENT,
    WIDE_TREE_PERCENT,
};
use super::mouse::{mouse_pane, MouseLayout};
use super::{App, FocusPane, InputKind, TreeEntry};

const SCRUB_SECONDS: i64 = 5;
const MOUSE_SCROLL_LINES: usize = 1;

impl App {
    pub(super) fn move_down(&mut self) {
        self.move_pane_selection(self.focus, 1, 1);
    }

    pub(super) fn move_up(&mut self) {
        self.move_pane_selection(self.focus, -1, 1);
    }

    pub(super) fn page_down(&mut self) {
        self.move_pane_selection(self.focus, 1, 10);
    }

    pub(super) fn page_up(&mut self) {
        self.move_pane_selection(self.focus, -1, 10);
    }

    pub(super) fn move_command_selection(&mut self, direction: i32, amount: usize) {
        self.command_output.move_selection(direction, amount);
    }

    pub(super) fn move_pane_selection(&mut self, pane: FocusPane, direction: i32, amount: usize) {
        match pane {
            FocusPane::Tree => {
                let len = self.tree_entries().len();
                self.browser.move_tree_selection(direction, amount, len);
            }
            FocusPane::Tracks => {
                for _ in 0..amount {
                    if let Some(row) = self.next_track_row(direction) {
                        if row == self.browser.selected_track_row() {
                            break;
                        }
                        self.browser.select_track_row(row);
                    }
                }
            }
            FocusPane::Playlist => self.move_playlist_selection(direction, amount),
            FocusPane::Keymap => self.move_keymap_selection(direction, amount),
        }
        self.sync_selection();
    }

    pub(super) fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            FocusPane::Tree => FocusPane::Tracks,
            FocusPane::Tracks if self.management_panel.playlist_open() => FocusPane::Playlist,
            FocusPane::Tracks if self.management_panel.keymap_open() => FocusPane::Keymap,
            FocusPane::Tracks | FocusPane::Playlist | FocusPane::Keymap => FocusPane::Tree,
        };
    }

    pub(super) fn activate(&mut self, conn: &Connection) -> Result<()> {
        let focus = self.focus;
        match self.focus {
            FocusPane::Tree => {
                if let Some(entry) = self.first_selected_tree_playback_entry() {
                    self.play_entry(conn, entry)?;
                } else {
                    self.message = String::from("no tracks in this selection");
                }
            }
            FocusPane::Tracks => self.play_selected_row(conn)?,
            FocusPane::Playlist => self.activate_playlist_selection(conn)?,
            FocusPane::Keymap => self.activate_keymap_selection(),
        }
        self.focus = focus;
        self.sync_selection();
        Ok(())
    }

    pub(super) fn space_action(&mut self) {
        match self.focus {
            FocusPane::Tree => {
                self.toggle_artist_expansion();
                self.sync_selection();
            }
            FocusPane::Tracks => {
                self.message = String::from("space is tree expand; use x/c/v/b/z for playback");
            }
            FocusPane::Playlist => {
                self.toggle_selected_playlist_expansion();
                self.sync_selection();
            }
            FocusPane::Keymap => self.activate_keymap_selection(),
        }
    }

    pub(super) fn toggle_artist_expansion(&mut self) {
        let Some(entry) = self.selected_tree_entry().cloned() else {
            return;
        };
        if matches!(entry, TreeEntry::Playlists | TreeEntry::Playlist { .. }) {
            let collapsed = self.browser.playlists_expanded();
            let expanded = self.browser.toggle_playlists_expanded();
            if collapsed {
                let position = self
                    .tree_entries()
                    .iter()
                    .position(|entry| matches!(entry, TreeEntry::Playlists))
                    .unwrap_or(self.browser.selected_tree());
                self.browser.select_tree(position);
            }
            self.message = if expanded {
                String::from("expanded Playlists")
            } else {
                String::from("collapsed Playlists")
            };
            return;
        }
        if matches!(
            entry,
            TreeEntry::Compilation | TreeEntry::CompilationAlbum { .. }
        ) {
            let collapsed = self.browser.compilations_expanded();
            let expanded = self.browser.toggle_compilations_expanded();
            if collapsed {
                let position = self
                    .tree_entries()
                    .iter()
                    .position(|entry| matches!(entry, TreeEntry::Compilation))
                    .unwrap_or(self.browser.selected_tree());
                self.browser.select_tree(position);
            }
            self.message = if expanded {
                String::from("expanded Compilations")
            } else {
                String::from("collapsed Compilations")
            };
            return;
        }
        let artist = entry.artist().to_string();
        if self.browser.collapse_artist(&artist) {
            let position = self
                .tree_entries()
                .iter()
                .position(|entry| matches!(entry, TreeEntry::Artist { artist: entry_artist } if entry_artist == &artist))
                .unwrap_or(self.browser.selected_tree());
            self.browser.select_tree(position);
            self.message = format!("collapsed {artist}");
        } else {
            self.browser.expand_artist(artist.clone());
            self.message = format!("expanded {artist}");
        }
    }

    pub(super) fn handle_key(&mut self, conn: &Connection, key: KeyEvent) -> Result<bool> {
        self.dismiss_startup_info();

        if matches!(key.code, KeyCode::Char('c')) && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(true);
        }

        if self.management_panel.keymap.is_capturing() {
            self.capture_key_binding(conn, key)?;
            return Ok(false);
        }

        if self.input.kind() == InputKind::Command {
            match key.code {
                KeyCode::Esc => {
                    self.input.cancel_command();
                    if self.clear_command_output() {
                        self.message = String::from("output cleared");
                    } else if self.input.filter().is_empty() {
                        self.message = String::from("command cancelled");
                    } else {
                        self.clear_filter(conn)?;
                    }
                }
                KeyCode::Enter => self.submit_command(conn),
                KeyCode::Tab => self.complete_command(conn)?,
                KeyCode::Backspace => self.input.pop_command(),
                KeyCode::Char(char) => self.input.push_command(char),
                _ => {}
            }
            return Ok(false);
        }

        if self.input.kind() == InputKind::Rate {
            match key.code {
                KeyCode::Esc => self.cancel_rate_input(),
                KeyCode::Enter | KeyCode::Tab => self.confirm_rate_input()?,
                KeyCode::Backspace => self.input.pop_rate(),
                KeyCode::Char(char) => self.input.push_rate(char),
                _ => {}
            }
            return Ok(false);
        }

        if self.input.kind() == InputKind::Filter {
            match key.code {
                KeyCode::Esc => {
                    self.clear_filter(conn)?;
                }
                KeyCode::Enter | KeyCode::Tab => self.confirm_filter(conn)?,
                KeyCode::Backspace => {
                    self.input.pop_filter();
                    self.browser.reset_selection();
                    self.sync_selection();
                }
                KeyCode::Char(char) => {
                    self.input.push_filter(char);
                    self.browser.reset_selection();
                    self.sync_selection();
                }
                _ => {}
            }
            return Ok(false);
        }

        if self.command_output.is_focused() {
            return self.handle_command_focus_key(conn, key);
        }

        let action = self.key_action_for_event(key);
        if !matches!(key.code, KeyCode::Esc) && action != Some(KeyAction::CommandMode) {
            self.clear_command_output();
        }

        if matches!(key.code, KeyCode::Esc) {
            self.handle_escape(conn)?;
        } else if let Some(action) = action {
            return self.handle_key_action(conn, action);
        }
        Ok(false)
    }

    fn handle_key_action(&mut self, conn: &Connection, action: KeyAction) -> Result<bool> {
        match action {
            KeyAction::Quit => {
                return Ok(true);
            }
            KeyAction::RefreshLibrary => self.refresh(conn)?,
            KeyAction::ToggleFocus => self.toggle_focus(),
            KeyAction::MoveDown => self.move_down(),
            KeyAction::MoveUp => self.move_up(),
            KeyAction::PageDown => self.page_down(),
            KeyAction::PageUp => self.page_up(),
            KeyAction::Activate => self.activate(conn)?,
            KeyAction::SpaceAction => self.space_action(),
            KeyAction::Escape => self.handle_escape(conn)?,
            KeyAction::ToggleArtistExpansion => {
                self.toggle_artist_expansion();
                self.sync_selection();
            }
            KeyAction::ToggleKeymap => self.toggle_keymap_panel(),
            KeyAction::ToggleInfo => {
                if self.management_panel.playlist_open() || self.management_panel.keymap_open() {
                    self.show_track_info_panel();
                } else {
                    self.toggle_info_panel();
                }
            }
            KeyAction::OpenPlaylist => self.open_playlist_panel(conn)?,
            KeyAction::ShrinkPane => self.resize_focused_pane(conn, false)?,
            KeyAction::GrowPane => self.resize_focused_pane(conn, true)?,
            KeyAction::CommandMode => self.enter_command_mode(),
            KeyAction::FilterMode => self.enter_filter_mode(conn)?,
            KeyAction::RateMode => self.enter_rate_mode(),
            KeyAction::PlaySelected => self.play_from_controls(conn)?,
            KeyAction::TogglePause => self.toggle_pause(conn)?,
            KeyAction::Stop => self.stop_current(conn)?,
            KeyAction::PlayNext => self.play_next(conn)?,
            KeyAction::PlayPrevious => self.play_previous(conn)?,
            KeyAction::SeekBack => self.seek_relative(-SCRUB_SECONDS)?,
            KeyAction::SeekForward => self.seek_relative(SCRUB_SECONDS)?,
            KeyAction::SeekBackMinute => self.seek_relative(-60)?,
            KeyAction::SeekForwardMinute => self.seek_relative(60)?,
            KeyAction::ToggleContinuous => self.toggle_continuous(),
            KeyAction::TogglePlayTarget => self.toggle_play_target(),
            KeyAction::ToggleRepeat => self.toggle_repeat(),
            KeyAction::ToggleShuffle => self.toggle_shuffle(),
            KeyAction::SelectCurrent => self.select_current_track(),
            KeyAction::AddToPlaylist => self.add_selected_tracks_to_active_playlist(conn)?,
            KeyAction::RemoveFromPlaylist => {
                self.remove_selected_tracks_from_active_playlist(conn)?
            }
        }
        Ok(false)
    }

    pub(super) fn handle_command_focus_key(
        &mut self,
        conn: &Connection,
        key: KeyEvent,
    ) -> Result<bool> {
        match key.code {
            KeyCode::Char('q') => {
                return Ok(true);
            }
            KeyCode::Esc if self.clear_command_output() => {
                self.message = String::from("output cleared");
            }
            KeyCode::Esc => {}
            KeyCode::Down | KeyCode::Char('j') => self.move_command_selection(1, 1),
            KeyCode::Up | KeyCode::Char('k') => self.move_command_selection(-1, 1),
            KeyCode::PageDown => self.move_command_selection(1, 10),
            KeyCode::PageUp => self.move_command_selection(-1, 10),
            KeyCode::Enter | KeyCode::Char(' ') => self.toggle_selected_library_root(conn)?,
            KeyCode::Tab => {
                self.clear_command_output();
                self.focus = FocusPane::Tree;
                self.apply_selection_state();
            }
            KeyCode::Char(':') => self.enter_command_mode(),
            KeyCode::Char('/') => self.enter_filter_mode(conn)?,
            KeyCode::Char('r') => self.enter_rate_mode(),
            _ => {}
        }
        Ok(false)
    }

    fn enter_command_mode(&mut self) {
        self.input.enter_command();
        self.clear_command_output();
        self.message = String::from("typing command");
    }

    fn enter_filter_mode(&mut self, conn: &Connection) -> Result<()> {
        if !self.input.filter().is_empty() {
            self.clear_filter(conn)?;
        }
        self.input.enter_filter();
        self.clear_command_output();
        self.message = String::from("typing filter");
        Ok(())
    }

    fn enter_rate_mode(&mut self) {
        self.input.enter_rate();
        self.clear_command_output();
        self.message = String::from("typing playback rate");
    }

    pub(super) fn handle_mouse(
        &mut self,
        mouse: MouseEvent,
        terminal_width: u16,
        terminal_height: u16,
    ) -> bool {
        let dismissed_startup_info = self.dismiss_startup_info();
        if self.input.is_active() || self.command_output.is_focused() {
            return dismissed_startup_info;
        }

        let direction = match mouse.kind {
            MouseEventKind::ScrollDown => 1,
            MouseEventKind::ScrollUp => -1,
            _ => return dismissed_startup_info,
        };

        let layout = MouseLayout {
            terminal_width,
            terminal_height,
            reserved_bottom_rows: self.reserved_bottom_rows(),
            info_visible: self.info_area_visible(),
            input_visible: self.input_bar_visible(),
            playlist_info_visible: self.management_panel.playlist_open(),
            keymap_info_visible: self.management_panel.keymap_open(),
            library_pane_percent_offset: self.layout.library_pane_percent_offset(),
            info_pane_height_offset: self.layout.info_pane_height_offset(),
            column_layout_width: self.layout.column_layout_width(),
        };
        let Some(pane) = mouse_pane(mouse.column, mouse.row, layout) else {
            return dismissed_startup_info;
        };
        self.clear_command_output();
        self.move_pane_selection(pane, direction, MOUSE_SCROLL_LINES);
        true
    }

    pub(super) fn handle_escape(&mut self, conn: &Connection) -> Result<()> {
        if self.clear_command_output() {
            self.message = String::from("output cleared");
        } else {
            self.clear_filter(conn)?;
        }
        Ok(())
    }

    fn confirm_rate_input(&mut self) -> Result<()> {
        let value = self.input.rate().trim();
        if value.is_empty() {
            self.input.finish_rate();
            self.message = playback_rate_message(self.player.rate());
            return Ok(());
        }

        let Some(rate) = parse_playback_rate(value) else {
            self.message = String::from(RATE_USAGE);
            return Ok(());
        };
        self.player.set_rate(rate)?;
        self.input.finish_rate();
        self.message = playback_rate_message(rate);
        Ok(())
    }

    fn cancel_rate_input(&mut self) {
        self.input.finish_rate();
        self.message = String::from("rate cancelled");
    }

    fn resize_focused_pane(&mut self, conn: &Connection, grow: bool) -> Result<()> {
        match self.focus {
            FocusPane::Tree => {
                let delta = if grow {
                    LIBRARY_PANE_STEP_PERCENT
                } else {
                    -LIBRARY_PANE_STEP_PERCENT
                };
                self.resize_library_boundary(conn, delta, "library", grow)
            }
            FocusPane::Tracks => {
                let delta = if grow {
                    LIBRARY_PANE_STEP_PERCENT
                } else {
                    -LIBRARY_PANE_STEP_PERCENT
                };
                self.resize_library_boundary(conn, delta, "tracks", !grow)
            }
            FocusPane::Playlist | FocusPane::Keymap => {
                let delta = if grow {
                    -INFO_PANE_STEP_ROWS
                } else {
                    INFO_PANE_STEP_ROWS
                };
                self.resize_info_boundary(conn, delta, !grow)
            }
        }
    }

    fn resize_library_boundary(
        &mut self,
        conn: &Connection,
        delta: i16,
        pane: &str,
        grow: bool,
    ) -> Result<()> {
        let (previous, next) = self.layout.resize_library_pane(delta);
        self.save_pane_layout(conn)?;

        let direction = if grow { "larger" } else { "smaller" };
        let split = library_pane_percent(WIDE_TREE_PERCENT, next);
        self.message = if previous == next {
            format!("{pane} pane size limit (library split {split}%)")
        } else {
            format!("{pane} pane {direction} (library split {split}%)")
        };
        self.show_transient_status(self.message.clone());
        Ok(())
    }

    fn resize_info_boundary(&mut self, conn: &Connection, delta: i16, grow: bool) -> Result<()> {
        let (previous, next) = self.layout.resize_info_pane(delta);
        self.save_pane_layout(conn)?;

        let direction = if grow { "larger" } else { "smaller" };
        let height = info_panel_target_height(next);
        self.message = if previous == next {
            format!("info pane size limit ({height} rows)")
        } else {
            format!("info pane {direction} ({height} rows)")
        };
        self.show_transient_status(self.message.clone());
        Ok(())
    }

    fn save_pane_layout(&self, conn: &Connection) -> Result<()> {
        db::save_pane_layout(
            conn,
            SavedPaneLayout {
                library_percent_offset: self.layout.library_pane_percent_offset(),
                info_height_offset: self.layout.info_pane_height_offset(),
            },
        )
    }

    fn dismiss_startup_info(&mut self) -> bool {
        self.layout.dismiss_startup_info()
    }
}
