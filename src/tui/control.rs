use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use rusqlite::Connection;

use crate::db::{self, SavedPaneLayout};

use super::keymap::KeyAction;
use super::layout::{
    clamp_info_panel_offset, clamp_library_pane_offset, info_panel_target_height,
    library_pane_percent, INFO_PANE_STEP_ROWS, LIBRARY_PANE_STEP_PERCENT, WIDE_TREE_PERCENT,
};
use super::mouse::{mouse_pane, MouseLayout};
use super::{App, FocusPane, TreeEntry};

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
        if self.command_roots.is_empty() {
            self.command_selected = 0;
            return;
        }

        if direction >= 0 {
            self.command_selected =
                (self.command_selected + amount).min(self.command_roots.len() - 1);
        } else {
            self.command_selected = self.command_selected.saturating_sub(amount);
        }
    }

    pub(super) fn move_pane_selection(&mut self, pane: FocusPane, direction: i32, amount: usize) {
        match pane {
            FocusPane::Tree => {
                let len = self.tree_entries().len();
                if len > 0 {
                    if direction >= 0 {
                        self.selected_tree = (self.selected_tree + amount).min(len - 1);
                    } else {
                        self.selected_tree = self.selected_tree.saturating_sub(amount);
                    }
                    self.selected_track_row = 0;
                }
            }
            FocusPane::Tracks => {
                for _ in 0..amount {
                    if let Some(row) = self.next_track_row(direction) {
                        if row == self.selected_track_row {
                            break;
                        }
                        self.selected_track_row = row;
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
            FocusPane::Tracks if self.playlist_panel_open => FocusPane::Playlist,
            FocusPane::Tracks if self.keymap_panel_open => FocusPane::Keymap,
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
            let collapsed = self.playlists_expanded;
            self.playlists_expanded = !self.playlists_expanded;
            if collapsed {
                self.selected_tree = self
                    .tree_entries()
                    .iter()
                    .position(|entry| matches!(entry, TreeEntry::Playlists))
                    .unwrap_or(self.selected_tree);
            }
            self.message = if self.playlists_expanded {
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
            let collapsed = self.compilations_expanded;
            self.compilations_expanded = !self.compilations_expanded;
            if collapsed {
                self.selected_tree = self
                    .tree_entries()
                    .iter()
                    .position(|entry| matches!(entry, TreeEntry::Compilation))
                    .unwrap_or(self.selected_tree);
            }
            self.message = if self.compilations_expanded {
                String::from("expanded Compilations")
            } else {
                String::from("collapsed Compilations")
            };
            return;
        }
        let artist = entry.artist().to_string();
        if self.expanded_artists.remove(&artist) {
            self.selected_tree = self
                .tree_entries()
                .iter()
                .position(|entry| matches!(entry, TreeEntry::Artist { artist: entry_artist } if entry_artist == &artist))
                .unwrap_or(self.selected_tree);
            self.message = format!("collapsed {artist}");
        } else {
            self.expanded_artists.insert(artist.clone());
            self.message = format!("expanded {artist}");
        }
    }

    pub(super) fn handle_key(&mut self, conn: &Connection, key: KeyEvent) -> Result<bool> {
        self.dismiss_startup_info();

        if matches!(key.code, KeyCode::Char('c')) && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.shutdown(conn)?;
            return Ok(true);
        }

        if self.keymap_capture_action.is_some() {
            self.capture_key_binding(conn, key)?;
            return Ok(false);
        }

        if self.command_mode {
            match key.code {
                KeyCode::Esc => {
                    self.command_mode = false;
                    self.command.clear();
                    if self.clear_command_output() {
                        self.message = String::from("output cleared");
                    } else if self.filter.is_empty() {
                        self.message = String::from("command cancelled");
                    } else {
                        self.clear_filter(conn)?;
                    }
                }
                KeyCode::Enter => self.submit_command(conn),
                KeyCode::Tab => self.complete_command(conn)?,
                KeyCode::Backspace => {
                    self.command.pop();
                }
                KeyCode::Char(char) => self.command.push(char),
                _ => {}
            }
            return Ok(false);
        }

        if self.filter_mode {
            match key.code {
                KeyCode::Esc => {
                    self.clear_filter(conn)?;
                }
                KeyCode::Enter | KeyCode::Tab => self.confirm_filter(conn)?,
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.selected_tree = 0;
                    self.selected_track_row = 0;
                    self.sync_selection();
                }
                KeyCode::Char(char) => {
                    self.filter.push(char);
                    self.selected_tree = 0;
                    self.selected_track_row = 0;
                    self.sync_selection();
                }
                _ => {}
            }
            return Ok(false);
        }

        if self.command_focus {
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
                self.shutdown(conn)?;
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
                if self.playlist_panel_open || self.keymap_panel_open {
                    self.show_track_info_panel();
                } else {
                    self.toggle_info_panel();
                }
            }
            KeyAction::OpenPlaylist => self.open_playlist_panel(conn)?,
            KeyAction::ShrinkPane => self.resize_focused_pane(conn, false)?,
            KeyAction::GrowPane => self.resize_focused_pane(conn, true)?,
            KeyAction::CommandMode => {
                self.filter_mode = false;
                self.command_mode = true;
                self.command.clear();
                self.clear_command_output();
                self.message = String::from("typing command");
            }
            KeyAction::FilterMode => {
                self.filter_mode = true;
                self.message = String::from("typing filter");
            }
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
                self.shutdown(conn)?;
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
            KeyCode::Char(':') => {
                self.clear_command_output();
                self.filter_mode = false;
                self.command_mode = true;
                self.command.clear();
                self.message = String::from("typing command");
            }
            KeyCode::Char('/') => {
                self.clear_command_output();
                self.filter_mode = true;
                self.message = String::from("typing filter");
            }
            _ => {}
        }
        Ok(false)
    }

    pub(super) fn handle_mouse(
        &mut self,
        mouse: MouseEvent,
        terminal_width: u16,
        terminal_height: u16,
    ) -> bool {
        let dismissed_startup_info = self.dismiss_startup_info();
        if self.filter_mode || self.command_mode || self.command_focus {
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
            playlist_info_visible: self.playlist_panel_open,
            keymap_info_visible: self.keymap_panel_open,
            library_pane_percent_offset: self.library_pane_percent_offset,
            info_pane_height_offset: self.info_pane_height_offset,
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
        let previous = clamp_library_pane_offset(self.library_pane_percent_offset);
        let next = clamp_library_pane_offset(previous.saturating_add(delta));
        self.library_pane_percent_offset = next;
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
        let previous = clamp_info_panel_offset(self.info_pane_height_offset);
        let next = clamp_info_panel_offset(previous.saturating_add(delta));
        self.info_pane_height_offset = next;
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
                library_percent_offset: self.library_pane_percent_offset,
                info_height_offset: self.info_pane_height_offset,
            },
        )
    }

    fn dismiss_startup_info(&mut self) -> bool {
        if self.startup_info_visible {
            self.startup_info_visible = false;
            true
        } else {
            false
        }
    }
}
