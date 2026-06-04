use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use rusqlite::Connection;

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
        }
        self.sync_selection();
    }

    pub(super) fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            FocusPane::Tree => FocusPane::Tracks,
            FocusPane::Tracks if self.playlist_panel_open => FocusPane::Playlist,
            FocusPane::Tracks | FocusPane::Playlist => FocusPane::Tree,
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
        if matches!(key.code, KeyCode::Char('c')) && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.shutdown(conn)?;
            return Ok(true);
        }
        if matches!(key.code, KeyCode::Char('r') | KeyCode::Char('R'))
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            self.clear_command_output();
            self.refresh(conn)?;
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
                        self.clear_filter();
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
                    self.clear_filter();
                }
                KeyCode::Enter | KeyCode::Tab => self.confirm_filter(),
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

        if !matches!(key.code, KeyCode::Esc | KeyCode::Char(':')) {
            self.clear_command_output();
        }

        match key.code {
            KeyCode::Char('q') => {
                self.shutdown(conn)?;
                return Ok(true);
            }
            KeyCode::Esc => self.handle_escape(),
            KeyCode::Tab => self.toggle_focus(),
            KeyCode::Down | KeyCode::Char('j') => self.move_down(),
            KeyCode::Up | KeyCode::Char('k') => self.move_up(),
            KeyCode::PageDown => self.page_down(),
            KeyCode::PageUp => self.page_up(),
            KeyCode::Enter => self.activate(conn)?,
            KeyCode::Char(' ') => self.space_action(),
            KeyCode::Char('e') => {
                self.toggle_artist_expansion();
                self.sync_selection();
            }
            KeyCode::Char('c') => {
                self.toggle_pause(conn)?;
            }
            KeyCode::Char('p') => self.open_playlist_panel(conn)?,
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.add_selected_tracks_to_active_playlist(conn)?;
            }
            KeyCode::Char('-') => {
                self.remove_selected_tracks_from_active_playlist(conn)?;
            }
            KeyCode::Char('C') => self.toggle_continuous(),
            KeyCode::Char('x') => self.play_from_controls(conn)?,
            KeyCode::Char('v') => self.stop_current(conn)?,
            KeyCode::Char('b') => self.play_next(conn)?,
            KeyCode::Char('z') => self.play_previous(conn)?,
            KeyCode::Char('L') => self.toggle_play_target(),
            KeyCode::Char('R') => self.toggle_repeat(),
            KeyCode::Char('S') => self.toggle_shuffle(),
            KeyCode::Char('i') => {
                if self.playlist_panel_open {
                    self.show_track_info_panel();
                } else {
                    self.toggle_info_panel();
                }
            }
            KeyCode::Char('I') => self.select_current_track(),
            KeyCode::Char(':') => {
                self.filter_mode = false;
                self.command_mode = true;
                self.command.clear();
                self.clear_command_output();
                self.message = String::from("typing command");
            }
            KeyCode::Char('/') => {
                self.filter_mode = true;
                self.message = String::from("typing filter");
            }
            KeyCode::Left | KeyCode::Char('h') => self.seek_relative(-SCRUB_SECONDS)?,
            KeyCode::Right | KeyCode::Char('l') => self.seek_relative(SCRUB_SECONDS)?,
            KeyCode::Char(',') => self.seek_relative(-60)?,
            KeyCode::Char('.') => self.seek_relative(60)?,
            _ => {}
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
        if self.filter_mode || self.command_mode || self.command_focus {
            return false;
        }

        let direction = match mouse.kind {
            MouseEventKind::ScrollDown => 1,
            MouseEventKind::ScrollUp => -1,
            _ => return false,
        };

        let layout = MouseLayout {
            terminal_width,
            terminal_height,
            reserved_bottom_rows: self.reserved_bottom_rows(),
            info_visible: self.info_area_visible(),
            input_visible: self.input_bar_visible(),
            playlist_info_visible: self.playlist_panel_open,
        };
        let Some(pane) = mouse_pane(mouse.column, mouse.row, layout) else {
            return false;
        };
        self.clear_command_output();
        self.move_pane_selection(pane, direction, MOUSE_SCROLL_LINES);
        true
    }

    pub(super) fn handle_escape(&mut self) {
        if self.clear_command_output() {
            self.message = String::from("output cleared");
        } else {
            self.clear_filter();
        }
    }
}
